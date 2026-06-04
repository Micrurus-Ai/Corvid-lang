//! `QueueApprover` — the approver `corvid serve` constructs when an
//! approval-gated route is hit and no interactive approver is available.
//!
//! `corvid serve` (slice `35V2-P42-E0-serve-2`) constructs an
//! interpreter `Runtime` for each request, but the runtime expects an
//! `Approver` to decide every `approve` boundary synchronously.
//! Slice `35V2-P42-E0-serve-4` shipped a `ProgrammaticApprover::
//! always_no()` so dangerous writes deny-by-default and the route
//! answers `403 approval_required`. That stance was safe but
//! developer-unusable — every approval-gated request died at the gate.
//!
//! This module implements the **async-approval** model the slice spec
//! at `ROADMAP.md` line ~2848 calls for: when an `approve` boundary
//! fires, `QueueApprover` creates a pending entry in the existing
//! `ApprovalQueueRuntime` flow (the same store the runtime / catalog
//! ABI / audit-log surfaces already use) and surfaces the queued state
//! to the HTTP layer via the new `RuntimeError::ApprovalQueued`
//! variant. `serve_cmd::finish` then answers `202 Accepted` with the
//! approval id so the client can poll `GET /__approvals/<id>` for the
//! eventual decision.
//!
//! **Why `RuntimeError::ApprovalQueued` rather than a third
//! `ApprovalDecision` variant.** The `Approver::approve` contract
//! returns `Result<ApprovalDecision, RuntimeError>` where the `Ok`
//! arm covers `Approve` / `Deny` — both *synchronous* decisions the
//! agent uses to either proceed or fail-fast. A queued approval is
//! neither: the agent must NOT proceed (no decision yet), but it
//! also has not been denied. Modelling the suspend as an error
//! variant keeps the trait shape stable for every existing impl
//! (`StdinApprover`, `ProgrammaticApprover`, the future browser
//! dialog approver) — none of those ever produce `ApprovalQueued`,
//! so they need no change. The runtime's existing fast-fail-on-error
//! plumbing carries the queued state up to the request boundary
//! where `serve_cmd::finish` differentiates it from
//! `ApprovalDenied`.
//!
//! **Why a synthesized default contract rather than per-route
//! declarations.** The `ApprovalCreate` envelope the existing
//! `ApprovalQueueRuntime::create()` expects carries per-tenant
//! contract metadata (`required_role`, `data_class`, `max_cost_usd`,
//! …) that today's `server` block doesn't declare. Wiring the
//! source-level contract through into the queue is a separate
//! slice (out of scope here; see follow-up `serve-6`). For this
//! slice's MVP, every queued approval lives under a single
//! `serve-default` tenant with a synthesized contract derived from
//! the `ApprovalRequest::label` — enough to exercise the queue
//! create / get / list path end-to-end, while keeping the source
//! surface unchanged. The follow-up slice replaces the synthesized
//! contract with a per-route declared one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use corvid_runtime::approval_queue::{
    ApprovalContractRecord, ApprovalCreate, ApprovalQueueRuntime,
};
use corvid_runtime::approvals::{ApprovalDecision, ApprovalRequest, Approver};
use corvid_runtime::RuntimeError;
use futures::future::BoxFuture;
use futures::FutureExt;

/// Tenant id every `corvid serve` approval is created under. Multi-
/// tenant routing is a follow-up; for the MVP every served binary
/// is single-tenant.
pub const SERVE_DEFAULT_TENANT: &str = "serve-default";
/// Requester actor id for every `corvid serve` request — the HTTP
/// layer is anonymous at the slice MVP boundary. Per-request actor
/// inference (from `Authorization` header, mTLS, session cookie) is
/// a follow-up.
pub const SERVE_DEFAULT_REQUESTER: &str = "serve-anonymous";

/// Approver that turns every `approve` boundary into a pending queue
/// entry and surfaces the queued state as a `RuntimeError::
/// ApprovalQueued`. The wrapped `ApprovalQueueRuntime` is the same
/// store the rest of the runtime uses, so admin endpoints below can
/// list / get / transition queue entries through its API surface.
pub struct QueueApprover {
    queue: Arc<ApprovalQueueRuntime>,
    /// Per-process monotonic counter, combined with the nanosecond
    /// wall clock to produce an approval id that's unique across
    /// every call in this serve process (a wall-clock collision
    /// inside the same nanosecond is bounded above by `AtomicU64`'s
    /// monotonic step).
    next: AtomicU64,
}

impl QueueApprover {
    pub fn new(queue: Arc<ApprovalQueueRuntime>) -> Self {
        Self {
            queue,
            next: AtomicU64::new(0),
        }
    }

    fn mint_approval_id(&self) -> String {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let seq = self.next.fetch_add(1, Ordering::Relaxed);
        format!("serve-{now_ns}-{seq}")
    }
}

impl Approver for QueueApprover {
    fn approve<'a>(
        &'a self,
        req: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, RuntimeError>> {
        async move {
            let approval_id = self.mint_approval_id();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let label = &req.label;
            let create = ApprovalCreate {
                id: approval_id.clone(),
                tenant_id: SERVE_DEFAULT_TENANT.to_string(),
                requester_actor_id: SERVE_DEFAULT_REQUESTER.to_string(),
                contract: ApprovalContractRecord {
                    id: format!("serve:{label}"),
                    version: "v1".to_string(),
                    action: label.clone(),
                    target_kind: "agent_call".to_string(),
                    target_id: format!("{label}-call"),
                    tenant_id: SERVE_DEFAULT_TENANT.to_string(),
                    required_role: "operator".to_string(),
                    max_cost_usd: 0.0,
                    data_class: "private".to_string(),
                    irreversible: true,
                    expires_ms: now_ms.saturating_add(24 * 60 * 60 * 1000),
                    replay_key: format!("serve-replay-{approval_id}"),
                    created_ms: 0,
                },
                risk_level: "medium".to_string(),
                trace_id: format!("serve-trace-{approval_id}"),
            };
            match self.queue.create(create) {
                Ok(_record) => Err(RuntimeError::ApprovalQueued { approval_id }),
                Err(e) => Err(RuntimeError::ApprovalFailed(format!(
                    "approval queue create failed: {e}"
                ))),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_queue() -> Arc<ApprovalQueueRuntime> {
        Arc::new(
            ApprovalQueueRuntime::open_in_memory().expect("open in-memory approval queue"),
        )
    }

    #[tokio::test]
    async fn queue_approver_creates_pending_entry_and_returns_approval_queued() {
        let queue = fresh_queue();
        let approver = QueueApprover::new(queue.clone());

        let req = ApprovalRequest {
            label: "SendExecutiveFollowUp".to_string(),
            args: vec![],
        };
        let outcome = approver.approve(&req).await;

        match outcome {
            Err(RuntimeError::ApprovalQueued { approval_id }) => {
                let record = queue
                    .get(&approval_id)
                    .expect("queue get must not error")
                    .expect("queue must contain the freshly created approval");
                assert_eq!(record.id, approval_id);
                assert_eq!(record.tenant_id, SERVE_DEFAULT_TENANT);
                assert_eq!(record.requester_actor_id, SERVE_DEFAULT_REQUESTER);
                assert_eq!(record.action, "SendExecutiveFollowUp");
                assert_eq!(record.status, "pending");
            }
            other => panic!(
                "expected RuntimeError::ApprovalQueued, got {other:?} — the queue approver \
                 must NEVER return Approve / Deny synchronously (that's the StdinApprover / \
                 ProgrammaticApprover contract); a queued approval has no decision yet."
            ),
        }
    }

    #[tokio::test]
    async fn queue_approver_mints_distinct_ids_under_burst() {
        // Same-process burst — the AtomicU64 sequence step guarantees
        // distinct ids even if the nanosecond wall clock collides
        // (which on Windows it can — system clock granularity is
        // ~15ms historically).
        let queue = fresh_queue();
        let approver = QueueApprover::new(queue.clone());

        let mut ids = std::collections::HashSet::new();
        for _ in 0..50 {
            let req = ApprovalRequest {
                label: "Burst".to_string(),
                args: vec![],
            };
            match approver.approve(&req).await {
                Err(RuntimeError::ApprovalQueued { approval_id }) => {
                    assert!(
                        ids.insert(approval_id.clone()),
                        "duplicate approval id `{approval_id}` — the per-process \
                         AtomicU64 step is meant to make collisions impossible"
                    );
                }
                other => panic!("expected ApprovalQueued, got {other:?}"),
            }
        }
        assert_eq!(ids.len(), 50);
    }
}
