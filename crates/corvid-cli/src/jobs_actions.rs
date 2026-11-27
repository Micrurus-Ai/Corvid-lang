//! Actions as background jobs (B10-4b): `corvid jobs enqueue --task
//! <action>` and the workers of `corvid jobs run` call a declared action
//! through the same path every other surface uses (`action_calls`). Its
//! input is the action's public JSON input, checked at enqueue time when
//! the source is given and again when a worker runs it; its result is
//! recorded as the job's outcome; and the call is traced as
//! `surface: "worker"`.
//!
//! A worker runs as no one. An action that requires a signed-in caller is
//! refused by name until B11 carries the enqueuing person's authority with
//! the job, never run without its `requires`. A task that names an
//! action's body directly is refused by the plain executor, so a payload
//! can never supply a forged `Caller`.

use std::collections::HashMap;
use std::sync::Arc;

use corvid_ir::{IrFile, IrType};
use corvid_resolve::DefId;
use corvid_runtime::queue::QueueJob;
use corvid_runtime::worker_pool::JobOutcome;
use corvid_runtime::{Runtime, RuntimeError};
use corvid_vm::{DefaultJobRuntimeExecutor, JobRuntimeExecutor};

use crate::action_calls::{catalog, decode_json_input, ActionEndpoint};

/// Why an action cannot be enqueued or run as a job, as the operator
/// reads it.
pub(crate) fn refusal(endpoint: &ActionEndpoint) -> Option<String> {
    endpoint.policy.requires_auth().then(|| {
        format!(
            "action `{}` requires a signed-in caller, and a worker runs as no one: carrying the enqueuing person's authority with the job is B11's on-behalf-of authority. Call it through `corvid serve` with a session for now.",
            endpoint.name
        )
    })
}

/// Check a job's payload against the action it names, as `jobs enqueue
/// --source` does before storing it. `Ok(None)` when the task names no
/// action, so any other job is enqueued as before.
pub(crate) fn check_enqueue(ir: &IrFile, task: &str, payload: &serde_json::Value) -> anyhow::Result<Option<String>> {
    let actions = catalog(ir)?;
    let Some(endpoint) = actions.iter().find(|a| a.name == task) else {
        return Ok(None);
    };
    if let Some(refused) = refusal(endpoint) {
        anyhow::bail!(refused);
    }
    let types_by_id: HashMap<DefId, &IrType> = ir.types.iter().map(|t| (t.id, t)).collect();
    if let Err(refused) = decode_json_input(endpoint, payload, &types_by_id) {
        anyhow::bail!(
            "the payload is not `{}`'s input: {}",
            endpoint.name,
            refused["detail"].as_str().unwrap_or("the input was refused")
        );
    }
    Ok(Some(endpoint.name.clone()))
}

/// The workers' executor: a job whose task names an action runs through
/// the action path; any other job runs as before.
pub(crate) struct ActionJobExecutor {
    ir: Arc<IrFile>,
    actions: Vec<Arc<ActionEndpoint>>,
    agents: DefaultJobRuntimeExecutor,
}

impl ActionJobExecutor {
    pub(crate) fn new(ir: Arc<IrFile>, agents: DefaultJobRuntimeExecutor) -> anyhow::Result<Self> {
        Ok(Self { actions: catalog(&ir)?, ir, agents })
    }
}

impl JobRuntimeExecutor for ActionJobExecutor {
    fn execute(&self, runtime: &Runtime, job: &QueueJob) -> Result<JobOutcome, RuntimeError> {
        let Some(endpoint) = self.actions.iter().find(|a| a.name == job.task) else {
            return self.agents.execute(runtime, job);
        };
        let failure = |kind: &str, detail: String| JobOutcome::Failure {
            failure_kind: kind.to_string(),
            failure_fingerprint: detail,
            base_delay_ms: 1_000,
        };
        if let Some(refused) = refusal(endpoint) {
            return Ok(failure("ActionNeedsSignedInCaller", refused));
        }
        let types_by_id: HashMap<DefId, &IrType> = self.ir.types.iter().map(|t| (t.id, t)).collect();
        let inputs = match decode_json_input(endpoint, &job.payload, &types_by_id) {
            Ok(inputs) => inputs,
            Err(refused) => {
                return Ok(failure(
                    "ActionInput",
                    refused["detail"].as_str().unwrap_or("the input was refused").to_string(),
                ))
            }
        };
        runtime.emit_host_event(
            "action",
            serde_json::json!({ "action": endpoint.name, "surface": "worker", "job": job.id }),
        );
        self.agents.run_job_agent(runtime, job, &endpoint.agent, inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROGRAM: &str = "identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    roles:
        member: \"cards.edit\"

type Label:
    card: Int
    label: String

action add_label(input: Label) -> String requires anyone:
    return input.label + \" on \" + input.card.to_string()

action claim(card: Int, by: Caller) -> Int requires permission(\"cards.edit\"):
    return card
";

    fn job(task: &str, payload: serde_json::Value) -> QueueJob {
        QueueJob {
            id: "job-7".to_string(),
            task: task.to_string(),
            payload,
            input_schema: None,
            status: corvid_runtime::queue::QueueJobStatus::Leased,
            attempts: 0,
            max_retries: 0,
            budget_usd: 0.0,
            effect_summary: None,
            replay_key: None,
            idempotency_key: None,
            output_kind: None,
            output_fingerprint: None,
            failure_kind: None,
            failure_fingerprint: None,
            next_run_ms: None,
            lease_owner: Some("worker-0".to_string()),
            lease_expires_ms: None,
            approval_id: None,
            approval_expires_ms: None,
            approval_reason: None,
            created_ms: 0,
            updated_ms: 0,
        }
    }

    async fn run(runtime: Runtime, job: QueueJob) -> JobOutcome {
        let ir = Arc::new(corvid_driver::compile_to_ir(PROGRAM).expect("compiles"));
        let executor = ActionJobExecutor::new(ir.clone(), DefaultJobRuntimeExecutor::new(ir)).unwrap();
        tokio::task::spawn_blocking(move || executor.execute(&runtime, &job).unwrap()).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_worker_runs_an_action_from_its_public_input_and_traces_it_as_a_worker() {
        let trace_dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::builder().trace_to(trace_dir.path()).build();
        let trace_path = runtime.tracer().path().to_path_buf();
        let outcome = run(runtime, job("add_label", json!({"card": 7, "label": "urgent"}))).await;
        assert!(matches!(outcome, JobOutcome::Success { .. }), "{outcome:?}");
        let trace = std::fs::read_to_string(trace_path).unwrap();
        assert!(trace.contains("\"surface\":\"worker\""), "the call is traced as a worker's: {trace}");
        assert!(trace.contains("\"job\":\"job-7\""), "with its job: {trace}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_worker_refuses_a_signed_in_action_and_a_payload_that_is_not_the_input() {
        let refused = run(Runtime::builder().build(), job("claim", json!({"card": 1}))).await;
        assert!(
            matches!(&refused, JobOutcome::Failure { failure_kind, .. } if failure_kind == "ActionNeedsSignedInCaller"),
            "{refused:?}"
        );
        let wrong = run(Runtime::builder().build(), job("add_label", json!({"card": "seven", "label": "x"}))).await;
        assert!(
            matches!(&wrong, JobOutcome::Failure { failure_kind, failure_fingerprint, .. }
                if failure_kind == "ActionInput" && failure_fingerprint.contains("card")),
            "{wrong:?}"
        );
    }

    #[test]
    fn enqueue_checks_an_actions_payload_and_leaves_other_tasks_alone() {
        let ir = corvid_driver::compile_to_ir(PROGRAM).expect("compiles");
        assert_eq!(check_enqueue(&ir, "add_label", &json!({"card": 7, "label": "x"})).unwrap().as_deref(), Some("add_label"));
        let bad = check_enqueue(&ir, "add_label", &json!({"card": 7})).unwrap_err().to_string();
        assert!(bad.contains("not `add_label`'s input"), "{bad}");
        let signed_in = check_enqueue(&ir, "claim", &json!({"card": 7})).unwrap_err().to_string();
        assert!(signed_in.contains("requires a signed-in caller"), "{signed_in}");
        assert!(check_enqueue(&ir, "daily_brief", &json!({})).unwrap().is_none());
    }
}
