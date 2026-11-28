use crate::errors::{InterpError, InterpErrorKind};
use crate::interp::{ExprFlow, Interpreter};
use crate::value::{ResumeTokenValue, Value};
use corvid_ast::Span;
use corvid_ir::{IrPrompt, IrRoutePattern};
use corvid_resolve::DefId;
use corvid_runtime::trace_text;
use corvid_types::Type;


impl<'ir> Interpreter<'ir> {
    pub(super) async fn select_prompt_route_model(
        &mut self,
        prompt: &'ir IrPrompt,
        callee_name: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
        span: Span,
    ) -> Result<Option<String>, InterpError> {
        for (arm_index, arm) in prompt.route.iter().enumerate() {
            let matched = match &arm.pattern {
                IrRoutePattern::Wildcard => true,
                IrRoutePattern::Guard(expr) => {
                    let guard_value = match self.eval_expr(expr).await?.into_value() {
                        Ok(v) | Err(v) => v,
                    };
                    super::super::expr::require_bool(&guard_value, expr.span, "route guard")?
                }
            };
            if !matched {
                continue;
            }
            return self
                .select_named_prompt_model(
                    callee_name,
                    &arm.model_name,
                    prompt.output_format_required.as_deref(),
                    prompt_tokens,
                    completion_tokens,
                    Some(arm_index),
                    None,
                    span,
                )
                .map(Some);
        }

        Err(InterpError::new(
            InterpErrorKind::Runtime(corvid_runtime::RuntimeError::NoMatchingRoute {
                prompt: callee_name.to_string(),
            }),
            span,
        ))
    }

    /// A prompt streams live (46d) when it returns Stream<String>
    /// and uses none of the multi-model dispatch clauses (those
    /// need complete responses to compare/vote/escalate).
    fn prompt_streams_live(prompt: &IrPrompt) -> bool {
        matches!(&prompt.return_ty, Type::Stream(inner) if matches!(inner.as_ref(), Type::String))
            && prompt.route.is_empty()
            && prompt.progressive.is_empty()
            && prompt.rollout.is_none()
            && prompt.ensemble.is_none()
            && prompt.adversarial.is_none()
            && prompt.escalate_to.is_none()
    }

    /// Live streaming dispatch (slice 46d): sets up the provider
    /// stream, then feeds deltas into a Corvid stream as they
    /// arrive. Token limits apply CUMULATIVELY mid-stream; cost is
    /// charged on the final chunk from accumulated estimates. On
    /// stream-setup failure, falls back to the whole-call path.
    async fn dispatch_streaming_prompt(
        &mut self,
        prompt: &'ir IrPrompt,
        callee_name: &str,
        arg_values: &[Value],
        span: Span,
    ) -> Result<ExprFlow, InterpError> {
        
        let rendered = self.render_prompt_text(prompt, arg_values, span)?;
        let json_args = self.model_args(prompt, arg_values, span)?;
        let model = self.select_prompt_model(prompt, callee_name, &rendered, arg_values, span)
            .await?.unwrap_or_default();
        let sampling = self.resolve_sampling(prompt, &model);
        let segments = super::build_message_segments(prompt, arg_values, &rendered, &self.types_by_id, span)?;
        let req = corvid_runtime::llm::LlmRequest {
            prompt: callee_name.to_string(),
            model,
            rendered: if segments.is_empty() {
                rendered.clone()
            } else {
                segments.concat()
            },
            args: json_args,
            output_schema: None,
            sampling,
            messages: segments.flatten(),
        };
        let chunk_stream = self.runtime.call_llm_stream(req).await
            .map_err(|err| InterpError::new(InterpErrorKind::Runtime(err), span))?;

        let backpressure = super::super::effect_compose::prompt_backpressure(prompt);
        let (sender, stream) = crate::value::StreamValue::channel(backpressure);
        let token_limit = prompt.max_tokens;
        let confidence_floor = prompt.min_confidence;
        let effect_confidence = prompt.effect_confidence;
        let per_call_cost = prompt.effect_cost;
        let feed_span = span;
        stream.spawn_producer(async move {
            let mut chunks = chunk_stream;
            let mut emitted_tokens = 0u64;
            let mut characters = 0u64;
            while let Some(item) = futures::StreamExt::next(&mut chunks).await {
                match item {
                    Ok(chunk) => {
                        let done = chunk.done;
                        // Provider-reported usage on the final chunk
                        // is authoritative (the mock and whole-call
                        // fallback report it); mid-stream deltas use
                        // the text estimate.
                        characters = characters.saturating_add(chunk.delta.chars().count() as u64);
                        let total_tokens = match &chunk.usage {
                            Some(usage) => u64::from(usage.completion_tokens),
                            None if characters > 0 => (characters / 4).max(1),
                            None => 0,
                        };
                        let tokens = total_tokens.saturating_sub(emitted_tokens);
                        emitted_tokens = total_tokens;
                        if let Some(limit) = token_limit {
                            if total_tokens > limit {
                                let _ = sender
                                    .send_chunk(Err(InterpError::new(
                                        InterpErrorKind::TokenLimitExceeded {
                                            limit,
                                            used: total_tokens,
                                        },
                                        feed_span,
                                    )))
                                    .await;
                                return;
                            }
                        }
                        let chunk_confidence =
                            chunk.confidence.unwrap_or(effect_confidence);
                        if let Some(floor) = confidence_floor {
                            if chunk_confidence < floor {
                                let _ = sender
                                    .send_chunk(Err(InterpError::new(
                                        InterpErrorKind::ConfidenceFloorBreached {
                                            floor,
                                            actual: chunk_confidence,
                                        },
                                        feed_span,
                                    )))
                                    .await;
                                return;
                            }
                        }
                        if !chunk.delta.is_empty() || done {
                            // Cost rides the final chunk; deltas are
                            // free so mid-stream budget checks see
                            // the whole cost exactly once.
                            let cost = if done { per_call_cost } else { 0.0 };
                            let sc = crate::value::StreamChunk::with_metrics(
                                Value::String(chunk.delta.into()),
                                cost,
                                chunk_confidence,
                                tokens,
                            );
                            if !sender.send_chunk(Ok(sc)).await {
                                return;
                            }
                        }
                        if done {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = sender
                            .send_chunk(Err(InterpError::new(
                                InterpErrorKind::Runtime(e),
                                feed_span,
                            )))
                            .await;
                        return;
                    }
                }
            }
        });

        stream.set_resume_context(crate::value::StreamResumeContext {
            prompt_name: callee_name.to_string(),
            args: arg_values.to_vec(),
            provider_session: None,
        });
        Ok(ExprFlow::Value(Value::Stream(stream)))
    }

    pub(super) fn prompt_by_id(
        &self,
        def_id: DefId,
        prompt_name: &str,
        span: Span,
    ) -> Result<&'ir IrPrompt, InterpError> {
        self.prompts_by_id.get(&def_id).copied().ok_or_else(|| {
            InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "prompt `{prompt_name}` is missing from the IR"
                )),
                span,
            )
        })
    }

    // Box prompt dispatch and stream resumption so their provider state does
    // not inflate the evaluator frame for unrelated nested expressions.
    #[async_recursion::async_recursion]
    pub(in crate::interp) async fn dispatch_prompt_expr(
        &mut self,
        def_id: DefId,
        callee_name: &str,
        arg_values: &[Value],
        span: Span,
    ) -> Result<ExprFlow, InterpError> {
        let prompt = self.prompt_by_id(def_id, callee_name, span)?;
        Self::check_refined_arguments(&prompt.name, &prompt.params, arg_values, span)?;
        if let Some(mock) = self.mock_tools.get(&def_id).copied() {
            let value = self.eval_mock_body(mock, callee_name, arg_values.to_vec(), span).await?;
            let value = super::super::grounding::maybe_ground_prompt_result(prompt, callee_name, value);
            return Ok(ExprFlow::Value(value));
        }
        // Real provider streaming (slice 46d): plain-path
        // Stream<String> prompts stream incrementally. Everything
        // else (dispatch clauses, mocks, structured outputs) keeps
        // the whole-call-then-singleton behavior.
        if Self::prompt_streams_live(prompt) {
            return self
                .dispatch_streaming_prompt(prompt, callee_name, arg_values, span)
                .await;
        }
        let result = self
            .dispatch_prompt(prompt, callee_name, arg_values, span)
            .await?;
        let result = self
            .maybe_escalate_stream_result(prompt, callee_name, arg_values, result, span)
            .await?;
        if !result.cost_charged && !matches!(&prompt.return_ty, Type::Stream(_)) {
            self.charge_cost(result.cost, span)?;
        }
        self.finalize_prompt_result(prompt, callee_name, arg_values, result, span)
            .await
    }

    #[async_recursion::async_recursion]
    pub(in crate::interp) async fn resume_prompt_stream(
        &mut self,
        prompt_def_id: DefId,
        prompt_name: &str,
        token: ResumeTokenValue,
        span: Span,
    ) -> Result<ExprFlow, InterpError> {
        let prompt = self.prompt_by_id(prompt_def_id, prompt_name, span)?;
        if token.prompt_name != prompt_name {
            return Err(InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "resume token is for prompt `{}`, not `{prompt_name}`",
                    token.prompt_name
                )),
                span,
            ));
        }

        let base_rendered = self.render_prompt_text(prompt, &token.args, span)?;
        let delivered = token
            .delivered
            .iter()
            .map(|chunk| self.model_json(&chunk.value, &prompt.return_ty, span).map(|json| trace_text(&json)))
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        let continuation_rendered = if delivered.is_empty() {
            format!("{base_rendered}\n\nResume from interruption with no delivered elements.")
        } else {
            format!("{base_rendered}\n\nResume after delivered elements:\n{delivered}")
        };
        let selected_model = self
            .select_prompt_model(
                prompt,
                prompt_name,
                &continuation_rendered,
                &token.args,
                span,
            )
            .await?;
        let result = self
            .execute_prompt_call(
                prompt,
                prompt_name,
                &token.args,
                &continuation_rendered,
                selected_model,
                span,
            )
            .await?;
        self.finalize_prompt_result(prompt, prompt_name, &token.args, result, span)
            .await
    }
}
