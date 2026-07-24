//! Durable verified-provider-protocol execution.
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
//! 5. **The lifecycle is replayable**. Every provider
//!    exchange — submit, each observation, and the compensation — goes
//!    through [`Runtime::protocol_exchange`], which records it as a
//!    substitutable trace event and, when replaying, serves it from the
//!    recording instead of the provider.
//!
//! Mock, replay, and real all share this engine. Mock is decided below
//! this file (at the connector dispatch); replay is decided HERE, in
//! `protocol_exchange`, because the protocol driver deliberately bypasses
//! `call_tool` — it needs response metadata and its own durability — and
//! so must honour the same no-real-fallback contract itself.

use super::Runtime;
use crate::connectors::ConnectorHttpSpec;
use crate::errors::RuntimeError;
use crate::runtime::connector_dispatch::ConnectorResponseMeta;
use crate::tracing::now_ms;
use corvid_trace_schema::TraceEvent;
use crate::protocol::{
    bind_poll_path, bind_protocol_path, cancellation_disposition, intent_idempotency_key,
    is_terminal, observe, poll_delay_ms, resume_verdict, CancellationDisposition,
    ProtocolIntentState, ResumeVerdict,
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
        let mut intent = self.load_protocol_intent(job, &key, protocol, &spec.operation)?;

        // (1) Record the intent BEFORE any submit, so a crash here is
        // recoverable and a resume re-finds it instead of re-submitting.
        if !intent.submitted {
            self.checkpoint_protocol_intent(job, &key, &intent)?;

            // (2) Submit, then bind the provider job id from the DECODED
            // response — never before it is typed.
            let (response, _) = self.protocol_exchange(&spec.operation, spec, args).await?;
            intent.bind_submit_response(&unwrap_ok_envelope(&response));
            self.checkpoint_protocol_intent(job, &key, &intent)?;
            // The lifecycle is evidence, not just control flow. `intent`
            // is a SHA-256 of (connector, operation, args), so it
            // correlates an operator's view of this work across restarts
            // without carrying the arguments themselves.
            self.emit_host_event(
                "protocol.submitted",
                serde_json::json!({
                    "connector": spec.connector,
                    "operation": spec.operation,
                    "intent": key,
                    "state": intent.state,
                }),
            );
        }

        // (3) Observe until a declared terminal state, or the deadline
        // forces the declared deadline target.
        let started_ms = crate::tracing::now_ms();
        let deadline_ms = protocol.deadline_secs.saturating_mul(1000);
        // The provider's most recent `Retry-After`, if it asked us to
        // back off.
        let mut retry_after_hint: Option<u64> = None;
        // Consecutive failed observations, for circuit-breaker admission.
        // Reset by any successful poll.
        let mut consecutive_poll_failures: u64 = 0;
        loop {
            if is_terminal(protocol, &intent.state) {
                break;
            }
            // Semantic cancellation. The durable job is the
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
                self.emit_protocol_settled(spec, &key, &intent, "deadline");
                return Err(RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: format!(
                        "provider protocol `{}` reached its declared {}s deadline and moved to \
                         terminal state `{}`",
                        spec.operation, protocol.deadline_secs, intent.state
                    ),
                });
            }

            // Governed cadence: never poll faster than the
            // DECLARED interval, and never faster than the provider's own
            // `Retry-After`. Taking the max means a provider can slow us
            // down but never speed us up past what the source declared.
            // (The connector's client-side rate limit admits each poll
            // inside the dispatch below, so the poll loop cannot outrun
            // the declared request budget either.)
            let declared = poll_delay_ms(protocol, intent.polls);
            let delay = retry_after_hint.map_or(declared, |asked| declared.max(asked));
            // A REPLAYED lifecycle does not re-live the wall clock. The
            // recorded observation sequence already encodes
            // the cadence that actually happened; sleeping it again would
            // make replaying a day-long protocol take a day, and would
            // race the declared deadline against replay wall-time rather
            // than against the provider.
            if self.replay_source()?.is_none() {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }

            let poll_spec = self.poll_spec_for(spec, protocol, &intent, args)?;
            let poll_label = format!("{}.poll", spec.operation);
            // Circuit-breaker admission. A long-running
            // protocol must survive a provider's transient hiccup — a
            // single failed observation says nothing about the submitted
            // job, which is still out there. So a poll failure is
            // TOLERATED and retried on the next tick. But polling a
            // persistently broken provider forever is its own failure, so
            // `circuit_breaker: N` consecutive failures trips and gives
            // up. The declared deadline still bounds the whole loop.
            let (raw, meta) = match self.protocol_exchange(&poll_label, &poll_spec, &[]).await {
                Ok(ok) => {
                    consecutive_poll_failures = 0;
                    ok
                }
                Err(err) => {
                    // A replay divergence is NOT a provider hiccup, so the
                    // breaker must not absorb it. In replay
                    // there IS no provider to be transiently unwell — a
                    // divergence means the RECORDING does not cover this
                    // observation, which no amount of retrying can fix.
                    // Tolerating it would also spin the loop against a
                    // cursor that never advances until the deadline, and
                    // report a gap in the trace as a provider timeout.
                    if matches!(err, RuntimeError::ReplayDivergence(_)) {
                        return Err(err);
                    }
                    consecutive_poll_failures += 1;
                    let threshold = spec.circuit_breaker.unwrap_or(u64::MAX);
                    if consecutive_poll_failures >= threshold {
                        self.emit_protocol_settled(spec, &key, &intent, "breaker_open");
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

            let from_state = intent.state.clone();
            observe(protocol, &spec.operation, &mut intent, &poll_response).map_err(|e| {
                RuntimeError::ToolFailed {
                    tool: spec.operation.clone(),
                    message: e.message(),
                }
            })?;
            self.checkpoint_protocol_intent(job, &key, &intent)?;
            // The observed status and the transition it caused — the
            // declaration's own vocabulary, never the provider's payload.
            self.emit_host_event(
                "protocol.transition",
                serde_json::json!({
                    "connector": spec.connector,
                    "operation": spec.operation,
                    "intent": key,
                    "status": intent.status_history.last(),
                    "from": from_state,
                    "to": intent.state,
                    "polls": intent.polls,
                }),
            );

            if is_terminal(protocol, &intent.state) {
                self.emit_protocol_settled(spec, &key, &intent, "terminal");
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

    /// Record how an intent stopped being in flight.
    ///
    /// `outcome` distinguishes the endings that look alike from outside
    /// but are very different to the operator holding the provider job:
    /// `terminal` (the provider finished), `deadline` (it never did),
    /// `breaker_open` (we stopped observing, the job is still running),
    /// `cancelled` / `compensated` / `detached`.
    fn emit_protocol_settled(
        &self,
        spec: &ConnectorHttpSpec,
        key: &str,
        intent: &ProtocolIntentState,
        outcome: &str,
    ) {
        self.emit_host_event(
            "protocol.settled",
            serde_json::json!({
                "connector": spec.connector,
                "operation": spec.operation,
                "intent": key,
                "outcome": outcome,
                "state": intent.state,
                "polls": intent.polls,
                "submitted": intent.submitted,
            }),
        );
    }

    /// One provider exchange, through the same record/replay bracket
    /// ordinary tool calls get.
    ///
    /// `call_tool` consults the replay source BEFORE dispatching, which
    /// is what gives every other effect strict no-real-fallback. The
    /// protocol driver does not go through `call_tool`, so without this
    /// bracket a replayed lifecycle would reach for the live provider —
    /// re-submitting work that already happened. Here the replay source
    /// is consulted first and there is no path from that branch to the
    /// network.
    ///
    /// `label` names the lifecycle boundary — the operation itself for
    /// the submit, `<op>.poll` for an observation, `<op>.cancel` for the
    /// compensation — so a recorded observation can never be substituted
    /// for a submit. Observations deliberately SHARE one label: the
    /// replay cursor is strictly positional, so N recorded observations
    /// reproduce in exactly the order the provider produced them, which
    /// is the whole point of replaying a lifecycle rather than a call.
    async fn protocol_exchange(
        &self,
        label: &str,
        spec: &ConnectorHttpSpec,
        args: &[serde_json::Value],
    ) -> Result<(serde_json::Value, ConnectorResponseMeta), RuntimeError> {
        if self.tracer.is_enabled() {
            self.tracer.emit(TraceEvent::ToolCall {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: label.to_string(),
                args: args.to_vec(),
            });
        }
        let dispatched = match self.replay_source()? {
            // A replayed exchange carries no live response metadata: the
            // cadence the provider asked for is already baked into the
            // recorded sequence of observations, and no caller reads
            // `status` off this value (the status→error mapping happens
            // inside the dispatch, before the payload was recorded).
            Some(replay) => replay
                .replay_tool_call(label, args)
                .map(|value| (value, ConnectorResponseMeta::default())),
            None => self.dispatch_connector_http_detailed(spec, args).await,
        };
        // A FAILED exchange is recorded too, and as a substitutable
        // result — otherwise replay would reproduce a lifecycle in which
        // the provider never faltered, and the circuit-breaker tolerance
        // path (the one worth replaying) would silently vanish.
        let (payload, meta) = match dispatched {
            Ok(ok) => ok,
            Err(err) => {
                if self.tracer.is_enabled() {
                    self.tracer.emit(TraceEvent::ToolResult {
                        ts_ms: now_ms(),
                        run_id: self.tracer.run_id().to_string(),
                        tool: label.to_string(),
                        result: serde_json::json!({
                            crate::replay::CORVID_TOOL_ERROR_KEY: err.to_string(),
                        }),
                    });
                }
                return Err(err);
            }
        };
        if self.tracer.is_enabled() {
            // The credential never reaches here: it is resolved and
            // attached to a header inside the dispatch, so neither the
            // recorded args nor the recorded payload can carry it.
            self.tracer.emit(TraceEvent::ToolResult {
                ts_ms: now_ms(),
                run_id: self.tracer.run_id().to_string(),
                tool: label.to_string(),
                result: payload.clone(),
            });
        }
        Ok((payload, meta))
    }

    /// Act on a cancelled durable job. The disposition is
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
        let outcome = match cancellation_disposition(protocol, intent) {
            CancellationDisposition::Cancelled => "cancelled",
            CancellationDisposition::Compensate => "compensated",
            CancellationDisposition::Detached => "detached",
        };
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
                self.protocol_exchange(
                    &format!("{}.cancel", spec.operation),
                    &cancel_spec,
                    &[],
                )
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
        // accounted for rather than silently dropped. `compensated` is
        // only emitted once the compensating call SUCCEEDED, since the
        // failure path returns above.
        self.checkpoint_protocol_intent(job, key, intent)?;
        self.emit_protocol_settled(spec, key, intent, outcome);
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
        protocol_operation: &str,
    ) -> Result<ProtocolIntentState, RuntimeError> {
        let checkpoints = job.queue.list_checkpoints(&job.job_id)?;
        let resumed = checkpoints
            .iter()
            .rev()
            .find(|c| c.label == key)
            .and_then(|c| serde_json::from_value::<ProtocolIntentState>(c.payload.clone()).ok());
        let Some(intent) = resumed else {
            return Ok(ProtocolIntentState::new(protocol));
        };

        // The protocol may have changed under an intent that is already
        // in flight. Resuming a live provider job against a graph it was
        // never started under is a consequential decision, so the
        // DECLARATION makes it — never this dispatcher.
        let verdict = resume_verdict(protocol, &intent);

        // Every outcome is recorded, including the uneventful one: an
        // audit needs to see that a resume was CHECKED and matched, not
        // infer it from the absence of a complaint. Declarations and
        // state names only — no provider payload, no credential.
        self.emit_host_event(
            "protocol.resume_decision",
            serde_json::json!({
                "operation": protocol_operation,
                "decision": verdict.decision(),
                "state": intent.state,
                "submitted": intent.submitted,
                "recorded_protocol": intent
                    .protocol_canonical
                    .as_deref()
                    .map(ProviderProtocolDecl::fingerprint_of),
                "running_protocol": protocol.fingerprint(),
                "changes": verdict.changes(),
            }),
        );

        let still_running = if intent.submitted {
            " and the provider job it created is still running"
        } else {
            ""
        };
        match verdict {
            ResumeVerdict::Unchanged | ResumeVerdict::ResumedAcrossChange { .. } => Ok(intent),
            ResumeVerdict::RefusedByPolicy { changes } => Err(RuntimeError::ToolFailed {
                tool: protocol_operation.to_string(),
                message: format!(
                    "provider protocol `{protocol_operation}` changed while an intent was in \
                     flight, and the declaration says `on_protocol_change: refuse`. The intent \
                     stays checkpointed in state `{}` — nothing was re-submitted or \
                     re-polled{still_running}. What changed: {}",
                    intent.state,
                    describe_changes(&changes)
                ),
            }),
            ResumeVerdict::RefusedStateVanished { state, changes } => {
                Err(RuntimeError::ToolFailed {
                    tool: protocol_operation.to_string(),
                    message: format!(
                        "provider protocol `{protocol_operation}` declares \
                         `on_protocol_change: resume`, but the in-flight intent's state `{state}` \
                         no longer exists in the new declaration, so there is no sound point to \
                         resume from. The intent stays checkpointed{still_running}. Re-add the \
                         state, or move the protocol to `on_protocol_change: refuse` and \
                         reconcile deliberately. What changed: {}",
                        describe_changes(&changes)
                    ),
                })
            }
        }
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
/// Render the declaration differences for a diagnostic. An operator
/// holding a live provider job needs to know WHAT changed — two hashes
/// would tell them only that something did.
fn describe_changes(changes: &[String]) -> String {
    if changes.is_empty() {
        "the declaration differs but no canonical key did (report this)".to_string()
    } else {
        changes.join("; ")
    }
}

fn unwrap_ok_envelope(value: &serde_json::Value) -> serde_json::Value {
    if value.get("tag").and_then(|t| t.as_str()) == Some("ok") {
        if let Some(inner) = value.get("ok") {
            return inner.clone();
        }
    }
    value.clone()
}
