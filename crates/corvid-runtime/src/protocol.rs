//! Verified provider protocol execution core (slice 52h-2).
//!
//! Slice 52h-1 made a long-running provider protocol *language data*: an
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
//! 52h-1 grammar needs no amendment):
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
//!   cross the typed boundary (52i quarantines it).

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
    /// Number of completed polls (drives adaptive cadence in 52h-3).
    pub polls: u64,
    /// The most recent decoded poll payload. Persisted so a run that
    /// resumes an ALREADY-terminal intent returns the provider's real
    /// terminal observation instead of re-deriving one — the resumed
    /// call must be indistinguishable from the original.
    #[serde(default)]
    pub last_response: Option<serde_json::Value>,
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

/// What cancelling an intent may actually do (slice 52h-3).
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

/// The poll cadence in milliseconds for the next observation. Adaptive
/// cadence is governed in 52h-3; until then it backs off linearly from
/// the fixed floor so a long protocol does not hammer the provider.
pub fn poll_delay_ms(protocol: &ProviderProtocolDecl, polls: u64) -> u64 {
    match protocol.interval {
        ProtocolPollInterval::FixedSeconds(secs) => secs.saturating_mul(1000).max(1),
        ProtocolPollInterval::Adaptive => {
            let base = 1_000_u64;
            base.saturating_mul(polls.saturating_add(1).min(30))
        }
    }
}

/// The durable idempotency key for an intent. Derived from the connector,
/// operation, and the call's arguments, so the SAME logical call maps to
/// the SAME durable row — a retry or a restart re-finds the existing
/// intent instead of submitting a second provider job.
pub fn intent_idempotency_key(
    connector: &str,
    operation: &str,
    args: &[serde_json::Value],
) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
    let mut hasher = Sha256::new();
    hasher.update(connector.as_bytes());
    hasher.update(b"\0");
    hasher.update(operation.as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical.as_bytes());
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
        let a = intent_idempotency_key("ups", "ship", &[json!("o-1"), json!(3)]);
        let b = intent_idempotency_key("ups", "ship", &[json!("o-1"), json!(3)]);
        let different_args = intent_idempotency_key("ups", "ship", &[json!("o-2"), json!(3)]);
        let different_op = intent_idempotency_key("ups", "cancel", &[json!("o-1"), json!(3)]);
        assert_eq!(a, b, "a retry of the same call must reuse the same intent");
        assert_ne!(a, different_args);
        assert_ne!(a, different_op);
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
}
