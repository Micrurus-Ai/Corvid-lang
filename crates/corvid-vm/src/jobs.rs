//! Job executor that drives durable-queue jobs against a real `Runtime`.
//!
//! Slices `35V2-P38-C-1` and `35V2-P38-C-2` — the cross-layer bridge that
//! lets the multi-worker `WorkerPool` (shipped in slice 38K) execute
//! agent bodies instead of the no-op default, plus per-job JSONL trace
//! emission for `@replayable` agents (gated on `IrAgent.is_replayable`).
//! Production-mode `corvid jobs run --source <path>.cor` compiles the
//! source through the normal driver pipeline, constructs a
//! `DefaultJobRuntimeExecutor` over the resulting `IrFile`, and wires it
//! into the pool via `into_pool_executor`. Test executors and
//! single-task per-pool deployments can implement [`JobRuntimeExecutor`]
//! directly and reuse the same adapter.
//!
//! Trace path policy: a `@replayable` job persists its JSONL trace to
//! `<trace_dir>/<job_id>.jsonl`, where `trace_dir` is configurable via
//! [`DefaultJobRuntimeExecutor::with_trace_dir`] and defaults to
//! `target/trace/jobs`. Non-`@replayable` jobs emit no per-job trace
//! file (they still emit to the runtime's shared tracer if one is
//! attached). `QueueJob.replay_key` is operator-provided metadata at
//! enqueue time and is NOT mutated by the executor — the trace path is
//! always derivable from `job_id`.
//!
//! Sub-slice C-3 will extend this surface with replay-mode dispatch
//! (read the trace, drive the executor with quarantined adapters); C-1
//! and C-2 ship the *live*-mode execution + recording path only.

use corvid_ir::{IrFile, IrType};
use corvid_resolve::DefId;
use corvid_runtime::queue::QueueJob;
use corvid_runtime::tracing::Tracer;
use corvid_runtime::worker_pool::{JobExecutor, JobOutcome};
use corvid_runtime::{Runtime, RuntimeError};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;

use crate::conv::{json_to_value, value_to_json};
use crate::interp::run_agent;

/// Executes one leased `QueueJob` against a Runtime. Implementors translate
/// a job (task name + payload) into a [`JobOutcome`] the pool consumes to
/// finalise the job state.
///
/// The trait is sync. Async work (LLM calls, HTTP, store IO) happens behind
/// `Handle::block_on` inside the implementor — the surrounding
/// `WorkerPool` runs the executor on `tokio::task::spawn_blocking`, so the
/// blocking call is on a worker thread rather than a reactor thread.
pub trait JobRuntimeExecutor: Send + Sync {
    fn execute(&self, runtime: &Runtime, job: &QueueJob) -> Result<JobOutcome, RuntimeError>;
}

/// Default executor: resolves an agent by name from a compiled [`IrFile`]
/// and runs it through the interpreter against the supplied `Runtime`.
///
/// Resolution rules:
/// - `job.task` not declared in the IR → [`JobOutcome::Skip`]. Per-task
///   pools can coexist on one queue without misclaiming each others' jobs.
/// - Payload is not a JSON array (after unwrapping the schedule-fire
///   envelope for cron-fired jobs) → [`JobOutcome::Failure`] with kind
///   `PayloadShape`. Retry/backoff follows the queue's normal policy.
/// - Payload arity does not match the agent's params → [`JobOutcome::Failure`]
///   with kind `PayloadArity`.
/// - JSON → typed `Value` conversion fails for any arg →
///   [`JobOutcome::Failure`] with kind `PayloadType`.
/// - Agent body executes and the interpreter returns an error →
///   [`JobOutcome::Failure`] with kind `AgentInterpreter`.
/// - Success → [`JobOutcome::Success`] with `output_fingerprint` =
///   `sha256:<hex(value_to_json)>` and `output_kind` = the JSON shape
///   label (operator-visible metadata only; the canonical replay
///   comparison happens through Phase 21's `TraceEvent::RunCompleted`).
pub struct DefaultJobRuntimeExecutor {
    ir: Arc<IrFile>,
    trace_dir: PathBuf,
}

impl DefaultJobRuntimeExecutor {
    pub fn new(ir: Arc<IrFile>) -> Self {
        Self {
            ir,
            trace_dir: PathBuf::from("target/trace/jobs"),
        }
    }

    /// Override the directory where `@replayable` jobs persist their
    /// per-job JSONL traces. The trace file always lands at
    /// `<trace_dir>/<job_id>.jsonl`. Tests typically point this at a
    /// `tempfile::tempdir()` to avoid polluting `target/`.
    pub fn with_trace_dir(mut self, dir: PathBuf) -> Self {
        self.trace_dir = dir;
        self
    }

    pub fn ir(&self) -> &IrFile {
        &self.ir
    }

    pub fn trace_dir(&self) -> &std::path::Path {
        &self.trace_dir
    }
}

impl JobRuntimeExecutor for DefaultJobRuntimeExecutor {
    fn execute(&self, runtime: &Runtime, job: &QueueJob) -> Result<JobOutcome, RuntimeError> {
        let Some(agent) = self.ir.agents.iter().find(|a| a.name == job.task) else {
            return Ok(JobOutcome::Skip);
        };

        // A job fired by a queue schedule arrives wrapped in the
        // schedule-fire envelope `{ "schedule_id", "scheduled_fire_ms",
        // "payload": [...] }` (see `enqueue_schedule_fire`); unwrap it
        // so scheduled agents receive the same positional-args array a
        // directly-enqueued job carries. The envelope metadata stays in
        // the queue row for audit.
        let effective_payload = match &job.payload {
            serde_json::Value::Object(fields)
                if fields.contains_key("schedule_id") && fields.contains_key("payload") =>
            {
                &fields["payload"]
            }
            other => other,
        };
        let serde_json::Value::Array(payload_items) = effective_payload else {
            return Ok(JobOutcome::Failure {
                failure_kind: "PayloadShape".to_string(),
                failure_fingerprint:
                    "expected a JSON array of agent arguments (or a schedule-fire envelope \
                     wrapping one); got a different JSON shape"
                        .to_string(),
                base_delay_ms: 1_000,
            });
        };

        if payload_items.len() != agent.params.len() {
            return Ok(JobOutcome::Failure {
                failure_kind: "PayloadArity".to_string(),
                failure_fingerprint: format!(
                    "agent `{}` expects {} arguments; payload supplied {}",
                    agent.name,
                    agent.params.len(),
                    payload_items.len()
                ),
                base_delay_ms: 1_000,
            });
        }

        let types_by_id: HashMap<DefId, &IrType> =
            self.ir.types.iter().map(|t| (t.id, t)).collect();
        let mut args = Vec::with_capacity(agent.params.len());
        for (param, raw) in agent.params.iter().zip(payload_items.iter().cloned()) {
            match json_to_value(raw, &param.ty, &types_by_id) {
                Ok(value) => args.push(value),
                Err(err) => {
                    return Ok(JobOutcome::Failure {
                        failure_kind: "PayloadType".to_string(),
                        failure_fingerprint: format!(
                            "agent `{}` param `{}`: {}",
                            agent.name, param.name, err
                        ),
                        base_delay_ms: 1_000,
                    });
                }
            }
        }

        // For `@replayable` agents (slice C-2), open a per-job JSONL
        // tracer at `<trace_dir>/<job_id>.jsonl` and swap it into the
        // runtime. The interpreter's `RunStarted` / `ToolCall` /
        // `LlmCall` / `ApprovalDecision` / `RunCompleted` emits then
        // land in the per-job file instead of (or alongside) the
        // runtime's shared tracer. Non-`@replayable` agents skip this
        // step and use the supplied runtime verbatim.
        let job_runtime = if agent.is_replayable {
            // Best-effort directory creation. Tracer::open / its writer
            // already swallow IO failures (per the tracing module's
            // "broken tracer must never crash an agent" rule), so even
            // if create_dir_all fails the agent run still completes —
            // it just doesn't get a trace file.
            let _ = std::fs::create_dir_all(&self.trace_dir);
            let tracer = Tracer::open(&self.trace_dir, &job.id);
            runtime.with_tracer(tracer)
        } else {
            runtime.clone()
        };

        // Drive the async interpreter from a sync executor closure. The
        // surrounding `WorkerPool` calls us under
        // `tokio::task::spawn_blocking`, so we are on a blocking worker
        // thread — `Handle::current().block_on` is safe and does not
        // deadlock the reactor.
        let handle = Handle::current();
        let ir = self.ir.clone();
        let agent_name = agent.name.clone();
        let interp_result = handle.block_on(async move {
            run_agent(ir.as_ref(), &agent_name, args, &job_runtime).await
        });

        match interp_result {
            Ok(value) => {
                let json = value_to_json(&value);
                let canonical = serde_json::to_string(&json).map_err(|err| {
                    RuntimeError::Other(format!(
                        "failed to canonicalise agent output for fingerprint: {err}"
                    ))
                })?;
                let mut hasher = Sha256::new();
                hasher.update(canonical.as_bytes());
                let fingerprint = format!("sha256:{:x}", hasher.finalize());
                Ok(JobOutcome::Success {
                    output_kind: Some(json_kind_label(&json).to_string()),
                    output_fingerprint: Some(fingerprint),
                })
            }
            Err(err) => Ok(JobOutcome::Failure {
                failure_kind: "AgentInterpreter".to_string(),
                failure_fingerprint: err.to_string(),
                base_delay_ms: 1_000,
            }),
        }
    }
}

fn json_kind_label(json: &serde_json::Value) -> &'static str {
    match json {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Adapt a [`JobRuntimeExecutor`] into the sync closure shape the
/// `WorkerPool` consumes. The `Runtime` is captured in an `Arc` so the
/// pool's `spawn_blocking` invocations can share one immutable Runtime
/// handle across worker threads.
pub fn into_pool_executor(
    executor: Arc<dyn JobRuntimeExecutor>,
    runtime: Arc<Runtime>,
) -> JobExecutor {
    Arc::new(move |job: &QueueJob| executor.execute(runtime.as_ref(), job))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ir::lower;
    use corvid_resolve::resolve;
    use corvid_runtime::approvals::ProgrammaticApprover;
    use corvid_syntax::{lex, parse_file};
    use corvid_types::typecheck;

    /// Compile `source` all the way to IR through the workspace's lex →
    /// parse → resolve → typecheck → lower path. Mirrors the pattern in
    /// `corvid-vm/src/tests/mod.rs::ir_of`. Stays inside corvid-vm to
    /// avoid pulling `corvid-driver` (which depends on corvid-vm) and
    /// creating a circular dep.
    fn compile(source: &str) -> IrFile {
        let tokens = lex(source).expect("lex");
        let (file, perr) = parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = resolve(&file);
        assert!(
            resolved.errors.is_empty(),
            "resolve: {:?}",
            resolved.errors
        );
        let checked = typecheck(&file, &resolved);
        assert!(
            checked.errors.is_empty(),
            "typecheck: {:?}",
            checked.errors
        );
        lower(&file, &resolved, &checked)
    }

    fn empty_runtime() -> Runtime {
        Runtime::builder()
            .approver(Arc::new(ProgrammaticApprover::always_yes()))
            .build()
    }

    fn fake_job(task: &str, payload: serde_json::Value) -> QueueJob {
        QueueJob {
            id: "job-1".to_string(),
            task: task.to_string(),
            payload,
            input_schema: None,
            status: corvid_runtime::queue::QueueJobStatus::Leased,
            attempts: 0,
            max_retries: 1,
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

    /// A schedule-fired job arrives wrapped in the schedule-fire
    /// envelope; the executor must unwrap it and execute the agent
    /// with the inner positional args.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_unwraps_schedule_fire_envelope() {
        let ir = Arc::new(compile(
            r#"
agent heartbeat(label: String) -> String:
    return label
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job(
            "heartbeat",
            serde_json::json!({
                "schedule_id": "sched_heartbeat_0",
                "scheduled_fire_ms": 1234,
                "payload": ["tick"],
            }),
        );

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        match outcome {
            JobOutcome::Success { .. } => {}
            other => panic!("schedule-fire envelope must unwrap to Success; got {other:?}"),
        }
    }

    /// Positive: an agent with no params + a String return resolves and
    /// executes through the real interpreter, producing a Success outcome
    /// with a stable fingerprint.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_runs_zero_arg_agent_to_success() {
        let ir = Arc::new(compile(
            r#"
agent noop() -> String:
    return "ok"
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job("noop", serde_json::json!([]));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        match outcome {
            JobOutcome::Success {
                output_kind,
                output_fingerprint,
            } => {
                assert_eq!(output_kind.as_deref(), Some("string"));
                let fp = output_fingerprint.expect("fingerprint present");
                assert!(fp.starts_with("sha256:"));
                // Stable across runs: `value_to_json("ok")` is the JSON string
                // `"ok"` whose SHA256 is deterministic.
                let mut hasher = Sha256::new();
                hasher.update("\"ok\"".as_bytes());
                let expected = format!("sha256:{:x}", hasher.finalize());
                assert_eq!(fp, expected);
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    /// Adversarial: when the job's task does not match any agent in the
    /// compiled IR, the executor returns Skip so the lease can release
    /// back to the queue for another (per-task) worker pool to claim.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_skips_unknown_agent() {
        let ir = Arc::new(compile(
            r#"
agent noop() -> String:
    return "ok"
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job("not_declared", serde_json::json!([]));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        assert_eq!(outcome, JobOutcome::Skip);
    }

    /// Adversarial: payload is a JSON object instead of an array. Returns
    /// PayloadShape failure so the queue's retry policy applies.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_rejects_non_array_payload() {
        let ir = Arc::new(compile(
            r#"
agent noop() -> String:
    return "ok"
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job("noop", serde_json::json!({"oops": 1}));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        match outcome {
            JobOutcome::Failure {
                failure_kind,
                ..
            } => assert_eq!(failure_kind, "PayloadShape"),
            other => panic!("expected PayloadShape failure, got {other:?}"),
        }
    }

    /// Adversarial: arity mismatch. Agent takes one String, payload is
    /// empty.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_rejects_wrong_arity() {
        let ir = Arc::new(compile(
            r#"
agent greet(name: String) -> String:
    return name
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job("greet", serde_json::json!([]));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        match outcome {
            JobOutcome::Failure { failure_kind, .. } => {
                assert_eq!(failure_kind, "PayloadArity");
            }
            other => panic!("expected PayloadArity failure, got {other:?}"),
        }
    }

    /// Slice C-2 positive: a `@replayable` agent's run emits a per-job
    /// JSONL trace at `<trace_dir>/<job_id>.jsonl`. The file exists,
    /// is non-empty, and every line round-trips through
    /// `corvid_trace_schema::TraceEvent` deserialisation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn replayable_agent_emits_per_job_jsonl_trace() {
        let ir = Arc::new(compile(
            r#"
@replayable
agent noop() -> String:
    return "ok"
"#,
        ));
        let runtime = empty_runtime();
        let trace_dir = tempfile::tempdir().expect("tempdir");
        let executor = DefaultJobRuntimeExecutor::new(ir)
            .with_trace_dir(trace_dir.path().to_path_buf());
        let job = fake_job("noop", serde_json::json!([]));
        let job_id = job.id.clone();
        let trace_path = trace_dir.path().join(format!("{job_id}.jsonl"));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        assert!(matches!(outcome, JobOutcome::Success { .. }));

        let raw = std::fs::read_to_string(&trace_path)
            .unwrap_or_else(|err| panic!("trace at {trace_path:?} must exist: {err}"));
        assert!(!raw.is_empty(), "trace file should not be empty");
        let mut event_count = 0;
        for line in raw.lines() {
            if line.is_empty() {
                continue;
            }
            let event: corvid_trace_schema::TraceEvent = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("trace line failed to deserialise: {err}\n{line}"));
            event_count += 1;
            // Sanity: at least the schema header + RunStarted +
            // RunCompleted should appear.
            let _ = event;
        }
        assert!(
            event_count >= 3,
            "trace should contain ≥3 events (header + start + completed), got {event_count}"
        );
    }

    /// Slice C-2 adversarial: a non-`@replayable` agent does NOT emit
    /// a per-job trace file. The opposite test of the positive case —
    /// gates the executor on the IR-level attribute.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn non_replayable_agent_emits_no_per_job_trace() {
        let ir = Arc::new(compile(
            r#"
agent noop() -> String:
    return "ok"
"#,
        ));
        let runtime = empty_runtime();
        let trace_dir = tempfile::tempdir().expect("tempdir");
        let executor = DefaultJobRuntimeExecutor::new(ir)
            .with_trace_dir(trace_dir.path().to_path_buf());
        let job = fake_job("noop", serde_json::json!([]));
        let job_id = job.id.clone();
        let trace_path = trace_dir.path().join(format!("{job_id}.jsonl"));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        assert!(matches!(outcome, JobOutcome::Success { .. }));
        assert!(
            !trace_path.exists(),
            "non-@replayable agent must not write {trace_path:?}"
        );
    }

    /// Positive: agent with a String param resolves, binds, returns the
    /// input.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn default_executor_runs_string_arg_agent() {
        let ir = Arc::new(compile(
            r#"
agent greet(name: String) -> String:
    return name
"#,
        ));
        let runtime = empty_runtime();
        let executor = DefaultJobRuntimeExecutor::new(ir);
        let job = fake_job("greet", serde_json::json!(["world"]));

        let outcome = tokio::task::spawn_blocking(move || executor.execute(&runtime, &job))
            .await
            .expect("blocking task completed")
            .expect("executor surface ok");
        match outcome {
            JobOutcome::Success {
                output_fingerprint,
                ..
            } => {
                let fp = output_fingerprint.expect("fingerprint present");
                let mut hasher = Sha256::new();
                hasher.update("\"world\"".as_bytes());
                let expected = format!("sha256:{:x}", hasher.finalize());
                assert_eq!(fp, expected);
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }
}
