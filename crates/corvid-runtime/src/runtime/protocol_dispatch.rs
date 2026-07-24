//! Durable verified-provider-protocol execution (slice 52h-2).
//!
//! Drives the pure transition engine in [`crate::protocol`] against a
//! real provider, with the durability that makes a long-running external
//! job safe:
//!
//! 1. **Intent before submit.** The intent is written to the durable job
//!    as a checkpoint BEFORE the submit request leaves the process, and
//!    the submit itself carries an idempotency key derived from the
//!    logical call. A crash between "intent recorded" and "submit
//!    acknowledged" therefore cannot lose the work, and a resumed run
//!    cannot create a second provider job.
//! 2. **Bind only on a typed response.** The provider job id is taken
//!    from the DECODED submit response and only then written into the
//!    intent — never guessed, never assumed from the request.
//! 3. **Every transition is a checkpoint.** Each observed status is
//!    applied through the declared transition table and the new state is
//!    persisted, so a restart resumes at the last observation instead of
//!    re-submitting or re-walking the graph.
//! 4. **Never treat submit as completion.** The call returns only when a
//!    declared terminal state is reached, or when the declared deadline
//!    forces the declared deadline target.
//!
//! Mock, replay, and real all share this engine: the submit and poll
//! requests bottom out at the same connector dispatch, so the mode is
//! decided there, not here.

use super::Runtime;
use crate::connectors::ConnectorHttpSpec;
use crate::errors::RuntimeError;
use crate::protocol::{
    bind_poll_path, bind_protocol_path, cancellation_disposition, intent_idempotency_key,
    is_terminal, observe, poll_delay_ms, CancellationDisposition, ProtocolIntentState,
};
use crate::queue::JobCheckpointKind;
use corvid_ast::ProviderProtocolDecl;

impl Runtime {
    /// Execute a protocol-bearing connector operation by NAME, looking
    /// up its dispatch spec. The VM's entry point for a verified
    /// provider protocol.
    pub async fn call_protocol_operation(
        &self,
        operation: &str,
        protocol: &ProviderProtocolDecl,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let spec = self
            .connector_calls
            .get(operation)
            .cloned()
            .ok_or_else(|| RuntimeError::ToolFailed {
                tool: operation.to_string(),
                message: format!(
                    "connector operation `{operation}` has no dispatch spec — startup validation \
                     should have refused this"
                ),
            })?;
        self.dispatch_protocol_operation(&spec, protocol, &args)
            .await
    }

    /// Execute one verified provider protocol to a terminal state,
    /// durably. Returns the decoded terminal poll response.
    pub async fn dispatch_protocol_operation(
        &self,
        spec: &ConnectorHttpSpec,
        protocol: &ProviderProtocolDecl,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        // A protocol's intent must outlive the process, so it is only
        // executable inside a durable job. Refusing here (rather than
        // silently degrading to a non-durable poll loop) is what makes
        // the durability guarantee real.
        let job = self.durable_job().ok_or_else(|| RuntimeError::ToolFailed {
            tool: spec.operation.clone(),
            message: format!(
                "operation `{}` declares a verified provider protocol, so it must run inside a \
                 durable job (its intent has to survive a restart). Invoke it from a \
                 `@replayable` agent executed by the durable job queue.",
                spec.operation
            ),
        })?;

        let key = intent_idempotency_key(&spec.connector, &spec.operation, args);
        let mut intent = self.load_protocol_intent(job, &key, protocol)?;

        // (1) Record the intent BEFORE any submit, so a crash here is
        // recoverable and a resume re-finds it instead of re-submitting.
        if !intent.submitted {
            self.checkpoint_protocol_intent(job, &key, &intent)?;

            // (2) Submit, then bind the provider job id from the DECODED
            // response — never before it is typed.
            let response = self.dispatch_connector_http(spec, args).await?;
            intent.bind_submit_response(&unwrap_ok_envelope(&response));
            self.checkpoint_protocol_intent(job, &key, &intent)?;
        }

        // (3) Observe until a declared terminal state, or the deadline
        // forces the declared deadline target.
        let started_ms = crate::tracing::now_ms();
        let deadline_ms = protocol.deadline_secs.saturating_mul(1000);
        // The provider's most recent `Retry-After`, if it asked us to
        // back off (slice 52h-3).
        let mut retry_after_hint: Option<u64> = None;
        // Consecutive failed observations, for circuit-breaker admission
        // (slice 52h-3). Reset by any successful poll.
        let mut consecutive_poll_failures: u64 = 0;
        loop {
            if is_terminal(protocol, &intent.state) {
                break;
            }
            // Semantic cancellation (slice 52h-3). The durable job is the
            // cancellation channel: if it was cancelled while we were
            // observing, act on it at the next boundary rather than
            // finishing work nobody wants. What "act on it" MEANS depends
            // on the declaration and on whether we already submitted —
            // see `cancellation_disposition`.
            if let Ok(Some(current)) = job.queue.get(&job.job_id) {
                if matches!(current.status, crate::queue::QueueJobStatus::Canceled) {
                    return self
                        .apply_protocol_cancellation(spec, protocol, &mut intent, job, &key, args)
                        .await;
                }
            }
            if crate::tracing::now_ms().saturating_sub(started_ms) >= deadline_ms {
                // The deadline is a declared, checked transition target —
                // not an ad-hoc timeout error.
                intent.state = protocol.deadline_target.name.clone();
                self.checkpoint_protocol_intent(job, &key, &intent)?;
                return Err(RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: format!(
                        "provider protocol `{}` reached its declared {}s deadline and moved to \
                         terminal state `{}`",
                        spec.operation, protocol.deadline_secs, intent.state
                    ),
                });
            }

            // Governed cadence (slice 52h-3): never poll faster than the
            // DECLARED interval, and never faster than the provider's own
            // `Retry-After`. Taking the max means a provider can slow us
            // down but never speed us up past what the source declared.
            // (The connector's client-side rate limit admits each poll
            // inside the dispatch below, so the poll loop cannot outrun
            // the declared request budget either.)
            let declared = poll_delay_ms(protocol, intent.polls);
            let delay = retry_after_hint.map_or(declared, |asked| declared.max(asked));
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;

            let poll_spec = self.poll_spec_for(spec, protocol, &intent, args)?;
            // Circuit-breaker admission (slice 52h-3). A long-running
            // protocol must survive a provider's transient hiccup — a
            // single failed observation says nothing about the submitted
            // job, which is still out there. So a poll failure is
            // TOLERATED and retried on the next tick. But polling a
            // persistently broken provider forever is its own failure, so
            // `circuit_breaker: N` consecutive failures trips and gives
            // up. The declared deadline still bounds the whole loop.
            let (raw, meta) = match self
                .dispatch_connector_http_detailed(&poll_spec, &[])
                .await
            {
                Ok(ok) => {
                    consecutive_poll_failures = 0;
                    ok
                }
                Err(err) => {
                    consecutive_poll_failures += 1;
                    let threshold = spec.circuit_breaker.unwrap_or(u64::MAX);
                    if consecutive_poll_failures >= threshold {
                        return Err(RuntimeError::ToolFailed {
                            tool: spec.operation.clone(),
                            message: format!(
                                "provider protocol `{}`: circuit breaker open after {} \
                                 consecutive failed observations (last: {}). The submitted \
                                 provider job is NOT cancelled — the intent stays recorded in \
                                 state `{}` for a later resume.",
                                spec.operation,
                                consecutive_poll_failures,
                                err,
                                intent.state
                            ),
                        });
                    }
                    // Tolerated: keep the intent as-is and observe again
                    // on the next tick.
                    continue;
                }
            };
            retry_after_hint = meta.retry_after_ms;
            let poll_response = unwrap_ok_envelope(&raw);

            observe(protocol, &spec.operation, &mut intent, &poll_response).map_err(|e| {
                RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: e.message(),
                }
            })?;
            self.checkpoint_protocol_intent(job, &key, &intent)?;

            if is_terminal(protocol, &intent.state) {
                return Ok(poll_response);
            }
        }

        // Already terminal on entry — a resumed intent whose protocol
        // completed before the restart. Return the provider's recorded
        // terminal observation so the resumed call is indistinguishable
        // from the original, and NOTHING is re-submitted or re-polled.
        intent
            .last_response
            .clone()
            .ok_or_else(|| RuntimeError::ToolFailed {
                tool: spec.operation.clone(),
                message: format!(
                    "provider protocol `{}` resumed in terminal state `{}` but recorded no \
                     terminal observation",
                    spec.operation, intent.state
                ),
            })
    }

    /// Act on a cancelled durable job (slice 52h-3). The disposition is
    /// decided by the DECLARATION plus whether we already submitted — it
    /// is never an assumption about what the provider will tolerate.
    async fn apply_protocol_cancellation(
        &self,
        spec: &ConnectorHttpSpec,
        protocol: &ProviderProtocolDecl,
        intent: &mut ProtocolIntentState,
        job: &super::DurableJobContext,
        key: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, RuntimeError> {
        let message = match cancellation_disposition(protocol, intent) {
            // Nothing was submitted: no provider work exists, so this
            // cancellation is exact.
            CancellationDisposition::Cancelled => format!(
                "provider protocol `{}` was cancelled before submit — no provider job was created",
                spec.operation
            ),
            // A provider job exists AND the protocol declares how to
            // cancel it, so carry the cancellation to the provider.
            CancellationDisposition::Compensate => {
                let cancel = protocol
                    .cancel
                    .as_ref()
                    .expect("Compensate implies a declared cancel endpoint");
                let path = bind_protocol_path(
                    &cancel.path,
                    &spec.operation,
                    intent,
                    &spec.param_names,
                    args,
                )
                .map_err(|e| RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: e.message(),
                })?;
                let cancel_spec = self.request_spec_for(
                    spec,
                    cancel.method.as_str().to_string(),
                    path.clone(),
                );
                // A failed compensation must not be reported as a clean
                // cancellation — the provider job may still be running.
                self.dispatch_connector_http(&cancel_spec, &[])
                    .await
                    .map_err(|err| RuntimeError::ToolFailed {
                        tool: spec.operation.clone(),
                        message: format!(
                            "provider protocol `{}` was cancelled, but compensating via `{}` \
                             FAILED ({err}) — the provider job may still be running; the intent \
                             stays recorded in state `{}`",
                            spec.operation, path, intent.state
                        ),
                    })?;
                format!(
                    "provider protocol `{}` was cancelled and compensated via `{}`",
                    spec.operation, path
                )
            }
            // A provider job exists and the protocol declares no way to
            // cancel it. Detach and say so — never pretend it was undone.
            CancellationDisposition::Detached => format!(
                "provider protocol `{}` was cancelled after submit, but declares no `cancel` \
                 endpoint — DETACHED in state `{}`. The provider job is still running and is \
                 NOT cancelled; the intent stays recorded so it can be resumed or reconciled.",
                spec.operation, intent.state
            ),
        };
        // Whatever the disposition, the intent is recorded — the work is
        // accounted for rather than silently dropped.
        self.checkpoint_protocol_intent(job, key, intent)?;
        Err(RuntimeError::ToolFailed {
            tool: spec.operation.clone(),
            message,
        })
    }

    /// Build a connector spec for a protocol side-request (poll or
    /// cancel): same base URL and credentials as the operation, with an
    /// already-bound path.
    fn request_spec_for(
        &self,
        spec: &ConnectorHttpSpec,
        method: String,
        path: String,
    ) -> ConnectorHttpSpec {
        ConnectorHttpSpec {
            connector: spec.connector.clone(),
            operation: spec.operation.clone(),
            base_url: spec.base_url.clone(),
            method,
            path,
            param_names: Vec::new(),
            body: None,
            auth: spec.auth.clone(),
            error_map: Vec::new(),
            returns_result: false,
            retry: spec.retry,
            rate_limit: spec.rate_limit,
            circuit_breaker: spec.circuit_breaker,
        }
    }

    /// Synthesize the poll request as a connector spec: same base URL and
    /// credentials as the operation, the protocol's declared (GET) poll
    /// method, and a path already bound from the submit response fields
    /// plus the call arguments.
    fn poll_spec_for(
        &self,
        spec: &ConnectorHttpSpec,
        protocol: &ProviderProtocolDecl,
        intent: &ProtocolIntentState,
        args: &[serde_json::Value],
    ) -> Result<ConnectorHttpSpec, RuntimeError> {
        let path = bind_poll_path(
            protocol,
            &spec.operation,
            intent,
            &spec.param_names,
            args,
        )
        .map_err(|e| RuntimeError::ToolFailed {
            tool: spec.operation.clone(),
            message: e.message(),
        })?;
        Ok(ConnectorHttpSpec {
            connector: spec.connector.clone(),
            operation: spec.operation.clone(),
            base_url: spec.base_url.clone(),
            method: protocol.poll.method.as_str().to_string(),
            // Already bound; no placeholders remain, so no params.
            path,
            param_names: Vec::new(),
            body: None,
            auth: spec.auth.clone(),
            // A poll observes; it never maps statuses to typed errors and
            // never returns a Result envelope.
            error_map: Vec::new(),
            returns_result: false,
            retry: spec.retry,
            rate_limit: spec.rate_limit,
            circuit_breaker: spec.circuit_breaker,
        })
    }

    /// Load the intent for this logical call from the durable job's
    /// checkpoints, or start a fresh one at the declared initial state.
    fn load_protocol_intent(
        &self,
        job: &super::DurableJobContext,
        key: &str,
        protocol: &ProviderProtocolDecl,
    ) -> Result<ProtocolIntentState, RuntimeError> {
        let checkpoints = job.queue.list_checkpoints(&job.job_id)?;
        let resumed = checkpoints
            .iter()
            .rev()
            .find(|c| c.label == key)
            .and_then(|c| serde_json::from_value::<ProtocolIntentState>(c.payload.clone()).ok());
        Ok(resumed.unwrap_or_else(|| ProtocolIntentState::new(protocol)))
    }

    /// Persist the intent as the job's newest checkpoint under this
    /// call's idempotency key.
    fn checkpoint_protocol_intent(
        &self,
        job: &super::DurableJobContext,
        key: &str,
        intent: &ProtocolIntentState,
    ) -> Result<(), RuntimeError> {
        let payload = serde_json::to_value(intent).map_err(|e| RuntimeError::Other(
            format!("failed to serialize provider-protocol intent: {e}"),
        ))?;
        job.queue.record_checkpoint(
            &job.job_id,
            JobCheckpointKind::PartialOutput,
            key.to_string(),
            payload,
            None,
        )?;
        Ok(())
    }
}

/// Connector dispatch wraps a success in `{"tag":"ok","ok":…}` when the
/// operation returns a `Result`. Protocol binding and status extraction
/// work on the provider's own payload, so unwrap that envelope first.
fn unwrap_ok_envelope(value: &serde_json::Value) -> serde_json::Value {
    if value.get("tag").and_then(|t| t.as_str()) == Some("ok") {
        if let Some(inner) = value.get("ok") {
            return inner.clone();
        }
    }
    value.clone()
}
