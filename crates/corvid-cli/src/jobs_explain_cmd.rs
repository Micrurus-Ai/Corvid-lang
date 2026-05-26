//! `corvid jobs explain <job_id>` — assistive AI helper that
//! renders a typed root-cause summary for a single durable job.
//!
//! Walks the durable queue's typed record + audit-event trail
//! for the supplied id, classifies the operational position
//! (running / retrying / dead-lettered / awaiting approval /
//! succeeded / cancelled), and surfaces the typed
//! operator-relevant facts (task, attempts vs max_retries,
//! lease owner / expiry, failure kind + fingerprint, approval
//! linkage, loop usage). The output's `sources` array carries
//! the audit-event ids the explanation consulted — the
//! Grounded<T> shape at the JSON layer.
//!
//! Deterministic by construction: typed classifier over typed
//! records, no LLM round trip. Same pattern as Phase 40's
//! `corvid observe explain` and Phase 39's
//! `corvid approvals explain`.

use anyhow::{anyhow, Result};
use corvid_runtime::queue::{DurableQueueRuntime, JobAuditEvent, JobLoopUsage, QueueJob};
use std::path::Path;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-coverage
/// sentinel. Names the registry id whose runtime enforcement
/// lives in `run_jobs_explain` below: the output's `sources`
/// array is grounded in audit-event ids the explanation
/// consulted.
#[allow(dead_code)]
pub const GUARANTEE_ID_JOBS_EXPLAIN_SOURCES_GROUNDED: &str = "jobs.explain_sources_grounded";

#[derive(Debug, Clone, PartialEq)]
pub struct JobExplanation {
    pub job_id: String,
    pub operational_position: String,
    pub headline: String,
    pub operator_facts: OperatorFacts,
    pub transitions: Vec<JobTransitionSummary>,
    pub loop_usage: Option<LoopUsageSummary>,
    pub suggested_next_steps: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperatorFacts {
    pub task: String,
    pub status: String,
    pub attempts: u64,
    pub max_retries: u64,
    pub budget_usd: f64,
    pub effect_summary: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_expires_ms: Option<u64>,
    pub next_run_ms: Option<u64>,
    pub failure_kind: Option<String>,
    pub failure_fingerprint: Option<String>,
    pub approval_id: Option<String>,
    pub approval_expires_ms: Option<u64>,
    pub approval_reason: Option<String>,
    pub replay_key: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobTransitionSummary {
    pub audit_event_id: String,
    pub event_kind: String,
    pub status_before: String,
    pub status_after: String,
    pub actor: String,
    pub reason: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoopUsageSummary {
    pub steps: u64,
    pub wall_ms: u64,
    pub spend_usd: f64,
    pub tool_calls: u64,
}

pub fn run_jobs_explain(state: &Path, job_id: &str) -> Result<JobExplanation> {
    let queue = DurableQueueRuntime::open(state)
        .map_err(|e| anyhow!("open durable queue at `{}`: {e}", state.display()))?;
    let job = queue
        .get(job_id)
        .map_err(|e| anyhow!("read job `{job_id}`: {e}"))?
        .ok_or_else(|| anyhow!("job `{job_id}` not found"))?;
    let events = queue
        .job_audit_events(job_id)
        .map_err(|e| anyhow!("read audit events for `{job_id}`: {e}"))?;
    let usage = queue
        .loop_usage(job_id)
        .map_err(|e| anyhow!("read loop usage for `{job_id}`: {e}"))?;

    let operational_position = classify_position(&job);
    let headline = render_headline(&job, &operational_position);
    let suggested = suggest_next_steps(&job, &operational_position);
    let operator_facts = operator_facts(&job);
    let transitions: Vec<JobTransitionSummary> = events.iter().map(transition_summary).collect();
    let sources: Vec<String> = events.iter().map(|e| e.id.clone()).collect();
    let loop_usage = usage.map(loop_usage_summary);

    Ok(JobExplanation {
        job_id: job_id.to_string(),
        operational_position,
        headline,
        operator_facts,
        transitions,
        loop_usage,
        suggested_next_steps: suggested,
        sources,
    })
}

fn classify_position(job: &QueueJob) -> String {
    job.status.as_str().to_string()
}

fn render_headline(job: &QueueJob, position: &str) -> String {
    match position {
        "pending" => format!(
            "Pending: `{}` is queued (attempt {}/{}); waiting for a worker to lease it.",
            job.task,
            job.attempts.saturating_add(1),
            job.max_retries.max(1),
        ),
        "leased" | "running" => format!(
            "Leased: `{}` is owned by `{}`.",
            job.task,
            job.lease_owner.as_deref().unwrap_or("(no lease owner)"),
        ),
        "retry_wait" => format!(
            "Retry-wait: `{}` failed at attempt {}/{} ({}); next retry scheduled.",
            job.task,
            job.attempts,
            job.max_retries.max(1),
            job.failure_kind.as_deref().unwrap_or("unknown failure"),
        ),
        "approval_wait" => format!(
            "Approval-wait: `{}` is paused on approval `{}` — `{}`.",
            job.task,
            job.approval_id.as_deref().unwrap_or("(no approval id)"),
            job.approval_reason
                .as_deref()
                .unwrap_or("(no reason recorded)"),
        ),
        "dead_lettered" => format!(
            "Dead-lettered: `{}` exhausted {}/{} retries ({}). Operator action required.",
            job.task,
            job.attempts,
            job.max_retries.max(1),
            job.failure_kind.as_deref().unwrap_or("unknown failure"),
        ),
        "loop_stall_escalated" => format!(
            "Loop stall escalated: `{}` exceeded its stall budget — escalated for operator review.",
            job.task
        ),
        "loop_stall_terminated" => format!(
            "Loop stall terminated: `{}` exceeded its stall budget and was forcibly terminated.",
            job.task
        ),
        "succeeded" => format!(
            "Succeeded: `{}` completed after {} attempt(s).",
            job.task,
            job.attempts.max(1),
        ),
        "failed" => format!(
            "Failed: `{}` is in terminal failure ({}). Inspect audit trail before retrying.",
            job.task,
            job.failure_kind.as_deref().unwrap_or("unknown failure"),
        ),
        "canceled" => format!(
            "Canceled: `{}` was cancelled by an operator before completing.",
            job.task
        ),
        other => format!(
            "Unrecognised operational position `{other}` — walk transitions for context."
        ),
    }
}

fn suggest_next_steps(job: &QueueJob, position: &str) -> Vec<String> {
    match position {
        "pending" => vec![
            "if no worker is leasing, confirm `corvid jobs run --source <path>.cor --workers N` is active".to_string(),
            "check `corvid jobs limit list` for a concurrency limit that may be blocking lease"
                .to_string(),
        ],
        "leased" | "running" => vec![
            "if the lease keeps expiring without progress, raise `--lease-ttl-ms` or add a heartbeat"
                .to_string(),
            "for long-running loops, inspect `corvid jobs loop usage --job <id>` for spend / step trends"
                .to_string(),
        ],
        "retry_wait" => vec![
            format!(
                "next retry will fire after `next_run_ms = {}` — inspect failure_fingerprint for the root cause",
                job.next_run_ms.unwrap_or(0)
            ),
            "if the failure_kind matches a known transient pattern, no action needed"
                .to_string(),
            "if the failure recurs, raise `--max-retries` only after confirming the root cause is transient"
                .to_string(),
        ],
        "approval_wait" => vec![
            "decide via `corvid jobs approval decide --job <id> --approval-id <id> --decision approve|deny|expire --actor <op>`"
                .to_string(),
            "audit the approval history via `corvid jobs approval audit --job <id>`".to_string(),
        ],
        "dead_lettered" => vec![
            "inspect every audit transition — the failure kind + fingerprint pin the recurring failure"
                .to_string(),
            "if the underlying cause is fixed, requeue via `corvid jobs retry --job <id>` (resets attempt counter)"
                .to_string(),
            "if the job should not be re-attempted, document the decision and leave dead-lettered"
                .to_string(),
        ],
        "loop_stall_escalated" => vec![
            "review loop usage + heartbeat history; the stall policy may need tuning"
                .to_string(),
            "if the stall is real (worker hung), terminate the job and investigate the worker process"
                .to_string(),
        ],
        "loop_stall_terminated" => vec![
            "the job was killed by stall policy — re-evaluate the worker before requeueing"
                .to_string(),
        ],
        "succeeded" => vec!["no action required; audit trail is final".to_string()],
        "failed" | "canceled" => vec![
            "terminal state — audit trail is the source of truth for compliance review".to_string(),
        ],
        _ => vec!["walk audit transitions for context".to_string()],
    }
}

fn operator_facts(job: &QueueJob) -> OperatorFacts {
    OperatorFacts {
        task: job.task.clone(),
        status: job.status.as_str().to_string(),
        attempts: job.attempts,
        max_retries: job.max_retries,
        budget_usd: job.budget_usd,
        effect_summary: job.effect_summary.clone(),
        lease_owner: job.lease_owner.clone(),
        lease_expires_ms: job.lease_expires_ms,
        next_run_ms: job.next_run_ms,
        failure_kind: job.failure_kind.clone(),
        failure_fingerprint: job.failure_fingerprint.clone(),
        approval_id: job.approval_id.clone(),
        approval_expires_ms: job.approval_expires_ms,
        approval_reason: job.approval_reason.clone(),
        replay_key: job.replay_key.clone(),
        idempotency_key: job.idempotency_key.clone(),
    }
}

fn transition_summary(event: &JobAuditEvent) -> JobTransitionSummary {
    JobTransitionSummary {
        audit_event_id: event.id.clone(),
        event_kind: event.event_kind.clone(),
        status_before: event.status_before.clone(),
        status_after: event.status_after.clone(),
        actor: event.actor.clone(),
        reason: event.reason.clone(),
        created_at_ms: event.created_ms,
    }
}

fn loop_usage_summary(usage: JobLoopUsage) -> LoopUsageSummary {
    LoopUsageSummary {
        steps: usage.steps,
        wall_ms: usage.wall_ms,
        spend_usd: usage.spend_usd,
        tool_calls: usage.tool_calls,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn temp_state() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempdir().unwrap();
        let state = dir.path().join("jobs.sqlite");
        (dir, state)
    }

    fn seed_approval_wait_then_deny(state: &Path, task: &str) -> String {
        use corvid_runtime::queue::JobApprovalDecision;
        let queue = DurableQueueRuntime::open(state).unwrap();
        let enqueued = queue
            .enqueue_typed(
                task.to_string(),
                serde_json::json!({}),
                None,
                3,
                0.0,
                Some("test effect".to_string()),
                None,
            )
            .unwrap();
        let leased = queue.lease_next("test-worker", 60_000).unwrap().unwrap();
        queue
            .enter_approval_wait(
                &leased.id,
                "test-worker",
                "approval-x",
                u64::MAX / 4,
                "reviewer must confirm",
            )
            .unwrap();
        queue
            .decide_approval_wait(
                &leased.id,
                "approval-x",
                JobApprovalDecision::Deny,
                "reviewer-alpha",
                Some("not now".to_string()),
            )
            .unwrap();
        enqueued.id
    }

    /// Slice 35V2-P38-G-LR (positive): a job that hit an
    /// approval-decision flow has audit events; every transition
    /// surfaced by the helper has a back-reference in `sources`
    /// — the Grounded<T> contract.
    #[test]
    fn jobs_explain_denied_approval_carries_grounded_sources() {
        let (_dir, state) = temp_state();
        let job_id = seed_approval_wait_then_deny(&state, "send_email");
        let report = run_jobs_explain(&state, &job_id).expect("explain");
        assert!(report
            .operational_position
            .starts_with("approval_denied")
            || report.operational_position == "approval_denied");
        assert!(!report.transitions.is_empty());
        assert!(!report.sources.is_empty());
        for transition in &report.transitions {
            assert!(
                report.sources.contains(&transition.audit_event_id),
                "transition `{}` missing from sources",
                transition.audit_event_id
            );
        }
        // The denial transition is named in the audit trail.
        let has_denial = report
            .transitions
            .iter()
            .any(|t| t.status_after == "approval_denied");
        assert!(has_denial, "expected an approval_denied transition");
        assert_eq!(report.operator_facts.task, "send_email");
        assert_eq!(report.operator_facts.approval_id.as_deref(), Some("approval-x"));
    }

    /// Slice 35V2-P38-G-LR (positive, pending): a freshly-enqueued
    /// job classifies as `pending` with the right suggested-next-step
    /// (confirm `corvid jobs run` is active).
    #[test]
    fn jobs_explain_pending_suggests_run_workers() {
        let (_dir, state) = temp_state();
        let queue = DurableQueueRuntime::open(&state).unwrap();
        let enqueued = queue
            .enqueue_typed(
                "summarise_doc".to_string(),
                serde_json::json!({}),
                None,
                3,
                0.0,
                None,
                None,
            )
            .unwrap();
        let report = run_jobs_explain(&state, &enqueued.id).unwrap();
        assert_eq!(report.operational_position, "pending");
        assert!(report.headline.contains("Pending"));
        assert!(report
            .suggested_next_steps
            .iter()
            .any(|s| s.contains("`corvid jobs run")));
    }

    /// Slice 35V2-P38-G-LR (adversarial): a missing job id returns
    /// a clear `not found` error, never a silent empty report.
    #[test]
    fn jobs_explain_unknown_job_refuses() {
        let (_dir, state) = temp_state();
        // Open + init the schema so the runtime is well-formed.
        let _ = DurableQueueRuntime::open(&state).unwrap();
        let err = run_jobs_explain(&state, "no-such-job")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "expected `not found`, got: {err}");
    }
}
