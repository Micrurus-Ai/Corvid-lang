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
//! **Identity and policy are both explicit.** The verified route actor
//! supplies `requested_by` and `tenant_id`; the compiled route supplies
//! reviewer role, risk, data class, expiry, cost ceiling, and
//! reversibility. There is no process-global tenant, anonymous
//! requester, fabricated reviewer role, or serve-owned policy default.
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

/// Verified identity attached to approvals created during one served
/// request. Both values are mandatory, so the queue cannot substitute
/// a process-global tenant or anonymous requester.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequester {
    pub actor_id: String,
    pub tenant_id: String,
}

/// Compiled policy of the route currently executing. All fields come
/// from its mandatory `@approval(...)` declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalQueuePolicy {
    pub route: String,
    pub required_role: String,
    pub risk_level: String,
    pub data_class: String,
    pub expires_in_ms: u64,
    pub max_cost_usd: f64,
    pub irreversible: bool,
}
/// Approver that turns every `approve` boundary into a pending queue
/// entry and surfaces the queued state as a `RuntimeError::
/// ApprovalQueued`. The wrapped `ApprovalQueueRuntime` is the same
/// store the rest of the runtime uses, so admin endpoints below can
/// list / get / transition queue entries through its API surface.
pub struct QueueApprover {
    queue: Arc<ApprovalQueueRuntime>,
    requester: ApprovalRequester,
    policy: ApprovalQueuePolicy,
    /// Per-process monotonic counter, combined with the nanosecond
    /// wall clock to produce an approval id that's unique across
    /// every call in this serve process (a wall-clock collision
    /// inside the same nanosecond is bounded above by `AtomicU64`'s
    /// monotonic step).
    next: AtomicU64,
}

impl QueueApprover {
    pub fn new(
        queue: Arc<ApprovalQueueRuntime>,
        requester: ApprovalRequester,
        policy: ApprovalQueuePolicy,
    ) -> Self {
        Self {
            queue,
            requester,
            policy,
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
                tenant_id: self.requester.tenant_id.clone(),
                requester_actor_id: self.requester.actor_id.clone(),
                contract: ApprovalContractRecord {
                    id: format!("serve:{label}"),
                    version: "v1".to_string(),
                    action: label.clone(),
                    target_kind: "http_route".to_string(),
                    target_id: self.policy.route.clone(),
                    tenant_id: self.requester.tenant_id.clone(),
                    required_role: self.policy.required_role.clone(),
                    max_cost_usd: self.policy.max_cost_usd,
                    data_class: self.policy.data_class.clone(),
                    irreversible: self.policy.irreversible,
                    expires_ms: now_ms.saturating_add(self.policy.expires_in_ms),
                    replay_key: format!("serve-replay-{approval_id}"),
                    created_ms: 0,
                },
                risk_level: self.policy.risk_level.clone(),
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

    fn policy() -> ApprovalQueuePolicy {
        ApprovalQueuePolicy {
            route: "POST /messages".into(),
            required_role: "reviewer".into(),
            risk_level: "high".into(),
            data_class: "customer_message".into(),
            expires_in_ms: 60_000,
            max_cost_usd: 2.5,
            irreversible: true,
        }
    }

    #[tokio::test]
    async fn queue_approver_creates_pending_entry_and_returns_approval_queued() {
        let queue = fresh_queue();
        let approver = QueueApprover::new(
            queue.clone(),
            ApprovalRequester {
                actor_id: "requester-7".into(),
                tenant_id: "tenant-blue".into(),
            },
            policy(),
        );

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
                assert_eq!(record.tenant_id, "tenant-blue");
                assert_eq!(record.requester_actor_id, "requester-7");
                assert_eq!(record.action, "SendExecutiveFollowUp");
                assert_eq!(record.status, "pending");
                assert_eq!(record.required_role, "reviewer");
                assert_eq!(record.risk_level, "high");
                assert_eq!(record.data_class, "customer_message");
                assert_eq!(record.max_cost_usd, 2.5);
                assert!(record.irreversible);
                assert_eq!(record.target_id, "POST /messages");
                let lifetime_ms = record.expires_ms.saturating_sub(record.created_ms);
                assert!(
                    (59_900..=60_000).contains(&lifetime_ms),
                    "the compiled 60s policy must survive queue insertion latency: {lifetime_ms}ms"
                );
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
        let approver = QueueApprover::new(
            queue.clone(),
            ApprovalRequester {
                actor_id: "requester-7".into(),
                tenant_id: "tenant-blue".into(),
            },
            policy(),
        );

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
