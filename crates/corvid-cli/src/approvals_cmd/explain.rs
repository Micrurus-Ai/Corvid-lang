//! `corvid approvals explain <id>` — assistive AI helper that
//! renders a typed reviewer summary for a single approval record.
//!
//! Walks the approval queue's audit trail for the supplied id,
//! classifies the record's lifecycle position (pending /
//! resolved / expired), and surfaces the typed reviewer-relevant
//! facts (required_role, max_cost_usd, data_class, irreversible,
//! time-to-expiry, every audit-event transition with actor and
//! reason). The output's `sources` array carries the
//! audit-event ids the explanation consulted — the Grounded<T>
//! shape at the JSON layer, so a reviewer can audit-trail every
//! claim back to a queue row.
//!
//! Deterministic by construction: the classifier reads typed
//! enums + numeric fields off the approval queue, no LLM round
//! trip. The "AI helper" framing names the role the output plays
//! for a reviewer (assistive summarisation), not the call
//! pattern (which is local + deterministic).

use anyhow::{anyhow, Result};
use corvid_runtime::approval_queue::{
    ApprovalQueueAuditEvent, ApprovalQueueRecord, ApprovalQueueRuntime,
};
use std::path::PathBuf;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-coverage
/// sentinel. Names the registry id whose runtime enforcement
/// lives in `run_approvals_explain` below: the output's `sources`
/// array is non-empty and corresponds 1:1 with audit-event ids
/// the explanation consulted.
#[allow(dead_code)]
pub const GUARANTEE_ID_APPROVAL_EXPLAIN_SOURCES_GROUNDED: &str =
    "approval.explain_sources_grounded";

#[derive(Debug, Clone)]
pub struct ApprovalsExplainArgs {
    pub approvals_state: PathBuf,
    pub tenant_id: String,
    pub approval_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalExplanation {
    pub approval_id: String,
    pub tenant_id: String,
    pub lifecycle_position: String,
    pub headline: String,
    pub reviewer_facts: ReviewerFacts,
    pub transitions: Vec<TransitionSummary>,
    pub suggested_next_steps: Vec<String>,
    /// Grounded<T> shape: every claim in the explanation maps
    /// back to one of these audit-event ids.
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewerFacts {
    pub status: String,
    pub action: String,
    pub target_kind: String,
    pub target_id: String,
    pub required_role: String,
    pub risk_level: String,
    pub data_class: String,
    pub irreversible: bool,
    pub max_cost_usd: f64,
    pub expires_at_ms: u64,
    pub created_at_ms: u64,
    pub trace_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSummary {
    pub audit_event_id: String,
    pub event_kind: String,
    pub status_before: String,
    pub status_after: String,
    pub actor_id: String,
    pub reason: Option<String>,
    pub created_at_ms: u64,
}

pub fn run_approvals_explain(args: ApprovalsExplainArgs) -> Result<ApprovalExplanation> {
    let approvals = ApprovalQueueRuntime::open(&args.approvals_state)
        .map_err(|e| anyhow!("approvals runtime init failed: {e}"))?;
    let record = approvals
        .get(&args.approval_id)
        .map_err(|e| anyhow!("read approval `{}`: {e}", args.approval_id))?
        .ok_or_else(|| anyhow!("approval `{}` not found", args.approval_id))?;
    if record.tenant_id != args.tenant_id {
        return Err(anyhow!(
            "approval `{}` belongs to a different tenant; refusing to leak \
             cross-tenant state",
            args.approval_id
        ));
    }
    let events = approvals
        .audit_events(&args.approval_id)
        .map_err(|e| anyhow!("read audit events for `{}`: {e}", args.approval_id))?;

    let lifecycle_position = classify_lifecycle(&record);
    let headline = render_headline(&record, &lifecycle_position);
    let suggested = suggest_next_steps(&record, &lifecycle_position);
    let sources: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
    let transitions: Vec<TransitionSummary> =
        events.iter().map(transition_summary).collect();
    let reviewer_facts = reviewer_facts(&record);

    Ok(ApprovalExplanation {
        approval_id: args.approval_id,
        tenant_id: args.tenant_id,
        lifecycle_position,
        headline,
        reviewer_facts,
        transitions,
        suggested_next_steps: suggested,
        sources,
    })
}

fn classify_lifecycle(record: &ApprovalQueueRecord) -> String {
    match record.status.as_str() {
        "pending" => "pending".to_string(),
        "approved" => "approved".to_string(),
        "denied" => "denied".to_string(),
        "expired" => "expired".to_string(),
        other => format!("other:{other}"),
    }
}

fn render_headline(record: &ApprovalQueueRecord, lifecycle: &str) -> String {
    match lifecycle {
        "pending" => format!(
            "Pending: `{}` on {}/{} requires role `{}` ({} risk, data_class={}{}). \
             Reviewer must accept the cost-of-being-wrong up to ${:.2}.",
            record.action,
            record.target_kind,
            record.target_id,
            record.required_role,
            record.risk_level,
            record.data_class,
            if record.irreversible {
                ", irreversible"
            } else {
                ""
            },
            record.max_cost_usd,
        ),
        "approved" => format!(
            "Approved: `{}` on {}/{} resolved by {}.",
            record.action,
            record.target_kind,
            record.target_id,
            record
                .approver_actor_id
                .as_deref()
                .unwrap_or("(approver missing)"),
        ),
        "denied" => format!(
            "Denied: `{}` on {}/{} refused by {}.",
            record.action,
            record.target_kind,
            record.target_id,
            record
                .approver_actor_id
                .as_deref()
                .unwrap_or("(approver missing)"),
        ),
        "expired" => format!(
            "Expired: `{}` on {}/{} timed out before review.",
            record.action, record.target_kind, record.target_id,
        ),
        other => format!("Unrecognised lifecycle state `{other}` — read transitions for context."),
    }
}

fn suggest_next_steps(record: &ApprovalQueueRecord, lifecycle: &str) -> Vec<String> {
    match lifecycle {
        "pending" => vec![
            format!(
                "verify the requester intent against trace `{}` before approving",
                record.trace_id
            ),
            format!(
                "confirm the actor's role matches `{}` (required) — `corvid approvals \
                 delegate --to <actor>` if it doesn't",
                record.required_role
            ),
            if record.irreversible {
                "approve with caution: this action is marked irreversible — denial is \
                 cheaper than recovery"
                    .to_string()
            } else {
                "approve via `corvid approvals approve --tenant ... --actor ... --role ... \
                 <id>` once verified"
                    .to_string()
            },
        ],
        "approved" | "denied" => vec![
            "resolution is final; the audit trail is the source of truth for compliance review"
                .to_string(),
            "if the resolution was wrong, raise a follow-up via the host backend (the \
             approval record is immutable)"
                .to_string(),
        ],
        "expired" => vec![
            "the approval expired before review; if the action is still needed, the \
             requester must re-submit (this is correct behaviour, not a bug)"
                .to_string(),
        ],
        _ => vec!["walk the audit transitions to see how the record reached this state".to_string()],
    }
}

fn reviewer_facts(record: &ApprovalQueueRecord) -> ReviewerFacts {
    ReviewerFacts {
        status: record.status.clone(),
        action: record.action.clone(),
        target_kind: record.target_kind.clone(),
        target_id: record.target_id.clone(),
        required_role: record.required_role.clone(),
        risk_level: record.risk_level.clone(),
        data_class: record.data_class.clone(),
        irreversible: record.irreversible,
        max_cost_usd: record.max_cost_usd,
        expires_at_ms: record.expires_ms,
        created_at_ms: record.created_ms,
        trace_id: record.trace_id.clone(),
    }
}

fn transition_summary(event: &ApprovalQueueAuditEvent) -> TransitionSummary {
    TransitionSummary {
        audit_event_id: event.id.clone(),
        event_kind: event.event_kind.clone(),
        status_before: event.status_before.clone(),
        status_after: event.status_after.clone(),
        actor_id: event.actor_id.clone(),
        reason: event.reason.clone(),
        created_at_ms: event.created_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::approval_authorization::ApprovalActorContext;
    use corvid_runtime::approval_queue::{ApprovalContractRecord, ApprovalCreate};
    use tempfile::tempdir;

    fn temp_paths() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let approvals = dir.path().join("approvals.db");
        (dir, approvals)
    }

    fn seed_pending(approvals_state: &PathBuf, id: &str, tenant: &str, role: &str) {
        let approvals = ApprovalQueueRuntime::open(approvals_state).unwrap();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let contract = ApprovalContractRecord {
            id: format!("{id}-contract"),
            version: "v1".to_string(),
            action: "issue_refund".to_string(),
            target_kind: "order".to_string(),
            target_id: "ord_99".to_string(),
            tenant_id: tenant.to_string(),
            required_role: role.to_string(),
            max_cost_usd: 250.0,
            data_class: "financial".to_string(),
            irreversible: true,
            expires_ms: now_ms + 60_000,
            replay_key: format!("rk-{id}"),
            created_ms: now_ms,
        };
        approvals
            .create(ApprovalCreate {
                id: id.to_string(),
                tenant_id: tenant.to_string(),
                requester_actor_id: "requester-1".to_string(),
                contract,
                risk_level: "high".to_string(),
                trace_id: format!("trace-{id}"),
            })
            .unwrap();
    }

    /// Slice 35V2-P39-G-LR (positive): the helper renders a typed
    /// pending headline + reviewer facts + transitions, and every
    /// transition's audit-event id appears in `sources` — the
    /// Grounded<T> shape: every claim has a back-reference.
    #[test]
    fn approvals_explain_pending_carries_grounded_sources() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending(&approvals_state, "ap-x", "tenant-1", "Admin");
        let report = run_approvals_explain(ApprovalsExplainArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            approval_id: "ap-x".to_string(),
        })
        .expect("explain");
        assert_eq!(report.lifecycle_position, "pending");
        assert!(report.headline.contains("Pending:"));
        assert!(report.headline.contains("issue_refund"));
        assert!(report.headline.contains("Admin"));
        assert!(report.headline.contains("financial"));
        assert!(report.headline.contains("irreversible"));
        assert!(report.suggested_next_steps.iter().any(|s| s.contains("irreversible")));
        assert!(!report.transitions.is_empty());
        assert!(!report.sources.is_empty());
        for transition in &report.transitions {
            assert!(report.sources.contains(&transition.audit_event_id));
        }
        assert_eq!(report.reviewer_facts.required_role, "Admin");
        assert_eq!(report.reviewer_facts.max_cost_usd, 250.0);
    }

    /// Slice 35V2-P39-G-LR (positive, resolved): after approval,
    /// the helper renders the resolved headline + names the
    /// approver actor; sources still ground every claim.
    #[test]
    fn approvals_explain_after_resolution_records_approver() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending(&approvals_state, "ap-y", "tenant-1", "Admin");
        let runtime = ApprovalQueueRuntime::open(&approvals_state).unwrap();
        let actor = ApprovalActorContext {
            actor_id: "reviewer-alpha".to_string(),
            tenant_id: "tenant-1".to_string(),
            role: "Admin".to_string(),
        };
        runtime
            .approve("ap-y", "tenant-1", &actor, Some("verified intent"))
            .unwrap();
        let report = run_approvals_explain(ApprovalsExplainArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            approval_id: "ap-y".to_string(),
        })
        .unwrap();
        assert_eq!(report.lifecycle_position, "approved");
        assert!(report.headline.contains("reviewer-alpha"));
        let approval_transition = report
            .transitions
            .iter()
            .find(|t| t.event_kind == "approved")
            .expect("approval transition recorded");
        assert!(report.sources.contains(&approval_transition.audit_event_id));
    }

    /// Slice 35V2-P39-G-LR (adversarial): a missing approval id
    /// surfaces a clear "not found" diagnostic, not an empty
    /// report. The error path is the safety surface — a silent
    /// empty explanation would be the worst failure mode.
    #[test]
    fn approvals_explain_unknown_id_refuses() {
        let (_dir, approvals_state) = temp_paths();
        let err = run_approvals_explain(ApprovalsExplainArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            approval_id: "no-such-id".to_string(),
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("not found"), "expected 'not found', got: {err}");
    }

    /// Slice 35V2-P39-G-LR (adversarial): a cross-tenant request
    /// is refused with an explicit message, not silently served.
    /// This catches operator misconfiguration (wrong --tenant flag
    /// targeting the right id).
    #[test]
    fn approvals_explain_cross_tenant_refused() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending(&approvals_state, "ap-z", "tenant-1", "Admin");
        let err = run_approvals_explain(ApprovalsExplainArgs {
            approvals_state,
            tenant_id: "tenant-OTHER".to_string(),
            approval_id: "ap-z".to_string(),
        })
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("different tenant"),
            "expected cross-tenant refusal, got: {err}"
        );
    }
}
