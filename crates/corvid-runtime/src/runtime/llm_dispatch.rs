//! LLM and tool dispatch methods on `Runtime`, plus the
//! approval-gate state machine and the human-in-the-loop ask /
//! choose helpers. These are the central call paths the
//! interpreter takes for every LLM, tool, approval, and human
//! interaction — they own the trace-event bracketing and the
//! replay-source consultation that those paths require.

use sha2::{Digest, Sha256};

use crate::approvals::{ApprovalDecision, ApprovalRequest, ApprovalToken};
use crate::errors::RuntimeError;
use crate::human::{HumanChoiceRequest, HumanInputRequest};
use crate::llm::{LlmRequest, LlmRequestRef, LlmResponse};
use crate::prompt_cache::PromptCache;
use crate::tracing::now_ms;
use crate::usage::{normalized_total_tokens, LlmUsageRecord};
use corvid_trace_schema::TraceEvent;

use super::{Runtime, APPROVAL_TOKEN_SCOPE_ONE_TIME, APPROVAL_TOKEN_TTL_MS};

impl Runtime {
    // ---- dispatch helpers ----

    /// Call a tool by name. Emits trace events bracketing the call.
    ///
    /// Slice 33S1a (with 33S1-fix-naming 2026-06-08): tool names
    /// starting with `io_` are intercepted and routed to
    /// `dispatch_stdlib_io_tool` so the executing file-I/O stdlib
    /// tools (declared in `std/io.cor` as `io_read_text`,
    /// `io_write_text`, `io_list_dir`) can reach the `IoRuntime` +
    /// the `[io] root` policy that the standard `tools.call`
    /// handler-closure path can't see. The original 33S1a wired
    /// the interception against an `io.` dotted prefix, but the IR
    /// lowers `import "./std/io" use io_read_text; io_read_text(p)`
    /// to a tool call with bare `callee_name = "io_read_text"` —
    /// no module prefix. Underscore matches the bare IR name.
    /// Replay-mode reads still substitute from the recorded trace
    /// (the `replay_source` branch runs first); writes pass through
    /// the dispatch which then hits the `IoRuntime::quarantine_writes`
    /// guard if write-quarantine is on.
    pub async fn call_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                args: args.clone(),
            });
        }
        let result = if let Some(replay) = self.replay_source()? {
            replay.replay_tool_call(name, &args)?
        } else if is_stdlib_io_tool(name) {
            self.dispatch_stdlib_io_tool(name, args.clone()).await?
        } else if is_stdlib_http_tool(name) {
            self.dispatch_stdlib_http_tool(name, args.clone()).await?
        } else {
            self.tools.call(name, args.clone()).await?
        };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: name.to_string(),
                result: result.clone(),
            });
        }
        Ok(result)
    }

    /// Slice 33S1a: dispatch handler for the three executing
    /// stdlib file-I/O tools (`io_read_text` / `io_write_text` /
    /// `io_list_dir`). Receives the full tool name; the caller
    /// (`call_tool`) gated entry via `is_stdlib_io_tool` so an
    /// unknown name here is a programmer error rather than a
    /// fall-through. Each branch:
    ///
    ///   1. Extracts + validates args as JSON values.
    ///   2. Resolves the caller's path through `self.io_policy`
    ///      (fails closed when `[io] root` is unconfigured;
    ///      rejects traversal + absolute escapes).
    ///   3. Calls the existing `IoRuntime` method.
    ///   4. Marshals the typed result back to a JSON value
    ///      matching the envelope schema declared in `std/io.cor`
    ///      (FileReadEnvelope / FileWriteEnvelope /
    ///      [DirectoryEntryEnvelope]).
    async fn dispatch_stdlib_io_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        match name {
            "io_read_text" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_read_text".to_string(),
                        message: "expected one String argument (path)".to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let read = self.io.read_text(&resolved).await?;
                Ok(serde_json::json!({
                    "path_value": read.path.display().to_string(),
                    "contents": read.contents,
                    "bytes": read.bytes as i64,
                    "effect_meta": stdlib_io_effect_envelope(&read.effect),
                }))
            }
            "io_write_text" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_write_text".to_string(),
                        message: "expected (path: String, content: String) — path missing"
                            .to_string(),
                    })?;
                let content_arg = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_write_text".to_string(),
                        message: "expected (path: String, content: String) — content missing"
                            .to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let write = self.io.write_text(&resolved, content_arg).await?;
                Ok(serde_json::json!({
                    "path_value": write.path.display().to_string(),
                    "bytes": write.bytes as i64,
                    "effect_meta": stdlib_io_effect_envelope(&write.effect),
                }))
            }
            "io_list_dir" => {
                let path_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "io_list_dir".to_string(),
                        message: "expected one String argument (path)".to_string(),
                    })?;
                let resolved = self.io_policy.resolve(path_arg)?;
                let entries = self.io.list_dir(&resolved).await?;
                let json_entries: Vec<serde_json::Value> = entries
                    .into_iter()
                    .map(|entry| {
                        serde_json::json!({
                            "path_value": entry.path.display().to_string(),
                            "name": entry.name,
                            "is_dir": entry.is_dir,
                            "effect_meta": stdlib_io_effect_envelope(&entry.effect),
                        })
                    })
                    .collect();
                Ok(serde_json::Value::Array(json_entries))
            }
            other => Err(RuntimeError::UnknownTool(other.to_string())),
        }
    }

    /// Slice 33S2a: dispatch handler for the two executing
    /// stdlib HTTP tools (`http_get` / `http_post_json`).
    /// Receives the full tool name; entry is gated by
    /// `is_stdlib_http_tool`. Each branch:
    ///
    ///   1. Extracts + validates args as JSON values.
    ///   2. Checks the URL through `self.http_policy` (always-on
    ///      SSRF block + required `[http] allow` allowlist; fails
    ///      closed when allowlist is unconfigured).
    ///   3. Calls `HttpClient::send` with the constructed
    ///      `HttpRequest`.
    ///   4. Marshals the typed `HttpResponse` back to a JSON
    ///      value matching the `HttpResponseEnvelope` schema
    ///      declared in `std/http.cor`.
    async fn dispatch_stdlib_http_tool(
        &self,
        name: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        use crate::http::HttpRequest;
        match name {
            "http_get" => {
                let url_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_get".to_string(),
                        message: "expected one String argument (url)".to_string(),
                    })?;
                self.http_policy.check(url_arg)?;
                let request = HttpRequest::get(url_arg.to_string())
                    .effect_tag("std.http.request");
                let response = self.http.send(&request).await?;
                Ok(serde_json::json!({
                    "status": response.status as i64,
                    "body": response.body,
                    "attempts": 1_i64,
                    "elapsed_ms": 0_i64,
                    "effect_meta": stdlib_http_effect_envelope(),
                }))
            }
            "http_post_json" => {
                let url_arg = args
                    .first()
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_post_json".to_string(),
                        message: "expected (url: String, body: String) — url missing"
                            .to_string(),
                    })?;
                let body_arg = args
                    .get(1)
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| RuntimeError::ToolFailed {
                        tool: "http_post_json".to_string(),
                        message: "expected (url: String, body: String) — body missing"
                            .to_string(),
                    })?;
                self.http_policy.check(url_arg)?;
                let request = HttpRequest::post_json(url_arg.to_string(), body_arg.to_string())
                    .effect_tag("std.http.request");
                let response = self.http.send(&request).await?;
                Ok(serde_json::json!({
                    "status": response.status as i64,
                    "body": response.body,
                    "attempts": 1_i64,
                    "elapsed_ms": 0_i64,
                    "effect_meta": stdlib_http_effect_envelope(),
                }))
            }
            other => Err(RuntimeError::UnknownTool(other.to_string())),
        }
    }

    /// Call an LLM. Falls back to `default_model` if `req.model` is empty.
    pub async fn call_llm(&self, mut req: LlmRequest) -> Result<LlmResponse, RuntimeError> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        self.call_llm_ref(req.as_ref()).await
    }

    /// Call an LLM through the prompt-response cache when the source
    /// prompt declared `cacheable: true`. Replay mode bypasses the live
    /// cache and consumes the recorded `LlmCall` / `LlmResult` pair instead.
    pub async fn call_llm_cacheable(
        &self,
        mut req: LlmRequest,
        cacheable: bool,
    ) -> Result<LlmResponse, RuntimeError> {
        if req.model.is_empty() {
            req.model = self.default_model.clone();
        }
        self.call_llm_ref_impl(req.as_ref(), None, cacheable).await
    }

    pub async fn call_llm_ref_with_trace_rendered(
        &self,
        req: LlmRequestRef<'_>,
        trace_rendered: Option<&str>,
    ) -> Result<LlmResponse, RuntimeError> {
        self.call_llm_ref_impl(req, trace_rendered, false).await
    }

    /// Borrowed LLM-call path for native bridges that already hold prompt and
    /// rendered text as borrowed strings and only need owned clones when
    /// tracing or provider JSON construction requires them.
    pub async fn call_llm_ref(&self, req: LlmRequestRef<'_>) -> Result<LlmResponse, RuntimeError> {
        self.call_llm_ref_impl(req, None, false).await
    }

    async fn call_llm_ref_impl(
        &self,
        req: LlmRequestRef<'_>,
        trace_rendered_override: Option<&str>,
        cacheable: bool,
    ) -> Result<LlmResponse, RuntimeError> {
        let req = if req.model.is_empty() {
            req.with_model(&self.default_model)
        } else {
            req
        };
        let trace_rendered = trace_rendered_override.unwrap_or(req.rendered);
        let replay = self.replay_source()?;
        let live_model_override = replay
            .and_then(|source| source.live_model_override())
            .map(str::to_owned);
        let trace_model = live_model_override.as_deref().unwrap_or(req.model);
        let recorded_model_version = self.model_version(req.model);
        let trace_model_version = self.model_version(trace_model);
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::LlmCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.to_string(),
                model: if trace_model.is_empty() {
                    None
                } else {
                    Some(trace_model.to_string())
                },
                model_version: trace_model_version.clone(),
                rendered: Some(trace_rendered.to_string()),
                args: req.args.to_vec(),
            });
        }
        let cache_fingerprint = if cacheable && replay.is_none() {
            Some(PromptCache::fingerprint(req))
        } else {
            None
        };
        if let Some(fingerprint) = cache_fingerprint.as_deref() {
            if let Some(cached) = self.prompt_cache.get(fingerprint) {
                if self.tracer.is_enabled() {
                    self.tracer.emit(TraceEvent::PromptCache {
                        ts_ms: now_ms(),
                        run_id: self.tracer.run_id().to_string(),
                        prompt: req.prompt.to_string(),
                        model: if trace_model.is_empty() {
                            None
                        } else {
                            Some(trace_model.to_string())
                        },
                        model_version: trace_model_version.clone(),
                        fingerprint: fingerprint.to_string(),
                        hit: true,
                    });
                    self.tracer.emit(TraceEvent::LlmResult {
                        ts_ms: now_ms(),
                        run_id: self.tracer.run_id().to_string(),
                        prompt: req.prompt.to_string(),
                        model: if trace_model.is_empty() {
                            None
                        } else {
                            Some(trace_model.to_string())
                        },
                        model_version: trace_model_version.clone(),
                        result: cached.value.clone(),
                    });
                }
                return Ok(PromptCache::cached_response(cached));
            }
        }
        let mut actual_model = live_model_override
            .as_deref()
            .unwrap_or(req.model)
            .to_string();
        let mut actual_adapter = if replay.is_some() {
            self.llms.adapter_name_for_model(&actual_model)
        } else {
            None
        };
        let mut result_trace_model = trace_model.to_string();
        let mut result_trace_model_version = trace_model_version.clone();
        let resp = if let Some(replay) = replay {
            let live_req = if let Some(model) = live_model_override.as_deref() {
                req.with_model(model)
            } else {
                req
            };
            replay
                .replay_llm_call(
                    req.prompt,
                    if req.model.is_empty() {
                        None
                    } else {
                        Some(req.model)
                    },
                    recorded_model_version.as_deref(),
                    trace_rendered,
                    req.args,
                    live_req,
                    &self.llms,
                )
                .await?
        } else {
            match self.llms.call_with_adapter_name(&req).await {
                Ok(outcome) => {
                    actual_adapter = Some(outcome.adapter);
                    outcome.response
                }
                Err(primary_err) => {
                    let primary_error = primary_err.to_string();
                    self.emit_host_event(
                        "llm.provider_degraded",
                        serde_json::json!({
                            "prompt": req.prompt,
                            "model": req.model,
                            "provider": self.model_catalog.get(req.model).and_then(|model| model.provider.clone()),
                            "error": primary_error,
                        }),
                    );
                    let mut last_err = primary_err;
                    let fallbacks = self.model_catalog.compatible_fallbacks_for(
                        req.model,
                        estimate_tokens(trace_rendered),
                        0,
                    );
                    let mut fallback_response = None;
                    for fallback in fallbacks {
                        let fallback_req = req.with_model(&fallback.model);
                        match self.llms.call_with_adapter_name(&fallback_req).await {
                            Ok(outcome) => {
                                self.emit_host_event(
                                    "llm.provider_failover",
                                    serde_json::json!({
                                        "prompt": req.prompt,
                                        "from_model": req.model,
                                        "from_provider": self.model_catalog.get(req.model).and_then(|model| model.provider.clone()),
                                        "to_model": fallback.model.clone(),
                                        "to_provider": fallback.provider.clone(),
                                        "adapter": outcome.adapter,
                                    }),
                                );
                                actual_model = fallback.model;
                                actual_adapter = Some(outcome.adapter);
                                result_trace_model = actual_model.clone();
                                result_trace_model_version = self.model_version(&actual_model);
                                fallback_response = Some(outcome.response);
                                break;
                            }
                            Err(err) => {
                                self.emit_host_event(
                                    "llm.provider_degraded",
                                    serde_json::json!({
                                        "prompt": req.prompt,
                                        "model": fallback.model.clone(),
                                        "provider": fallback.provider.clone(),
                                        "error": err.to_string(),
                                    }),
                                );
                                last_err = err;
                            }
                        }
                    }
                    fallback_response.ok_or(last_err)?
                }
            }
        };
        if let Some(fingerprint) = cache_fingerprint.as_deref() {
            self.prompt_cache
                .insert(fingerprint.to_string(), resp.clone());
            if self.tracer.is_enabled() {
                self.tracer.emit(TraceEvent::PromptCache {
                    ts_ms: now_ms(),
                    run_id: self.tracer.run_id().to_string(),
                    prompt: req.prompt.to_string(),
                    model: if trace_model.is_empty() {
                        None
                    } else {
                        Some(trace_model.to_string())
                    },
                    model_version: trace_model_version.clone(),
                    fingerprint: fingerprint.to_string(),
                    hit: false,
                });
            }
        }
        let cost_usd = if actual_model.is_empty() {
            0.0
        } else {
            self.model_catalog
                .describe_named_model(
                    &actual_model,
                    resp.usage.prompt_tokens as u64,
                    resp.usage.completion_tokens as u64,
                )
                .cost_estimate
        };
        let model_metadata = self.model_catalog.get(&actual_model);
        let provider = model_metadata.and_then(|model| model.provider.clone());
        let privacy_tier = model_metadata.and_then(|model| model.privacy_tier.clone());
        let total_tokens = normalized_total_tokens(resp.usage);
        let usage_record = LlmUsageRecord {
            ts_ms: now_ms(),
            prompt: req.prompt.to_string(),
            model: actual_model.clone(),
            provider: provider.clone(),
            adapter: actual_adapter.clone(),
            privacy_tier: privacy_tier.clone(),
            prompt_tokens: resp.usage.prompt_tokens as u64,
            completion_tokens: resp.usage.completion_tokens as u64,
            total_tokens,
            cost_usd,
            local: provider.as_deref() == Some("ollama") || privacy_tier.as_deref() == Some("local"),
        };
        self.usage_ledger.record(usage_record.clone());
        self.emit_host_event(
            "llm.usage",
            serde_json::json!({
                "prompt": usage_record.prompt,
                "model": usage_record.model,
                "provider": usage_record.provider,
                "adapter": usage_record.adapter,
                "privacy_tier": usage_record.privacy_tier,
                "prompt_tokens": usage_record.prompt_tokens,
                "completion_tokens": usage_record.completion_tokens,
                "total_tokens": usage_record.total_tokens,
                "cost_usd": usage_record.cost_usd,
                "currency": "USD",
                "unit": "token",
                "local": usage_record.local,
            }),
        );
        crate::observation_handles::record_llm_usage(resp.usage, cost_usd);
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::LlmResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.to_string(),
                model: if result_trace_model.is_empty() {
                    None
                } else {
                    Some(result_trace_model)
                },
                model_version: result_trace_model_version,
                result: resp.value.clone(),
            });
        }
        Ok(resp)
    }

    /// Ask the approver about an action. Returns `ApprovalDenied` if
    /// denied; the interpreter surfaces this as `InterpError::Runtime`.
    pub async fn approval_gate(
        &self,
        label: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        let trace_enabled = self.tracer.is_enabled();
        let label_owned = label.to_string();
        if trace_enabled {
            self.tracer.emit(TraceEvent::ApprovalRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                label: label_owned.clone(),
                args: args.clone(),
            });
        }
        let req = ApprovalRequest {
            label: label_owned.clone(),
            args,
        };
        let (approved, detail) = if let Some(replay) = self.replay_source()? {
            let outcome = replay.replay_approval(&label_owned, &req.args)?;
            let detail =
                outcome
                    .decision
                    .map(|decision| crate::approver_bridge::ApprovalDecisionInfo {
                        accepted: decision.accepted,
                        decider: decision.decider,
                        rationale: decision.rationale,
                    });
            (outcome.approved, detail)
        } else {
            let approved = self.approver.approve(&req).await? == ApprovalDecision::Approve;
            let detail = Some(crate::catalog_c_api::take_last_approval_detail().unwrap_or(
                crate::approver_bridge::ApprovalDecisionInfo {
                    accepted: approved,
                    decider: "runtime-approver".to_string(),
                    rationale: None,
                },
            ));
            (approved, detail)
        };
        if trace_enabled {
            if let Some(detail) = detail {
                self.tracer.emit(TraceEvent::ApprovalDecision {
                    ts_ms: now_ms(),
                    run_id: self.tracer.run_id().to_string(),
                    site: label_owned.clone(),
                    args: req.args.clone(),
                    accepted: detail.accepted,
                    decider: detail.decider,
                    rationale: detail.rationale,
                });
            }
        }
        if trace_enabled {
            self.tracer.emit(TraceEvent::ApprovalResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                label: label_owned.clone(),
                approved,
            });
        }
        if approved {
            if trace_enabled {
                let issued_at_ms = now_ms();
                let expires_at_ms = issued_at_ms.saturating_add(APPROVAL_TOKEN_TTL_MS);
                let run_id = self.tracer.run_id().to_string();
                self.tracer.emit(TraceEvent::ApprovalTokenIssued {
                    ts_ms: issued_at_ms,
                    run_id: run_id.clone(),
                    token_id: approval_token_id(
                        &run_id,
                        &label_owned,
                        &req.args,
                        APPROVAL_TOKEN_SCOPE_ONE_TIME,
                        issued_at_ms,
                        expires_at_ms,
                    ),
                    label: label_owned.clone(),
                    args: req.args.clone(),
                    scope: APPROVAL_TOKEN_SCOPE_ONE_TIME.to_string(),
                    issued_at_ms,
                    expires_at_ms,
                });
            }
            Ok(())
        } else {
            Err(RuntimeError::ApprovalDenied {
                action: label_owned,
            })
        }
    }

    pub fn validate_approval_token_scope(
        &self,
        token: &mut ApprovalToken,
        label: &str,
        args: &[serde_json::Value],
        session_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let now = now_ms();
        match token.validate(label, args, now, session_id) {
            Ok(()) => Ok(()),
            Err(reason) => {
                if self.tracer.is_enabled() {
                    self.tracer.emit(TraceEvent::ApprovalScopeViolation {
                        ts_ms: now,
                        run_id: self.tracer.run_id().to_string(),
                        token_id: token.token_id.clone(),
                        label: label.to_string(),
                        reason: reason.clone(),
                    });
                }
                Err(RuntimeError::ApprovalFailed(format!(
                    "approval token scope violation: {reason}"
                )))
            }
        }
    }

    pub async fn ask_human(
        &self,
        prompt: &str,
        expected_type: impl Into<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let req = HumanInputRequest {
            prompt: prompt.to_string(),
            expected_type: expected_type.into(),
        };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanInputRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt.clone(),
                expected_type: req.expected_type.clone(),
            });
        }
        let value = self.human.ask(&req).await?;
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanInputResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                prompt: req.prompt,
                value: value.clone(),
            });
        }
        Ok(value)
    }

    pub async fn choose_human(
        &self,
        options: Vec<serde_json::Value>,
    ) -> Result<usize, RuntimeError> {
        let req = HumanChoiceRequest { options };
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanChoiceRequest {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                options: req.options.clone(),
            });
        }
        let selected_index = self.human.choose(&req).await?;
        let selected_value = req.options.get(selected_index).cloned().ok_or_else(|| {
            RuntimeError::Other(format!("human choice index {selected_index} out of range"))
        })?;
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::HumanChoiceResponse {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                selected_index,
                selected_value,
            });
        }
        Ok(selected_index)
    }
}

fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4).max(1)
}

fn approval_token_id(
    run_id: &str,
    label: &str,
    args: &[serde_json::Value],
    scope: &str,
    issued_at_ms: u64,
    expires_at_ms: u64,
) -> String {
    let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(label.as_bytes());
    hasher.update(b"\0");
    hasher.update(args_json.as_bytes());
    hasher.update(b"\0");
    hasher.update(scope.as_bytes());
    hasher.update(b"\0");
    hasher.update(issued_at_ms.to_le_bytes());
    hasher.update(expires_at_ms.to_le_bytes());
    format!("apr_{}", hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Slice 33S1a: marshal `IoRuntime::FileSystemEffect` into a JSON
/// object matching the `EffectEnvelope` type declared in
/// `std/effects.cor`. Field order matches the type's declaration
/// order so the IR-side struct construction picks up each field
/// positionally even though we emit them as a named JSON object.
///
/// EffectEnvelope fields: effect_name, provenance_key,
/// approval_label, cache_key, replay_key.
fn stdlib_io_effect_envelope(
    effect: &crate::io::FileSystemEffect,
) -> serde_json::Value {
    serde_json::json!({
        "effect_name": effect.effect_tag,
        "provenance_key": "",
        "approval_label": effect.approval_label,
        "cache_key": "",
        "replay_key": effect.replay_key,
    })
}

/// Slice 33S1a (refactored 33S2a): exact-match gate for the
/// stdlib file-I/O tools. Returns true only for the three
/// declared tool names; a user-defined tool that happens to
/// start with `io_` (e.g. `io_foobar`) reaches the normal
/// `tools.call` path. Prevents the dispatch interception from
/// stealing user tool names.
fn is_stdlib_io_tool(name: &str) -> bool {
    matches!(name, "io_read_text" | "io_write_text" | "io_list_dir")
}

/// Slice 33S2a: exact-match gate for the stdlib HTTP tools.
/// Same rationale as `is_stdlib_io_tool` — exact names only,
/// no prefix sweep.
fn is_stdlib_http_tool(name: &str) -> bool {
    matches!(name, "http_get" | "http_post_json")
}

/// Slice 33S2a: marshal an `EffectEnvelope` for the executing
/// HTTP tools. The runtime side doesn't carry a per-call
/// effect-tag structure for HTTP (the request/response cycle
/// is the unit), so we emit a fixed `std.http.request` tag
/// matching what `std/http.cor` programs already expect.
fn stdlib_http_effect_envelope() -> serde_json::Value {
    serde_json::json!({
        "effect_name": "std.http.request",
        "provenance_key": "",
        "approval_label": "",
        "cache_key": "",
        "replay_key": "std.http.request",
    })
}
