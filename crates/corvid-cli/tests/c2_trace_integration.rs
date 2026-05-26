//! Integration tests for `35V2-P38-C-2` — per-job JSONL trace emission
//! for `@replayable` durable-queue jobs through the multi-worker pool.
//!
//! Drives the full path: enqueue → WorkerPool 1-worker → executor with
//! per-job tracer → terminal transition → assert trace file on disk
//! round-trips through `corvid_trace_schema::TraceEvent`. The negative
//! test confirms a non-`@replayable` agent produces no trace file.

use corvid_driver::compile_to_ir_with_config_at_path;
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_runtime::queue::{DurableQueueRuntime, QueueJobStatus};
use corvid_runtime::worker_pool::WorkerPool;
use corvid_runtime::Runtime;
use corvid_vm::{into_pool_executor, DefaultJobRuntimeExecutor, JobRuntimeExecutor};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn build_runtime() -> Runtime {
    Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .build()
}

/// Positive: a `@replayable` agent run through the multi-worker pool
/// emits a per-job JSONL trace at `<trace_dir>/<job_id>.jsonl`. The
/// file exists, every line deserialises through `TraceEvent`, and the
/// sequence contains at least the schema header + RunStarted +
/// RunCompleted events the interpreter is contracted to emit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_replayable_job_emits_per_job_jsonl_trace_through_pool() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("noop.cor");
    std::fs::write(
        &source_path,
        r#"
@replayable
agent noop() -> String:
    return "ok"
"#,
    )
    .unwrap();
    let source = std::fs::read_to_string(&source_path).unwrap();
    let ir = Arc::new(
        compile_to_ir_with_config_at_path(&source, &source_path, None).unwrap_or_else(|diags| {
            panic!(
                "test source failed to compile: {}",
                diags
                    .into_iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }),
    );

    let queue = Arc::new(DurableQueueRuntime::open(&dir.path().join("queue.db")).unwrap());
    let enqueued = queue
        .enqueue_typed(
            "noop",
            serde_json::json!([]),
            None,
            1,
            0.0,
            Some("test".to_string()),
            Some("rk:c2-positive".to_string()),
        )
        .unwrap();
    let job_id = enqueued.id.clone();

    let runtime_handle = Arc::new(build_runtime());
    let trace_dir = dir.path().join("traces");
    let executor: Arc<dyn JobRuntimeExecutor> = Arc::new(
        DefaultJobRuntimeExecutor::new(ir).with_trace_dir(trace_dir.clone()),
    );
    let job_executor = into_pool_executor(executor, runtime_handle);

    let pool = WorkerPool::new(queue.clone(), 1)
        .with_executor(job_executor)
        .with_lease_ttl_ms(5_000)
        .with_idle_poll_ms(10);
    let drain = pool.drain_handle();
    let counters = pool.counters();
    let handles = pool.spawn();

    let timeout_ms = 5_000;
    let start = std::time::Instant::now();
    loop {
        if counters.succeeded() >= 1 {
            break;
        }
        if start.elapsed().as_millis() as u64 > timeout_ms {
            panic!(
                "timed out waiting for job (succeeded={}, failed={}, skipped={})",
                counters.succeeded(),
                counters.failed(),
                counters.skipped()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.store(true, Ordering::SeqCst);
    for h in handles {
        let _ = h.await;
    }

    let jobs = queue.list().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, QueueJobStatus::Succeeded);

    let trace_path = trace_dir.join(format!("{job_id}.jsonl"));
    let raw = std::fs::read_to_string(&trace_path)
        .unwrap_or_else(|err| panic!("trace at {trace_path:?} must exist: {err}"));
    assert!(!raw.is_empty(), "trace file empty: {trace_path:?}");

    let mut events: Vec<corvid_trace_schema::TraceEvent> = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        let event: corvid_trace_schema::TraceEvent = serde_json::from_str(line)
            .unwrap_or_else(|err| panic!("trace line failed to deserialise: {err}\n{line}"));
        events.push(event);
    }
    assert!(
        events.len() >= 3,
        "expected ≥3 events (header + start + completed), got {} in {trace_path:?}",
        events.len()
    );
    // The interpreter contracts `RunStarted` + `RunCompleted` (with the
    // agent name) for every agent run — assert both appear.
    use corvid_trace_schema::TraceEvent::*;
    let has_run_started = events
        .iter()
        .any(|e| matches!(e, RunStarted { agent, .. } if agent == "noop"));
    let has_run_completed = events
        .iter()
        .any(|e| matches!(e, RunCompleted { ok: true, .. }));
    assert!(
        has_run_started,
        "RunStarted{{agent: \"noop\"}} absent from trace: {events:?}"
    );
    assert!(
        has_run_completed,
        "RunCompleted{{ok: true}} absent from trace: {events:?}"
    );
}

/// Adversarial: a non-`@replayable` agent produces NO trace file at
/// the expected path. Trace emission is gated on `IrAgent.is_replayable`
/// (lowered from `AgentAttribute::is_replayable`), not on every job.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_non_replayable_job_emits_no_trace_file() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("noop.cor");
    std::fs::write(
        &source_path,
        r#"
agent noop() -> String:
    return "ok"
"#,
    )
    .unwrap();
    let source = std::fs::read_to_string(&source_path).unwrap();
    let ir = Arc::new(
        compile_to_ir_with_config_at_path(&source, &source_path, None).unwrap_or_else(|diags| {
            panic!(
                "test source failed to compile: {}",
                diags
                    .into_iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }),
    );

    let queue = Arc::new(DurableQueueRuntime::open(&dir.path().join("queue.db")).unwrap());
    let enqueued = queue
        .enqueue_typed(
            "noop",
            serde_json::json!([]),
            None,
            1,
            0.0,
            Some("test".to_string()),
            Some("rk:c2-negative".to_string()),
        )
        .unwrap();
    let job_id = enqueued.id.clone();

    let runtime_handle = Arc::new(build_runtime());
    let trace_dir = dir.path().join("traces");
    let executor: Arc<dyn JobRuntimeExecutor> = Arc::new(
        DefaultJobRuntimeExecutor::new(ir).with_trace_dir(trace_dir.clone()),
    );
    let job_executor = into_pool_executor(executor, runtime_handle);

    let pool = WorkerPool::new(queue.clone(), 1)
        .with_executor(job_executor)
        .with_lease_ttl_ms(5_000)
        .with_idle_poll_ms(10);
    let drain = pool.drain_handle();
    let counters = pool.counters();
    let handles = pool.spawn();

    let timeout_ms = 5_000;
    let start = std::time::Instant::now();
    loop {
        if counters.succeeded() >= 1 {
            break;
        }
        if start.elapsed().as_millis() as u64 > timeout_ms {
            panic!("timed out waiting for job");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.store(true, Ordering::SeqCst);
    for h in handles {
        let _ = h.await;
    }

    let trace_path = trace_dir.join(format!("{job_id}.jsonl"));
    assert!(
        !trace_path.exists(),
        "non-@replayable agent must not emit a trace at {trace_path:?}"
    );
}
