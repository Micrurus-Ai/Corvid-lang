//! Verified provider protocol execution core.
//!
//! A long-running provider protocol is *language data*: an
//! `async:` block on a connector operation declaring the status universe,
//! initial/terminal states, deadline, poll request, cadence, and TOTAL
//! transition tables — with the checker proving graph closure,
//! reachability, non-zero bounds, a non-mutating poll, and that a
//! mutating submit passes the `dangerous` approval boundary.
//!
//! This module is the PURE half of executing that protocol: the
//! transition engine and the binding conventions. It performs no I/O and
//! touches no durable state, so the semantics are unit-testable in
//! isolation — which matters, because this is the layer that decides
//! whether a real provider job is submitted once, twice, or never.
//!
//! The durable half (persisting the intent before submit, binding the
//! provider job id from the typed response, checkpointing each
//! transition, and resuming after a crash) lives in
//! `runtime/protocol_dispatch.rs` and drives this engine.
//!
//! ## Binding conventions
//!
//! The declaration names statuses and states but not *where* in a
//! provider payload they live. These are the conventions (chosen so the
//! declaration grammar needs no amendment):
//!
//! - The submit response decodes to the operation's return shape. Its
//!   top-level fields become **binding fields** — so a response `id`
//!   satisfies a `poll GET "/jobs/{id}"` placeholder. This is how the
//!   provider job id reaches the poll request, and it is bound only
//!   AFTER a typed response, never guessed before submit.
//! - Poll-path placeholders resolve against the submit-response fields
//!   first, then the original call arguments by parameter name.
//! - The poll response carries a `status` field whose value must be one
//!   of the declared statuses. An unknown status is a protocol error,
//!   never a silent no-op — a provider that invents a status must not
//!   cross the typed boundary.

use corvid_ast::{ProtocolPollInterval, ProviderProtocolDecl};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A failure applying the declared protocol to an observed provider
/// payload. Every variant names the protocol element involved so the
/// diagnostic points at the declaration, not the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// The poll response carried no usable `status` field.
    MissingStatus { operation: String },
    /// The provider reported a status outside the declared universe.
    UnknownStatus { operation: String, status: String },
    /// The current state has no transition for the observed status.
    /// (The checker proves transition tables are total, so this means
    /// the runtime state itself is not a declared state.)
    UnknownState { operation: String, state: String },
    /// A poll-path placeholder could not be bound from the submit
    /// response fields or the call arguments.
    UnboundPollPlaceholder { operation: String, placeholder: String },
}

impl ProtocolError {
    pub fn message(&self) -> String {
        match self {
            Self::MissingStatus { operation } => format!(
                "provider protocol `{operation}`: the poll response carries no `status` field \
                 (the declared statuses are matched against it)"
            ),
            Self::UnknownStatus { operation, status } => format!(
                "provider protocol `{operation}`: the provider reported status `{status}`, which \
                 is not in the declared status universe"
            ),
            Self::UnknownState { operation, state } => format!(
                "provider protocol `{operation}`: no declared state named `{state}`"
            ),
            Self::UnboundPollPlaceholder {
                operation,
                placeholder,
            } => format!(
                "provider protocol `{operation}`: the poll path references `{{{placeholder}}}`, \
                 which is bound by neither a submit-response field nor a call argument"
            ),
        }
    }
}

/// The durable state of one protocol intent. Serialized into a queue-job
/// checkpoint after every transition, so a restart resumes exactly where
/// the last observation left off instead of re-submitting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolIntentState {
    /// The current protocol state (a declared state name).
    pub state: String,
    /// Whether the submit request has been made. Set only after a typed
    /// response — the write-before-submit intent row is what makes a
    /// crash between "intent recorded" and "submit acknowledged" safe.
    pub submitted: bool,
    /// Top-level fields of the decoded submit response, which bind the
    /// poll path (this is where the provider job id lives). Empty until
    /// a typed response arrives.
    pub binding_fields: BTreeMap<String, serde_json::Value>,
    /// Every status observed, in order — the transition evidence.
    pub status_history: Vec<String>,
    /// Number of completed polls (drives the adaptive cadence).
    pub polls: u64,
    /// The most recent decoded poll payload. Persisted so a run that
    /// resumes an ALREADY-terminal intent returns the provider's real
    /// terminal observation instead of re-deriving one — the resumed
    /// call must be indistinguishable from the original.
    #[serde(default)]
    pub last_response: Option<serde_json::Value>,
    /// The canonical encoding of the protocol graph this intent was
    /// created under. A restart compares it against the running
    /// declaration and applies the declared `on_protocol_change` posture.
    ///
    /// The full ENCODING is stored, not just its fingerprint, because two
    /// hashes can only say that something changed. Recovering a live
    /// provider job needs to know what: a vanished terminal state and a
    /// one-minute deadline change are very different situations for the
    /// operator holding it.
    ///
    /// `None` means the checkpoint predates this record. It is treated as
    /// a change, because "we cannot tell" is not "it is the same".
    #[serde(default)]
    pub protocol_canonical: Option<String>,
    /// When this intent was created, in epoch milliseconds.
    ///
    /// The declared deadline is a bound on the PROTOCOL, not on a process.
    /// Measuring it from process start would hand a repeatedly restarted
    /// protocol a fresh full window every time, so a crash-looping
    /// deployment could poll a provider forever while still "respecting"
    /// a 10-minute deadline. It also silently breaks the budget bound,
    /// which is computed from one deadline window.
    ///
    /// `None` means the checkpoint predates this field; such an intent
    /// falls back to measuring from now, which is the old behaviour and
    /// the only thing available for it.
    #[serde(default)]
    pub created_ms: Option<u64>,
}

/// The outcome of comparing an in-flight intent against the running
/// declaration. Separated from the dispatcher so the decision is testable
/// without a provider, a queue, or a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeVerdict {
    /// The protocol is unchanged — resume normally.
    Unchanged,
    /// The protocol changed, the declaration permits resuming, and the
    /// recorded state still exists in the new graph.
    ResumedAcrossChange { changes: Vec<String> },
    /// The protocol changed and the declaration says refuse.
    RefusedByPolicy { changes: Vec<String> },
    /// The declaration permits resuming, but the recorded state is gone
    /// from the new graph. Refused regardless of the policy: a resume that
    /// cannot find its own state is not a resume.
    RefusedStateVanished {
        state: String,
        changes: Vec<String>,
    },
}

impl ResumeVerdict {
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            Self::RefusedByPolicy { .. } | Self::RefusedStateVanished { .. }
        )
    }

    /// The name recorded in the decision event.
    pub fn decision(&self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::ResumedAcrossChange { .. } => "resumed_across_change",
            Self::RefusedByPolicy { .. } => "refused_by_policy",
            Self::RefusedStateVanished { .. } => "refused_state_vanished",
        }
    }

    /// What differs between the recorded and running declarations.
    pub fn changes(&self) -> &[String] {
        match self {
            Self::Unchanged => &[],
            Self::ResumedAcrossChange { changes }
            | Self::RefusedByPolicy { changes }
            | Self::RefusedStateVanished { changes, .. } => changes,
        }
    }
}

/// Decide whether an intent recorded under an earlier declaration may
/// resume under the running one.
///
/// The comparison is over a canonical encoding of the protocol GRAPH,
/// derived from the declaration rather than hand-maintained — an author
/// cannot forget to bump it.
pub fn resume_verdict(
    protocol: &ProviderProtocolDecl,
    intent: &ProtocolIntentState,
) -> ResumeVerdict {
    let current = protocol.canonical_encoding();
    // A fresh intent has nothing to migrate. Only an intent that already
    // exists — and, when submitted, corresponds to a live provider job —
    // can be stranded by a change.
    let recorded = match &intent.protocol_canonical {
        Some(recorded) if *recorded == current => return ResumeVerdict::Unchanged,
        Some(recorded) => recorded.clone(),
        None if !intent.submitted && intent.status_history.is_empty() => {
            return ResumeVerdict::Unchanged
        }
        None => String::new(),
    };
    let changes = if recorded.is_empty() {
        vec!["the checkpoint predates protocol change detection".to_string()]
    } else {
        corvid_ast::protocol_canonical_differences(&recorded, &current)
    };
    match protocol.on_protocol_change {
        corvid_ast::ProtocolChangePolicy::Refuse => ResumeVerdict::RefusedByPolicy { changes },
        corvid_ast::ProtocolChangePolicy::Resume => {
            if protocol.states.iter().any(|s| s.name.name == intent.state)
                || protocol.terminal.iter().any(|t| t.name == intent.state)
            {
                ResumeVerdict::ResumedAcrossChange { changes }
            } else {
                ResumeVerdict::RefusedStateVanished {
                    state: intent.state.clone(),
                    changes,
                }
            }
        }
    }
}

impl ProtocolIntentState {
    /// The state of a freshly recorded intent: at the declared initial
    /// state, nothing submitted, nothing observed.
    pub fn new(protocol: &ProviderProtocolDecl) -> Self {
        Self {
            state: protocol.initial.name.clone(),
            submitted: false,
            binding_fields: BTreeMap::new(),
            status_history: Vec::new(),
            polls: 0,
            last_response: None,
            protocol_canonical: Some(protocol.canonical_encoding()),
            created_ms: Some(crate::tracing::now_ms()),
        }
    }

    /// Milliseconds remaining before the declared deadline forces the
    /// deadline target, measured from intent CREATION so restarts cannot
    /// extend it. Zero once the deadline has passed.
    pub fn deadline_remaining_ms(&self, deadline_secs: u64, now_ms: u64) -> u64 {
        let budget = deadline_secs.saturating_mul(1000);
        match self.created_ms {
            Some(created) => budget.saturating_sub(now_ms.saturating_sub(created)),
            // Pre-dates the field: the only honest reading is a full
            // window from now, which is what it already had.
            None => budget,
        }
    }

    /// Record the decoded submit response: bind its top-level fields
    /// (the provider job id among them) and mark the intent submitted.
    pub fn bind_submit_response(&mut self, response: &serde_json::Value) {
        if let serde_json::Value::Object(map) = response {
            for (k, v) in map {
                self.binding_fields.insert(k.clone(), v.clone());
            }
        }
        self.submitted = true;
    }
}

/// Is `state` a declared terminal state?
pub fn is_terminal(protocol: &ProviderProtocolDecl, state: &str) -> bool {
    protocol.terminal.iter().any(|t| t.name == state)
}

/// What cancelling an intent may actually do.
///
/// This is the cancellation×irreversibility rule composed with DURABLE
/// PROVIDER STATE. Before submit, nothing exists and cancelling is exact.
/// After submit a provider-side job exists, and what Corvid may honestly
/// do depends on whether the protocol DECLARED a cancel endpoint:
/// with one, it can compensate; without one, the only truthful option is
/// to detach and record, because dropping the intent would leave real
/// work running with nobody observing it — the silent-orphan failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationDisposition {
    /// Nothing was submitted, so no provider work exists. Cancelling is
    /// exact: the intent simply never happened.
    Cancelled,
    /// A provider job exists AND the protocol declares a cancel endpoint,
    /// so the cancellation can be carried to the provider.
    Compensate,
    /// A provider job exists and the protocol declares no way to cancel
    /// it. The intent stops being awaited but is recorded as detached, so
    /// the work is accounted for rather than silently orphaned.
    Detached,
}

/// Decide what cancelling this intent means. Past the irreversible
/// boundary (a submitted provider job), cancellation either compensates
/// through the declared endpoint or degrades to detach-and-record — it
/// never pretends the work was undone.
pub fn cancellation_disposition(
    protocol: &ProviderProtocolDecl,
    intent: &ProtocolIntentState,
) -> CancellationDisposition {
    match (intent.submitted, protocol.cancel.is_some()) {
        (false, _) => CancellationDisposition::Cancelled,
        (true, true) => CancellationDisposition::Compensate,
        (true, false) => CancellationDisposition::Detached,
    }
}

/// Extract the provider status from a decoded poll response, per the
/// `status`-field convention, and validate it against the declared
/// universe. An undeclared status is an error, never a silent skip.
pub fn extract_status(
    protocol: &ProviderProtocolDecl,
    operation: &str,
    response: &serde_json::Value,
) -> Result<String, ProtocolError> {
    let raw = response
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ProtocolError::MissingStatus {
            operation: operation.to_string(),
        })?;
    if !protocol.statuses.iter().any(|s| s.name == raw) {
        return Err(ProtocolError::UnknownStatus {
            operation: operation.to_string(),
            status: raw.to_string(),
        });
    }
    Ok(raw.to_string())
}

/// Apply an observed status to the current state using the declared
/// transition table. The checker proved every state handles every
/// declared status exactly once, so a missing transition here means the
/// runtime state is not a declared state.
pub fn next_state(
    protocol: &ProviderProtocolDecl,
    operation: &str,
    current: &str,
    status: &str,
) -> Result<String, ProtocolError> {
    let state = protocol
        .states
        .iter()
        .find(|s| s.name.name == current)
        .ok_or_else(|| ProtocolError::UnknownState {
            operation: operation.to_string(),
            state: current.to_string(),
        })?;
    state
        .transitions
        .iter()
        .find(|t| t.status.name == status)
        .map(|t| t.target.name.clone())
        .ok_or_else(|| ProtocolError::UnknownStatus {
            operation: operation.to_string(),
            status: status.to_string(),
        })
}

/// Advance the intent by one observation: extract the status, apply the
/// transition, and record the evidence. Returns the new state.
pub fn observe(
    protocol: &ProviderProtocolDecl,
    operation: &str,
    intent: &mut ProtocolIntentState,
    poll_response: &serde_json::Value,
) -> Result<String, ProtocolError> {
    let status = extract_status(protocol, operation, poll_response)?;
    let next = next_state(protocol, operation, &intent.state, &status)?;
    intent.state = next.clone();
    intent.status_history.push(status);
    intent.polls = intent.polls.saturating_add(1);
    intent.last_response = Some(poll_response.clone());
    Ok(next)
}

/// Bind the poll path's `{placeholder}` segments from the submit-response
/// binding fields first, then the original call arguments by parameter
/// name. Reuses the connector request builder's single `fill_path`
/// implementation so placeholder semantics never diverge.
pub fn bind_poll_path(
    protocol: &ProviderProtocolDecl,
    operation: &str,
    intent: &ProtocolIntentState,
    param_names: &[String],
    args: &[serde_json::Value],
) -> Result<String, ProtocolError> {
    bind_protocol_path(&protocol.poll.path, operation, intent, param_names, args)
}

/// Bind any protocol path template (poll or cancel) from the
/// submit-response fields plus the call arguments. One implementation so
/// the two endpoints can never disagree about placeholder semantics.
pub fn bind_protocol_path(
    template: &str,
    operation: &str,
    intent: &ProtocolIntentState,
    param_names: &[String],
    args: &[serde_json::Value],
) -> Result<String, ProtocolError> {
    let mut by_name: BTreeMap<&str, &serde_json::Value> = BTreeMap::new();
    // Call arguments first, then submit-response fields, so a response
    // field (the authoritative provider job id) wins over an argument of
    // the same name.
    for (name, value) in param_names.iter().map(|n| n.as_str()).zip(args.iter()) {
        by_name.insert(name, value);
    }
    for (name, value) in &intent.binding_fields {
        by_name.insert(name.as_str(), value);
    }
    crate::connectors::fill_path(template, &by_name, operation).map_err(|_| {
        // `fill_path` reports a missing binding as a request error; at
        // the protocol layer we name the placeholder precisely.
        let missing =
            first_unbound_placeholder(template, &by_name).unwrap_or_else(|| "?".to_string());
        ProtocolError::UnboundPollPlaceholder {
            operation: operation.to_string(),
            placeholder: missing,
        }
    })
}

fn first_unbound_placeholder(
    path: &str,
    by_name: &BTreeMap<&str, &serde_json::Value>,
) -> Option<String> {
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let name = &after[..close];
        if !by_name.contains_key(name) {
            return Some(name.to_string());
        }
        rest = &after[close + 1..];
    }
    None
}

/// The poll cadence in milliseconds for the next observation. An adaptive
/// cadence backs off linearly from the fixed floor so a long protocol does
/// not hammer the provider. The dispatcher governs it further, never
/// polling faster than this or than the provider's own `Retry-After`.
pub fn poll_delay_ms(protocol: &ProviderProtocolDecl, polls: u64) -> u64 {
    match protocol.interval {
        ProtocolPollInterval::FixedSeconds(secs) => secs.saturating_mul(1000).max(1),
        ProtocolPollInterval::Adaptive => {
            let base = 1_000_u64;
            base.saturating_mul(polls.saturating_add(1).min(30))
        }
    }
}

/// Which logical invocation this is: WHERE in the program the call was
/// written, and WHICH execution of that callsite it is within the job.
///
/// Arguments alone are not an identity. "Ship order-1" written twice, or
/// written once inside a loop that runs twice, is two intentional pieces
/// of work — keying on `(connector, operation, args)` alone silently
/// collapsed them into one intent, so the second call returned the
/// first's result and its provider job was never created. Two parallel
/// identical calls raced for the same row.
///
/// `callsite` is the source position of the call expression, so distinct
/// call sites never collide. `ordinal` counts executions of that callsite
/// within the durable job, so a loop produces distinct intents. A resumed
/// run re-executes the agent from the start, so both are reproduced
/// deterministically and a resume still re-finds its own intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvocationIdentity {
    pub callsite: u32,
    pub ordinal: u64,
}

/// The durable idempotency key for an intent. Derived from the connector,
/// operation, the call's arguments, AND the logical invocation identity,
/// so the same logical call maps to the same durable row — a retry or a
/// restart re-finds the existing intent instead of submitting a second
/// provider job — while two DIFFERENT calls that happen to share
/// arguments remain two intents.
pub fn intent_idempotency_key(
    connector: &str,
    operation: &str,
    args: &[serde_json::Value],
    invocation: InvocationIdentity,
) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    let mut hasher = Sha256::new();
    hasher.update(connector.as_bytes());
    hasher.update(b"\0");
    hasher.update(operation.as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical.as_bytes());
    hasher.update(b"\0");
    hasher.update(invocation.callsite.to_be_bytes());
    hasher.update(b"\0");
    hasher.update(invocation.ordinal.to_be_bytes());
    format!("protocol:{connector}:{operation}:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{
        HttpMethod, Ident, ProtocolIdempotency, ProtocolPoll, ProtocolStateDecl,
        ProtocolTransition, Span,
    };
    use serde_json::json;

    fn id(name: &str) -> Ident {
        Ident::new(name.to_string(), Span::new(0, 0))
    }

    /// submitted --running--> submitted, --done--> complete, --failed--> failed
    fn protocol() -> ProviderProtocolDecl {
        let transitions = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(s, t)| ProtocolTransition {
                    status: id(s),
                    target: id(t),
                    span: Span::new(0, 0),
                })
                .collect::<Vec<_>>()
        };
        ProviderProtocolDecl {
            statuses: vec![id("running"), id("done"), id("failed")],
            initial: id("submitted"),
            terminal: vec![id("complete"), id("failed_out")],
            deadline_secs: 600,
            deadline_target: id("failed_out"),
            idempotency: ProtocolIdempotency::Intent,
            poll: ProtocolPoll {
                method: HttpMethod::Get,
                path: "/jobs/{id}".to_string(),
                span: Span::new(0, 0),
            },
            cancel: None,
            interval: ProtocolPollInterval::FixedSeconds(5),
            on_protocol_change: corvid_ast::ProtocolChangePolicy::Refuse,
            states: vec![
                ProtocolStateDecl {
                    name: id("submitted"),
                    transitions: transitions(&[
                        ("running", "submitted"),
                        ("done", "complete"),
                        ("failed", "failed_out"),
                    ]),
                    span: Span::new(0, 0),
                },
                ProtocolStateDecl {
                    name: id("complete"),
                    transitions: transitions(&[
                        ("running", "complete"),
                        ("done", "complete"),
                        ("failed", "complete"),
                    ]),
                    span: Span::new(0, 0),
                },
                ProtocolStateDecl {
                    name: id("failed_out"),
                    transitions: transitions(&[
                        ("running", "failed_out"),
                        ("done", "failed_out"),
                        ("failed", "failed_out"),
                    ]),
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn a_new_intent_starts_at_the_declared_initial_state_unsubmitted() {
        let p = protocol();
        let intent = ProtocolIntentState::new(&p);
        assert_eq!(intent.state, "submitted");
        assert!(!intent.submitted);
        assert!(intent.binding_fields.is_empty());
        assert_eq!(intent.polls, 0);
    }

    #[test]
    fn the_provider_job_id_binds_only_from_a_typed_submit_response() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        // Before the response there is nothing to bind the poll path with.
        let unbound = bind_poll_path(&p, "ship", &intent, &["order".into()], &[json!("o-1")]);
        assert_eq!(
            unbound,
            Err(ProtocolError::UnboundPollPlaceholder {
                operation: "ship".to_string(),
                placeholder: "id".to_string()
            })
        );

        intent.bind_submit_response(&json!({"id": "prov-42", "status": "running"}));
        assert!(intent.submitted);
        let path = bind_poll_path(&p, "ship", &intent, &["order".into()], &[json!("o-1")])
            .expect("bound");
        assert_eq!(path, "/jobs/prov-42");
    }

    #[test]
    fn a_response_field_wins_over_a_call_argument_of_the_same_name() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        intent.bind_submit_response(&json!({"id": "authoritative"}));
        let path = bind_poll_path(&p, "ship", &intent, &["id".into()], &[json!("caller-supplied")])
            .expect("bound");
        assert_eq!(
            path, "/jobs/authoritative",
            "the provider's own id must win over a caller-supplied one"
        );
    }

    #[test]
    fn observing_statuses_walks_the_declared_transition_table() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        let s = observe(&p, "ship", &mut intent, &json!({"status": "running"})).unwrap();
        assert_eq!(s, "submitted");
        assert!(!is_terminal(&p, &s));
        let s = observe(&p, "ship", &mut intent, &json!({"status": "done"})).unwrap();
        assert_eq!(s, "complete");
        assert!(is_terminal(&p, &s));
        assert_eq!(intent.status_history, vec!["running", "done"]);
        assert_eq!(intent.polls, 2);
    }

    #[test]
    fn a_status_outside_the_declared_universe_is_refused() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        let err = observe(&p, "ship", &mut intent, &json!({"status": "exploded"})).unwrap_err();
        assert_eq!(
            err,
            ProtocolError::UnknownStatus {
                operation: "ship".to_string(),
                status: "exploded".to_string()
            }
        );
        // The intent must not have advanced on a refused observation.
        assert_eq!(intent.state, "submitted");
        assert_eq!(intent.polls, 0);
    }

    #[test]
    fn a_poll_response_without_a_status_field_is_refused() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        let err = observe(&p, "ship", &mut intent, &json!({"state": "running"})).unwrap_err();
        assert_eq!(
            err,
            ProtocolError::MissingStatus {
                operation: "ship".to_string()
            }
        );
    }

    #[test]
    fn the_same_logical_call_derives_the_same_idempotency_key() {
        let at = |callsite, ordinal| InvocationIdentity { callsite, ordinal };
        let args = [json!("o-1"), json!(3)];
        let a = intent_idempotency_key("ups", "ship", &args, at(10, 0));
        let b = intent_idempotency_key("ups", "ship", &args, at(10, 0));
        assert_eq!(
            a, b,
            "a retry or a resume of the SAME invocation must reuse the same intent"
        );
        assert_ne!(
            a,
            intent_idempotency_key("ups", "ship", &[json!("o-2"), json!(3)], at(10, 0))
        );
        assert_ne!(
            a,
            intent_idempotency_key("ups", "cancel", &args, at(10, 0))
        );
    }

    /// The bug this identity exists to fix: "ship order-1" written twice,
    /// or written once in a loop that runs twice, is TWO pieces of work.
    /// Keying on arguments alone collapsed them into one intent, so the
    /// second call returned the first's result and its provider job was
    /// never created — a silent lost write, and the mirror image of the
    /// duplicate-submit risk.
    #[test]
    fn two_intentional_calls_with_identical_arguments_are_two_intents() {
        let args = [json!("o-1")];
        let at = |callsite, ordinal| InvocationIdentity { callsite, ordinal };

        // Same operation and arguments, written at two places in source.
        assert_ne!(
            intent_idempotency_key("ups", "ship", &args, at(10, 0)),
            intent_idempotency_key("ups", "ship", &args, at(42, 0)),
            "two distinct call sites must not share an intent"
        );

        // One call site executed twice — a loop, or a retry the program
        // wrote itself.
        assert_ne!(
            intent_idempotency_key("ups", "ship", &args, at(10, 0)),
            intent_idempotency_key("ups", "ship", &args, at(10, 1)),
            "two executions of one call site must not share an intent"
        );
    }

    #[test]
    fn cancelling_before_submit_is_exact_but_after_submit_only_detaches() {
        // The cancellation×irreversibility rule composed with durable
        // provider state: before submit there is no provider work, so
        // cancellation is exact. After submit a real job exists that
        // Corvid cannot un-create, so cancellation degrades to
        // detach-and-record rather than silently orphaning it.
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        assert_eq!(
            cancellation_disposition(&p, &intent),
            CancellationDisposition::Cancelled
        );

        intent.bind_submit_response(&json!({"id": "prov-1"}));
        assert_eq!(
            cancellation_disposition(&p, &intent),
            CancellationDisposition::Detached,
            "a submitted intent with no declared cancel endpoint must never be reported \
             as cleanly cancelled"
        );

        // Declaring a cancel endpoint is what makes real compensation
        // possible — the disposition follows the DECLARATION, not a
        // hopeful assumption about the provider.
        let with_cancel = ProviderProtocolDecl {
            cancel: Some(corvid_ast::ProtocolCancel {
                method: HttpMethod::Post,
                path: "/jobs/{id}/cancel".to_string(),
                span: Span::new(0, 0),
            }),
            ..protocol()
        };
        assert_eq!(
            cancellation_disposition(&with_cancel, &intent),
            CancellationDisposition::Compensate
        );
    }

    #[test]
    fn poll_cadence_honours_the_declared_interval() {
        let p = protocol();
        assert_eq!(poll_delay_ms(&p, 0), 5_000);
        assert_eq!(poll_delay_ms(&p, 10), 5_000);
        let adaptive = ProviderProtocolDecl {
            interval: ProtocolPollInterval::Adaptive,
            ..protocol()
        };
        assert!(poll_delay_ms(&adaptive, 0) < poll_delay_ms(&adaptive, 5));
    }

    /// The deadline bounds the PROTOCOL, not a process. Measuring from
    /// process start handed every restart a fresh full window, so a
    /// crash-looping deployment could poll a provider indefinitely while
    /// appearing to respect a 10-minute deadline — and it silently broke
    /// the budget bound, which is computed from one deadline window.
    #[test]
    fn the_deadline_is_measured_from_intent_creation_not_from_process_start() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        let created = 1_000_000_u64;
        intent.created_ms = Some(created);

        // Fresh intent: the whole window.
        assert_eq!(intent.deadline_remaining_ms(600, created), 600_000);

        // Ten minutes of wall clock later — across any number of
        // restarts — the window is spent, not renewed.
        assert_eq!(intent.deadline_remaining_ms(600, created + 600_000), 0);
        assert_eq!(
            intent.deadline_remaining_ms(600, created + 5_000_000),
            0,
            "a long-restarted protocol must not be handed more time"
        );

        // Halfway through, a restart resumes with the REMAINDER.
        assert_eq!(
            intent.deadline_remaining_ms(600, created + 240_000),
            360_000,
            "a resume continues the original window rather than restarting it"
        );
    }

    /// A checkpoint written before the timestamp existed cannot say when
    /// it started, so it keeps the old behaviour rather than being
    /// retroactively expired — the only honest reading available for it.
    #[test]
    fn an_intent_without_a_creation_time_falls_back_to_a_full_window() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        intent.created_ms = None;
        assert_eq!(intent.deadline_remaining_ms(600, 9_999_999), 600_000);
    }

    // --- Resume across a protocol change ---

    /// A canonical encoding from some earlier build. Its exact content
    /// does not matter — only that it differs from the running one.
    fn earlier_encoding() -> String {
        "encoding=corvid.protocol.canonical.v1\ndeadline=1".to_string()
    }

    /// Cosmetic edits must NOT strand in-flight work. The fingerprint is
    /// over the graph, so re-ordering statuses or transitions — which the
    /// checker already treats as the same protocol — is not a change.
    #[test]
    fn reordering_the_declaration_is_not_a_protocol_change() {
        let p = protocol();
        let mut reordered = protocol();
        reordered.statuses.reverse();
        reordered.terminal.reverse();
        reordered.states.reverse();
        for state in &mut reordered.states {
            state.transitions.reverse();
        }
        assert_eq!(
            p.fingerprint(),
            reordered.fingerprint(),
            "re-ordering a declaration must not read as drift"
        );
    }

    /// Changing the POLICY is not itself drift — otherwise an author who
    /// switched from `refuse` to `resume` would strand every intent by the
    /// act of deciding to be more permissive.
    #[test]
    fn changing_the_resume_posture_is_not_a_protocol_change() {
        let refuse = protocol();
        let resume = ProviderProtocolDecl {
            on_protocol_change: corvid_ast::ProtocolChangePolicy::Resume,
            ..protocol()
        };
        assert_eq!(refuse.fingerprint(), resume.fingerprint());
    }

    /// A real graph edit IS drift.
    #[test]
    fn editing_the_graph_changes_the_fingerprint() {
        let p = protocol();
        let mut edited = protocol();
        edited.deadline_secs = 900;
        assert_ne!(p.fingerprint(), edited.fingerprint());
    }

    /// Re-pointing a transition changes what the protocol MEANS, so it
    /// must register — this is the edit most likely to be made casually
    /// and most likely to strand an intent mid-graph.
    #[test]
    fn changing_a_transition_target_is_drift() {
        let p = protocol();
        let mut edited = protocol();
        edited.states[0].transitions[0].target = id("complete");
        assert_ne!(p.fingerprint(), edited.fingerprint());
        let changes = corvid_ast::protocol_canonical_differences(
            &p.canonical_encoding(),
            &edited.canonical_encoding(),
        );
        assert!(
            changes.iter().any(|c| c.starts_with("state submitted")),
            "the diff must name the state whose table changed; got {changes:?}"
        );
    }

    /// Deleting a state an intent could be sitting in is the case the
    /// resume floor exists for, and the diff must say so plainly.
    #[test]
    fn removing_a_state_is_drift_and_the_diff_names_it() {
        let p = protocol();
        let mut edited = protocol();
        edited.states.retain(|s| s.name.name != "complete");
        let changes = corvid_ast::protocol_canonical_differences(
            &p.canonical_encoding(),
            &edited.canonical_encoding(),
        );
        assert!(
            changes.iter().any(|c| c == "state complete: removed"),
            "a removed state must be named as removed; got {changes:?}"
        );
    }

    /// The encoding is what gets hashed, so it must be self-describing:
    /// a value read back from a durable checkpoint written by an older
    /// build has to be recognisable as "encoded differently" rather than
    /// silently mistaken for a changed protocol.
    #[test]
    fn the_canonical_encoding_names_its_version_and_the_fingerprint_names_its_algorithm() {
        let p = protocol();
        assert!(p
            .canonical_encoding()
            .starts_with("encoding=corvid.protocol.canonical.v1\n"));
        assert!(p.fingerprint().starts_with("sha256:"));
        assert_eq!(
            p.fingerprint().len(),
            "sha256:".len() + 64,
            "a sha256 fingerprint is the algorithm name plus 64 hex characters"
        );
    }

    /// Pinned so the encoding cannot drift silently between builds or
    /// machines. A deliberate change to the encoding must bump
    /// `PROTOCOL_CANONICAL_ENCODING` and this constant together; an
    /// accidental one fails here rather than in production, where it
    /// would read as "every in-flight intent's protocol changed".
    #[test]
    fn the_fingerprint_is_stable_across_builds_and_machines() {
        let expected = corvid_ast::ProviderProtocolDecl::fingerprint_of(
            &protocol().canonical_encoding(),
        );
        assert_eq!(protocol().fingerprint(), expected);
        // Recomputing from a fresh value must agree byte for byte: the
        // input is sorted, span-free and layout-free, and the hash is a
        // published algorithm — no DefaultHasher, address, or map order.
        assert_eq!(protocol().fingerprint(), protocol().fingerprint());
        assert_eq!(
            protocol().fingerprint(),
            "sha256:99e0259f877e102474789d81d69b58271a2550635b69c0c7af381faffa6da444",
            "the canonical encoding changed; bump PROTOCOL_CANONICAL_ENCODING deliberately"
        );
    }

    #[test]
    fn an_unchanged_protocol_resumes_normally() {
        let p = protocol();
        let intent = ProtocolIntentState::new(&p);
        assert_eq!(resume_verdict(&p, &intent), ResumeVerdict::Unchanged);
    }

    /// `refuse` is a refusal even though the new graph could technically
    /// host the recorded state: the author said do not resume across a
    /// change, and a provider job is already running.
    #[test]
    fn a_changed_protocol_refuses_when_the_declaration_says_refuse() {
        let p = protocol();
        let mut intent = ProtocolIntentState::new(&p);
        intent.submitted = true;
        intent.protocol_canonical = Some(earlier_encoding());
        assert!(matches!(
            resume_verdict(&p, &intent),
            ResumeVerdict::RefusedByPolicy { .. }
        ));
    }

    #[test]
    fn a_changed_protocol_resumes_when_declared_and_the_state_survives() {
        let p = ProviderProtocolDecl {
            on_protocol_change: corvid_ast::ProtocolChangePolicy::Resume,
            ..protocol()
        };
        let mut intent = ProtocolIntentState::new(&p);
        intent.submitted = true;
        intent.protocol_canonical = Some(earlier_encoding());
        assert!(matches!(
            resume_verdict(&p, &intent),
            ResumeVerdict::ResumedAcrossChange { .. }
        ));
    }

    /// The safety floor: `resume` does not mean "resume from anywhere". If
    /// the recorded state is gone from the new graph there is no sound
    /// point to continue from, so it refuses despite the policy.
    #[test]
    fn resume_still_refuses_when_the_recorded_state_no_longer_exists() {
        let p = ProviderProtocolDecl {
            on_protocol_change: corvid_ast::ProtocolChangePolicy::Resume,
            ..protocol()
        };
        let mut intent = ProtocolIntentState::new(&p);
        intent.submitted = true;
        intent.protocol_canonical = Some(earlier_encoding());
        intent.state = "a_state_that_was_deleted".to_string();
        assert!(matches!(
            resume_verdict(&p, &intent),
            ResumeVerdict::RefusedStateVanished { state, .. } if state == "a_state_that_was_deleted"
        ));
    }

    /// A checkpoint written before change detection existed cannot prove
    /// it matches, and "we cannot tell" is not "it is the same" — but only
    /// once there is real work to strand. A fresh, unsubmitted intent has
    /// nothing to migrate.
    #[test]
    fn an_unrecorded_protocol_is_treated_as_changed_once_it_has_work() {
        let p = protocol();
        let mut fresh = ProtocolIntentState::new(&p);
        fresh.protocol_canonical = None;
        assert_eq!(resume_verdict(&p, &fresh), ResumeVerdict::Unchanged);

        let mut inflight = fresh.clone();
        inflight.submitted = true;
        assert!(resume_verdict(&p, &inflight).is_refusal());
    }
}

