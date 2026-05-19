//! Annotation + delegation + batch subcommands: `corvid
//! approvals comment`, `delegate`, `batch`. These don't transition
//! the approval state directly (except `batch`, which is just a
//! loop over `approve`); they enrich the audit trail and let an
//! operator hand a pending approval off to a different reviewer.
//!
//! Per-id failures in `batch` are isolated rather than aborting
//! the whole call — the operator gets a clear "succeeded N,
//! failed M" summary with per-id reasons.

use anyhow::{anyhow, Result};
use corvid_runtime::approval_authorization::ApprovalActorContext;
use corvid_runtime::approval_queue::ApprovalQueueRuntime;
use std::path::PathBuf;

use super::{summarise, summarise_audit, ApprovalSummary, AuditEventSummary};

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-coverage
/// sentinel in `corvid-guarantees`. Names the registry id whose
/// runtime enforcement lives in `run_approvals_batch` below: the
/// cross-data-class drift refusal that catches the
/// `batch-approval-drift-across-data-classes` threat.
#[allow(dead_code)]
pub const GUARANTEE_ID_BATCH_REFUSES_CROSS_DATA_CLASS_DRIFT: &str =
    "approval.batch_refuses_cross_data_class_drift";

#[derive(Debug, Clone)]
pub struct ApprovalsCommentArgs {
    pub approvals_state: PathBuf,
    pub tenant_id: String,
    pub approval_id: String,
    pub actor_id: String,
    pub comment: String,
}

pub fn run_approvals_comment(args: ApprovalsCommentArgs) -> Result<AuditEventSummary> {
    let approvals = ApprovalQueueRuntime::open(&args.approvals_state)
        .map_err(|e| anyhow!("approvals runtime init failed: {e}"))?;
    let event = approvals
        .comment(
            &args.approval_id,
            &args.tenant_id,
            &args.actor_id,
            &args.comment,
        )
        .map_err(|e| anyhow!("comment: {e}"))?;
    Ok(summarise_audit(event))
}

#[derive(Debug, Clone)]
pub struct ApprovalsDelegateArgs {
    pub approvals_state: PathBuf,
    pub tenant_id: String,
    pub approval_id: String,
    pub actor_id: String,
    pub role: String,
    pub delegate_to: String,
    pub reason: Option<String>,
}

pub fn run_approvals_delegate(args: ApprovalsDelegateArgs) -> Result<ApprovalSummary> {
    let approvals = ApprovalQueueRuntime::open(&args.approvals_state)
        .map_err(|e| anyhow!("approvals runtime init failed: {e}"))?;
    let actor = ApprovalActorContext {
        actor_id: args.actor_id.clone(),
        tenant_id: args.tenant_id.clone(),
        role: args.role.clone(),
    };
    let record = approvals
        .delegate(
            &args.approval_id,
            &args.tenant_id,
            &actor,
            &args.delegate_to,
            args.reason.as_deref(),
        )
        .map_err(|e| anyhow!("delegate: {e}"))?;
    Ok(summarise(record))
}

#[derive(Debug, Clone)]
pub struct ApprovalsBatchArgs {
    pub approvals_state: PathBuf,
    pub tenant_id: String,
    pub actor_id: String,
    pub role: String,
    pub approval_ids: Vec<String>,
    pub reason: Option<String>,
    /// Pin the batch to a single `data_class`. Approvals whose
    /// data_class doesn't match are surfaced as individual
    /// failures rather than approved. When `None`, the batch
    /// refuses outright if the supplied ids span >1 data class —
    /// the adversarial-prevention default so an operator cannot
    /// silently approve `financial` and `pii` records in the same
    /// invocation.
    pub require_data_class: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalsBatchOutput {
    pub approved: Vec<ApprovalSummary>,
    pub failed: Vec<BatchFailure>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchFailure {
    pub approval_id: String,
    pub reason: String,
}

/// Approve a batch of approval ids in one operation. Per-approval
/// failures (wrong role, wrong tenant, already-resolved) are
/// reported individually rather than aborting the whole batch —
/// the operator gets a clear "succeeded N, failed M" summary.
///
/// Adversarial-prevention rule: a batch whose supplied ids span
/// multiple `data_class` values refuses outright unless the
/// operator pins `--require-data-class <CLASS>` — this catches
/// the "batch-approval-drift-across-data-classes" threat where
/// `financial` and `pii` records would otherwise resolve in the
/// same invocation under a single reviewer's role check.
pub fn run_approvals_batch(args: ApprovalsBatchArgs) -> Result<ApprovalsBatchOutput> {
    let approvals = ApprovalQueueRuntime::open(&args.approvals_state)
        .map_err(|e| anyhow!("approvals runtime init failed: {e}"))?;
    let actor = ApprovalActorContext {
        actor_id: args.actor_id.clone(),
        tenant_id: args.tenant_id.clone(),
        role: args.role.clone(),
    };

    // Resolve every id's data_class up front so the spanning
    // check + per-id mismatch check both have the same source of
    // truth. Ids that don't resolve are kept and surface their
    // own failure inside the approve loop.
    let mut id_data_classes: Vec<(String, Option<String>)> = Vec::new();
    for id in &args.approval_ids {
        let record = approvals
            .get(id)
            .map_err(|e| anyhow!("read approval `{id}` for data-class check: {e}"))?;
        let data_class = record.and_then(|r| {
            if r.tenant_id == args.tenant_id {
                Some(r.data_class)
            } else {
                None
            }
        });
        id_data_classes.push((id.clone(), data_class));
    }

    let observed: std::collections::BTreeSet<&str> = id_data_classes
        .iter()
        .filter_map(|(_, dc)| dc.as_deref())
        .collect();

    if args.require_data_class.is_none() && observed.len() > 1 {
        let mut classes: Vec<&str> = observed.into_iter().collect();
        classes.sort();
        return Err(anyhow!(
            "approvals batch refused: supplied ids span {} data classes ({}). \
             Re-run with `--require-data-class <CLASS>` to pin the batch to \
             one class.",
            classes.len(),
            classes.join(", "),
        ));
    }

    let mut approved = Vec::new();
    let mut failed = Vec::new();
    for (id, data_class) in &id_data_classes {
        if let (Some(required), Some(actual)) = (args.require_data_class.as_deref(), data_class.as_deref()) {
            if required != actual {
                failed.push(BatchFailure {
                    approval_id: id.clone(),
                    reason: format!(
                        "data_class `{actual}` does not match \
                         --require-data-class `{required}`"
                    ),
                });
                continue;
            }
        }
        match approvals.approve(id, &args.tenant_id, &actor, args.reason.as_deref()) {
            Ok(record) => approved.push(summarise(record)),
            Err(e) => failed.push(BatchFailure {
                approval_id: id.clone(),
                reason: format!("{e}"),
            }),
        }
    }
    Ok(ApprovalsBatchOutput { approved, failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals_cmd::queue::{run_approvals_inspect, ApprovalsInspectArgs};
    use corvid_runtime::approval_queue::{
        ApprovalContractRecord, ApprovalCreate, ApprovalQueueRuntime,
    };
    use tempfile::tempdir;

    fn temp_paths() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let approvals = dir.path().join("approvals.db");
        (dir, approvals)
    }

    fn seed_pending_approval(approvals_state: &PathBuf, id: &str, tenant: &str, role: &str) {
        seed_pending_approval_with_class(approvals_state, id, tenant, role, "financial");
    }

    fn seed_pending_approval_with_class(
        approvals_state: &PathBuf,
        id: &str,
        tenant: &str,
        role: &str,
        data_class: &str,
    ) {
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
            target_id: "ord_42".to_string(),
            tenant_id: tenant.to_string(),
            required_role: role.to_string(),
            max_cost_usd: 100.0,
            data_class: data_class.to_string(),
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

    /// Slice 39L: `corvid approvals comment` records an audit
    /// event without changing status.
    #[test]
    fn approvals_comment_writes_audit_without_status_change() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending_approval(&approvals_state, "ap-1", "tenant-1", "Admin");
        let event = run_approvals_comment(ApprovalsCommentArgs {
            approvals_state: approvals_state.clone(),
            tenant_id: "tenant-1".to_string(),
            approval_id: "ap-1".to_string(),
            actor_id: "actor-x".to_string(),
            comment: "needs more context".to_string(),
        })
        .expect("comment");
        assert_eq!(event.event_kind, "commented");
        let inspect = run_approvals_inspect(ApprovalsInspectArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            approval_id: "ap-1".to_string(),
        })
        .expect("inspect");
        assert_eq!(inspect.approval.status, "pending");
    }

    /// Slice 39L: `corvid approvals batch` approves multiple in
    /// one invocation; per-id failures are isolated.
    #[test]
    fn approvals_batch_approves_succeeded_isolates_failures() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending_approval(&approvals_state, "ap-1", "tenant-1", "Admin");
        seed_pending_approval(&approvals_state, "ap-2", "tenant-1", "Reviewer");
        let out = run_approvals_batch(ApprovalsBatchArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            actor_id: "actor-admin".to_string(),
            role: "Admin".to_string(),
            approval_ids: vec!["ap-1".to_string(), "ap-2".to_string(), "ap-missing".to_string()],
            reason: Some("batch approve".to_string()),
            require_data_class: None,
        })
        .expect("batch");
        assert_eq!(out.approved.len(), 1);
        assert_eq!(out.approved[0].id, "ap-1");
        assert_eq!(out.failed.len(), 2);
        let failed_ids: Vec<&str> = out.failed.iter().map(|f| f.approval_id.as_str()).collect();
        assert!(failed_ids.contains(&"ap-2"));
        assert!(failed_ids.contains(&"ap-missing"));
    }

    /// Slice 35V2-P39-L-LR: `--require-data-class` pins a batch
    /// to one `data_class`; mismatched ids surface as per-id
    /// failures while matching ids approve.
    #[test]
    fn approvals_batch_require_data_class_pins_to_one_class() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending_approval_with_class(
            &approvals_state,
            "ap-fin-1",
            "tenant-1",
            "Admin",
            "financial",
        );
        seed_pending_approval_with_class(
            &approvals_state,
            "ap-fin-2",
            "tenant-1",
            "Admin",
            "financial",
        );
        seed_pending_approval_with_class(
            &approvals_state,
            "ap-pii-1",
            "tenant-1",
            "Admin",
            "pii",
        );
        let out = run_approvals_batch(ApprovalsBatchArgs {
            approvals_state,
            tenant_id: "tenant-1".to_string(),
            actor_id: "actor-admin".to_string(),
            role: "Admin".to_string(),
            approval_ids: vec![
                "ap-fin-1".to_string(),
                "ap-fin-2".to_string(),
                "ap-pii-1".to_string(),
            ],
            reason: Some("batch approve financial only".to_string()),
            require_data_class: Some("financial".to_string()),
        })
        .expect("batch");
        let approved_ids: Vec<&str> = out.approved.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(approved_ids, vec!["ap-fin-1", "ap-fin-2"]);
        assert_eq!(out.failed.len(), 1);
        assert_eq!(out.failed[0].approval_id, "ap-pii-1");
        assert!(
            out.failed[0].reason.contains("data_class `pii`")
                && out.failed[0].reason.contains("--require-data-class")
        );
    }

    /// Slice 35V2-P39-L-LR (adversarial): a batch whose ids
    /// span >1 `data_class` without `--require-data-class`
    /// refuses outright. This catches the
    /// `batch-approval-drift-across-data-classes` threat.
    #[test]
    fn approvals_batch_refuses_cross_data_class_drift_without_pin() {
        let (_dir, approvals_state) = temp_paths();
        seed_pending_approval_with_class(
            &approvals_state,
            "ap-fin",
            "tenant-1",
            "Admin",
            "financial",
        );
        seed_pending_approval_with_class(
            &approvals_state,
            "ap-pii",
            "tenant-1",
            "Admin",
            "pii",
        );
        let err = run_approvals_batch(ApprovalsBatchArgs {
            approvals_state: approvals_state.clone(),
            tenant_id: "tenant-1".to_string(),
            actor_id: "actor-admin".to_string(),
            role: "Admin".to_string(),
            approval_ids: vec!["ap-fin".to_string(), "ap-pii".to_string()],
            reason: None,
            require_data_class: None,
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("approvals batch refused"));
        assert!(err.contains("financial"));
        assert!(err.contains("pii"));

        // Confirm neither approval was resolved as a side effect.
        let approvals = ApprovalQueueRuntime::open(&approvals_state).unwrap();
        for id in ["ap-fin", "ap-pii"] {
            let record = approvals.get(id).unwrap().unwrap();
            assert_eq!(record.status, "pending");
        }
    }
}
