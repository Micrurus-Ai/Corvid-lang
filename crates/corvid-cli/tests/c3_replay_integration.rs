//! Integration tests for `35V2-P38-C-3` — `replay_job_from_source`
//! entry point that replays a recorded durable-queue job by `job_id`.
//!
//! Drives the full path: enqueue → 1-worker WorkerPool with the C-1
//! executor (records trace via C-2) → `replay_job_from_source` →
//! assert `ReplayOutcome` matches the original execution shape. The
//! negative test confirms the helpful error message when the trace
//! file is missing (most common cause: original job was not
//! `@replayable`, so C-2 emitted no trace).

use corvid_driver::{compile_to_ir_with_config_at_path, replay_job_from_source};
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_runtime::queue::DurableQueueRuntime;
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

/// Positive: a `@replayable` job runs through the pool (recording a
/// trace via C-2), and then `replay_job_from_source` reproduces the
/// run from the trace — same agent name, same return value, no
/// error. Demonstrates the C-1 → C-2 → C-3 chain end-to-end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c3_replay_job_reproduces_original_result() {
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
            Some("rk:c3-positive".to_string()),
        )
        .unwrap();
    let job_id = enqueued.id.clone();
    let trace_dir = dir.path().join("traces");

    let runtime_handle = Arc::new(build_runtime());
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
            panic!("timed out waiting for job to succeed");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drain.store(true, Ordering::SeqCst);
    for h in handles {
        let _ = h.await;
    }

    // The trace file from C-2 emission should now live at
    // <trace_dir>/<job_id>.jsonl. Drive the C-3 replay against it.
    let base_builder =
        Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes()));
    let outcome = replay_job_from_source(&source_path, &job_id, &trace_dir, base_builder)
        .await
        .expect("replay_job_from_source ok");

    assert_eq!(outcome.agent_name, "noop");
    assert!(outcome.result_error.is_none(), "{outcome:?}");
    let value = outcome.result_value.expect("replay produced a value");
    let json = corvid_vm::value_to_json(&value);
    assert_eq!(json, serde_json::json!("ok"));
}

/// Adversarial: a job whose trace is missing (e.g. it was not
/// `@replayable` at run time, or the trace dir was wiped) surfaces a
/// helpful error message naming the trace path, not a generic
/// not-found.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c3_replay_missing_trace_emits_helpful_error() {
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
    let trace_dir = dir.path().join("traces");
    std::fs::create_dir_all(&trace_dir).unwrap();

    let base_builder =
        Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes()));
    let err = replay_job_from_source(&source_path, "nonexistent-job-id", &trace_dir, base_builder)
        .await
        .expect_err("missing trace must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("nonexistent-job-id"),
        "error must name the job id: {msg}"
    );
    assert!(
        msg.contains("@replayable"),
        "error must hint at @replayable as a possible cause: {msg}"
    );
}
