use super::{ExprFlow, Interpreter};

mod adversarial;
mod cost;
mod route_dispatch;
mod voting;
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{StreamChunk, StreamResumeContext, Value};
use crate::value_to_json;
use async_recursion::async_recursion;
use corvid_ast::Span;
use corvid_ir::IrPrompt;
use corvid_runtime::{trace_text, TraceEvent};
use corvid_types::Type;

pub(super) const DEFAULT_COMPLETION_TOKEN_ESTIMATE: u64 = 256;

struct PromptCallResult {
    value: Value,
    cost: f64,
    confidence: f64,
    tokens: u64,
    cost_charged: bool,
}

impl<'ir> Interpreter<'ir> {
    async fn finalize_prompt_result(
        &self,
        prompt: &'ir IrPrompt,
        callee_name: &str,
        arg_values: &[Value],
        result: PromptCallResult,
        span: Span,
    ) -> Result<ExprFlow, InterpError> {
        if matches!(&prompt.return_ty, Type::Stream(_)) {
            let chunk = StreamChunk::with_metrics(
                result.value,
                result.cost,
                result.confidence,
                result.tokens,
            );
            if let Some(limit) = prompt.max_tokens {
                if chunk.tokens > limit {
                    return self
                        .singleton_stream_error(
                            InterpError::new(
                                InterpErrorKind::TokenLimitExceeded {
                                    limit,
                                    used: chunk.tokens,
                                },
                                span,
                            ),
                            super::effect_compose::prompt_backpressure(prompt),
                        )
                        .await
                        .map(ExprFlow::Value);
                }
            }
            if let Some(floor) = prompt.min_confidence {
                if chunk.confidence < floor {
                    return self
                        .singleton_stream_error(
                            InterpError::new(
                                InterpErrorKind::ConfidenceFloorBreached {
                                    floor,
                                    actual: chunk.confidence,
                                },
                                span,
                            ),
                            super::effect_compose::prompt_backpressure(prompt),
                        )
                        .await
                        .map(ExprFlow::Value);
                }
            }
            let value = self
                .singleton_stream(chunk, super::effect_compose::prompt_backpressure(prompt))
                .await?;
            if let Value::Stream(stream) = &value {
                stream.set_resume_context(StreamResumeContext {
                    prompt_name: callee_name.to_string(),
                    args: arg_values.to_vec(),
                    provider_session: None,
                });
            }
            Ok(ExprFlow::Value(value))
        } else {
            // Provenance Propagation slice 7b: wrap in `Value::Grounded`
            // when the prompt's effect row carries `data: grounded`,
            // mirroring the typechecker's Design X return-type
            // promotion. Without this the IR's `UnwrapGrounded` (which
            // the typechecker inserts at every implicit
            // `Grounded<T> -> T` coercion site) finds a plain value at
            // runtime and panics. Grounding happens BEFORE confidence
            // composition so the confidence rides on the Grounded
            // wrapper, not buried inside.
            let grounded = super::grounding::maybe_ground_prompt_result(
                prompt,
                callee_name,
                result.value,
            );
            Ok(ExprFlow::Value(
                super::effect_compose::with_value_confidence(grounded, result.confidence),
            ))
        }
    }

    async fn maybe_escalate_stream_result(
        &mut self,
        prompt: &'ir IrPrompt,
        callee_name: &str,
        arg_values: &[Value],
        result: PromptCallResult,
        span: Span,
    ) -> Result<PromptCallResult, InterpError> {
        if !matches!(&prompt.return_ty, Type::Stream(_)) {
            return Ok(result);
        }
        let Some(threshold) = prompt.min_confidence else {
            return Ok(result);
        };
        if result.confidence >= threshold {
            return Ok(result);
        }
        let Some(escalate_to) = prompt.escalate_to.as_deref() else {
            return Ok(result);
        };

        let rendered = render_prompt(prompt, arg_values);
        let partial = value_to_json(&result.value);
        let continuation_rendered = format!(
            "{rendered}\n\nContinue from partial output:\n{}",
            trace_text(&partial)
        );
        let prompt_tokens = super::effect_compose::estimate_tokens(&continuation_rendered);
        let completion_tokens = prompt
            .max_tokens
            .unwrap_or(DEFAULT_COMPLETION_TOKEN_ESTIMATE);
        let selected_model = self.select_named_prompt_model(
            callee_name,
            escalate_to,
            prompt.output_format_required.as_deref(),
            prompt_tokens,
            completion_tokens,
            None,
            None,
            span,
        )?;
        self.runtime.tracer().emit(TraceEvent::StreamUpgrade {
            ts_ms: corvid_runtime::now_ms(),
            run_id: self.runtime.tracer().run_id().to_string(),
            prompt: callee_name.to_string(),
            to_model: selected_model.clone(),
            confidence_observed: result.confidence,
            threshold,
            partial: partial.clone(),
        });
        let mut upgraded = self
            .execute_prompt_call(
                prompt,
                callee_name,
                arg_values,
                &continuation_rendered,
                Some(selected_model),
                span,
            )
            .await?;
        upgraded.cost += result.cost;
        upgraded.tokens += result.tokens;
        Ok(upgraded)
    }

    #[async_recursion]
    async fn dispatch_prompt(
        &mut self,
        prompt: &'ir IrPrompt,
        callee_name: &str,
        arg_values: &[Value],
        span: Span,
    ) -> Result<PromptCallResult, InterpError> {
        let rendered = render_prompt(prompt, arg_values);
        if prompt.ensemble.is_some() {
            self.dispatch_ensemble_prompt(prompt, callee_name, arg_values, rendered.clone(), span)
                .await
        } else if prompt.adversarial.is_some() {
            self.dispatch_adversarial_prompt(
                prompt,
                callee_name,
                arg_values,
                rendered.clone(),
                span,
            )
            .await
        } else if let Some(spec) = &prompt.rollout {
            let prompt_tokens = super::effect_compose::estimate_tokens(&rendered);
            let completion_tokens = prompt
                .max_tokens
                .unwrap_or(DEFAULT_COMPLETION_TOKEN_ESTIMATE);
            let chosen_model = if self
                .runtime
                .choose_rollout_variant(spec.variant_percent)
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?
            {
                spec.variant_name.clone()
            } else {
                spec.baseline_name.clone()
            };
            self.runtime.tracer().emit(TraceEvent::AbVariantChosen {
                ts_ms: corvid_runtime::now_ms(),
                run_id: self.runtime.tracer().run_id().to_string(),
                prompt: callee_name.to_string(),
                variant: spec.variant_name.clone(),
                baseline: spec.baseline_name.clone(),
                rollout_pct: spec.variant_percent,
                chosen: chosen_model.clone(),
            });
            let selected_model = self.select_named_prompt_model(
                callee_name,
                &chosen_model,
                prompt.output_format_required.as_deref(),
                prompt_tokens,
                completion_tokens,
                None,
                None,
                span,
            )?;
            self.execute_prompt_call(
                prompt,
                callee_name,
                arg_values,
                &rendered,
                Some(selected_model),
                span,
            )
            .await
        } else if !prompt.progressive.is_empty() {
            let prompt_tokens = super::effect_compose::estimate_tokens(&rendered);
            let completion_tokens = prompt
                .max_tokens
                .unwrap_or(DEFAULT_COMPLETION_TOKEN_ESTIMATE);
            let stage_sequence: Vec<String> = prompt
                .progressive
                .iter()
                .map(|stage| stage.model_name.clone())
                .collect();
            for (stage_index, stage) in prompt.progressive.iter().enumerate() {
                let selected_model = self.select_named_prompt_model(
                    callee_name,
                    &stage.model_name,
                    prompt.output_format_required.as_deref(),
                    prompt_tokens,
                    completion_tokens,
                    None,
                    Some(stage_index),
                    span,
                )?;
                let result = self
                    .execute_prompt_call(
                        prompt,
                        callee_name,
                        arg_values,
                        &rendered,
                        Some(selected_model),
                        span,
                    )
                    .await?;
                if !matches!(&prompt.return_ty, Type::Stream(_)) {
                    self.charge_cost(result.cost, span)?;
                }
                let result = PromptCallResult {
                    cost_charged: !matches!(&prompt.return_ty, Type::Stream(_)),
                    ..result
                };
                match stage.threshold {
                    None => {
                        if stage_index > 0 {
                            self.runtime
                                .tracer()
                                .emit(TraceEvent::ProgressiveExhausted {
                                    ts_ms: corvid_runtime::now_ms(),
                                    run_id: self.runtime.tracer().run_id().to_string(),
                                    prompt: callee_name.to_string(),
                                    stages: stage_sequence.clone(),
                                });
                        }
                        return Ok(result);
                    }
                    Some(threshold) if result.confidence >= threshold => {
                        return Ok(result);
                    }
                    Some(threshold) => {
                        self.runtime
                            .tracer()
                            .emit(TraceEvent::ProgressiveEscalation {
                                ts_ms: corvid_runtime::now_ms(),
                                run_id: self.runtime.tracer().run_id().to_string(),
                                prompt: callee_name.to_string(),
                                from_stage: stage_index,
                                to_stage: stage_index + 1,
                                confidence_observed: result.confidence,
                                threshold,
                            });
                    }
                }
            }
            unreachable!("progressive prompt has at least one stage")
        } else {
            let selected_model = self
                .select_prompt_model(prompt, callee_name, &rendered, arg_values, span)
                .await?;
            self.execute_prompt_call(
                prompt,
                callee_name,
                arg_values,
                &rendered,
                selected_model,
                span,
            )
            .await
        }
    }
}

/// String extraction for AiMessage fields: strings pass through
/// verbatim; anything else renders as its JSON text.
fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => value_to_json(other).to_string(),
    }
}

fn render_prompt(prompt: &IrPrompt, args: &[Value]) -> String {
    // Conversation history (46c): with a history param, the
    // canonical rendered form is ALWAYS the role-labeled concat —
    // history in splice order between the declaration's system
    // blocks and the current turn. Best-effort here (a String
    // cannot error); strict role validation happens in
    // `build_message_segments` before dispatch.
    if let Some(idx) = prompt.history_param {
        let mut parts: Vec<String> = Vec::new();
        let turn: Vec<(String, String)> = if prompt.messages.is_empty() {
            vec![(
                "user".to_string(),
                render_template(&prompt.template, prompt, args),
            )]
        } else {
            prompt
                .messages
                .iter()
                .map(|m| (m.role.clone(), render_template(&m.template, prompt, args)))
                .collect()
        };
        for (role, content) in turn.iter().filter(|(r, _)| r == "system") {
            parts.push(format!("[{role}] {content}"));
        }
        if let Some(Value::List(list)) = args.get(idx) {
            for i in 0..list.len() {
                if let Some(Value::Struct(sv)) = list.get(i) {
                    let role = sv
                        .get_field("role")
                        .map(|v| value_text(&v))
                        .unwrap_or_default();
                    let content = sv
                        .get_field("content")
                        .map(|v| value_text(&v))
                        .unwrap_or_default();
                    parts.push(format!("[{role}] {content}"));
                }
            }
        }
        for (role, content) in turn.iter().filter(|(r, _)| r != "system") {
            parts.push(format!("[{role}] {content}"));
        }
        return parts.join("\n");
    }
    render_template(&prompt.template, prompt, args)
}

/// The three segments of a prompt request (slice 46c): declaration
/// system blocks, spliced history, and the current turn. Kept
/// separate so context-window truncation can drop history
/// oldest-first without ever touching the other two.
pub(crate) struct MessageSegments {
    pub system: Vec<corvid_runtime::llm::LlmMessage>,
    pub history: Vec<corvid_runtime::llm::LlmMessage>,
    pub turn: Vec<corvid_runtime::llm::LlmMessage>,
}

impl MessageSegments {
    pub fn is_empty(&self) -> bool {
        self.system.is_empty() && self.history.is_empty() && self.turn.is_empty()
    }

    pub fn flatten(&self) -> Vec<corvid_runtime::llm::LlmMessage> {
        let mut out =
            Vec::with_capacity(self.system.len() + self.history.len() + self.turn.len());
        out.extend(self.system.iter().cloned());
        out.extend(self.history.iter().cloned());
        out.extend(self.turn.iter().cloned());
        out
    }

    pub fn concat(&self) -> String {
        self.flatten()
            .iter()
            .map(|m| format!("[{}] {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Build the segmented message form. Errors with the offending
/// index when a history entry carries a role outside
/// system/user/assistant.
pub(crate) fn build_message_segments(
    prompt: &IrPrompt,
    args: &[Value],
    rendered: &str,
    span: Span,
) -> Result<MessageSegments, InterpError> {
    use corvid_runtime::llm::LlmMessage;
    let has_roles = !prompt.messages.is_empty();
    let has_history = prompt.history_param.is_some();
    if !has_roles && !has_history {
        return Ok(MessageSegments {
            system: Vec::new(),
            history: Vec::new(),
            turn: Vec::new(),
        });
    }

    let mut system = Vec::new();
    let mut turn = Vec::new();
    if has_roles {
        for m in &prompt.messages {
            let msg = LlmMessage {
                role: m.role.clone(),
                content: render_template(&m.template, prompt, args),
            };
            if m.role == "system" {
                system.push(msg);
            } else {
                turn.push(msg);
            }
        }
    } else {
        turn.push(LlmMessage {
            role: "user".to_string(),
            content: render_template(&prompt.template, prompt, args),
        });
    }

    let mut history = Vec::new();
    if let Some(idx) = prompt.history_param {
        if let Some(Value::List(list)) = args.get(idx) {
            for i in 0..list.len() {
                let Some(Value::Struct(sv)) = list.get(i) else {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "AiMessage".into(),
                            got: "a non-struct history entry".into(),
                        },
                        span,
                    ));
                };
                let role = sv
                    .get_field("role")
                    .map(|v| value_text(&v))
                    .unwrap_or_default();
                let content = sv
                    .get_field("content")
                    .map(|v| value_text(&v))
                    .unwrap_or_default();
                if role != "system" && role != "user" && role != "assistant" {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "history roles system/user/assistant".into(),
                            got: format!("`{role}` at history index {i}"),
                        },
                        span,
                    ));
                }
                history.push(LlmMessage { role, content });
            }
        }
    }

    // The escalation path appends continuation text to `rendered`;
    // the canonical-prefix rule turns the suffix into a final user
    // message (46b) — applies to the history form too.
    let canonical = render_prompt(prompt, args);
    if let Some(suffix) = rendered.strip_prefix(&canonical) {
        let suffix = suffix.trim();
        if !suffix.is_empty() {
            turn.push(LlmMessage {
                role: "user".to_string(),
                content: suffix.to_string(),
            });
        }
    }

    Ok(MessageSegments {
        system,
        history,
        turn,
    })
}

fn render_template(template: &str, prompt: &IrPrompt, args: &[Value]) -> String {
    let mut out = template.to_string();
    for (param, value) in prompt.params.iter().zip(args) {
        let needle = format!("{{{}}}", param.name);
        if out.contains(&needle) {
            let replacement = value_to_json(value).to_string();
            out = out.replace(&needle, &replacement);
        }
    }
    out
}

/// The 46b-compatible flat form, used by the ensemble/adversarial
/// paths (no truncation there — the single-shot path in `cost.rs`
/// owns the context-window policy).
pub(super) fn render_messages(
    prompt: &IrPrompt,
    args: &[Value],
    rendered: &str,
) -> Vec<corvid_runtime::llm::LlmMessage> {
    build_message_segments(prompt, args, rendered, Span::new(0, 0))
        .map(|segments| segments.flatten())
        .unwrap_or_default()
}
