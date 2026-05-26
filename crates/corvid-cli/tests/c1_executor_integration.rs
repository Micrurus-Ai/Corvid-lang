//! End-to-end integration tests for `35V2-P38-C-1` — the job→Runtime
//! executor bridge. Verifies:
//!
//! - A persisted job whose task matches a compiled agent runs through the
//!   real `Runtime` + `WorkerPool` stack and reaches `Succeeded` with the
//!   expected `output_fingerprint`.
//! - A persisted job whose task does not match any compiled agent is
//!   skipped by the executor; the lease releases and the job is still
//!   eligible for another (per-task) worker pool.
//!
//! Lives in `corvid-cli/tests` because the test compiles `.cor` source
//! through `corvid-driver`. corvid-vm cannot depend on corvid-driver
//! (the latter depends on the former), so the integration test must
//! live in a higher-level crate.

use corvid_driver::compile_to_ir_with_config_at_path;
use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_runtime::queue::{DurableQueueRuntime, QueueJobStatus};
use corvid_runtime::worker_pool::WorkerPool;
use corvid_runtime::Runtime;
use corvid_vm::{into_pool_executor, DefaultJobRuntimeExecutor, JobRuntimeExecutor};
use sha2::Digest;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

fn build_runtime() -> Runtime {
    Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .build()
}

/// Positive: one persisted job whose task matches a declared agent runs
/// to Succeeded through the real Runtime, with an output fingerprint
/// derived from the agent's return value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_one_persisted_job_runs_through_real_runtime() {
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
    queue
        .enqueue_typed(
            "noop",
            serde_json::json!([]),
            None,
            1,
            0.0,
            Some("test".to_string()),
            Some("rk:c1-positive".to_string()),
        )
        .unwrap();

    let runtime_handle = Arc::new(build_runtime());
    let executor: Arc<dyn JobRuntimeExecutor> = Arc::new(DefaultJobRuntimeExecutor::new(ir));
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

    // The single job reached succeeded state; the output_fingerprint
    // matches sha256 of the canonical JSON return ("ok").
    let jobs = queue.list().unwrap();
    assert_eq!(jobs.len(), 1, "exactly one job persisted");
    let job = &jobs[0];
    assert_eq!(job.task, "noop");
    assert_eq!(job.status, QueueJobStatus::Succeeded);
    assert_eq!(job.output_kind.as_deref(), Some("string"));
    let mut hasher = sha2::Sha256::new();
    hasher.update("\"ok\"".as_bytes());
    let expected = format!("sha256:{:x}", hasher.finalize());
    assert_eq!(job.output_fingerprint.as_deref(), Some(expected.as_str()));
    assert_eq!(counters.succeeded(), 1);
    assert_eq!(counters.failed(), 0);
}

/// Adversarial: the job's task name does not appear in the compiled
/// source, so the executor reports Skip. The pool releases the lease
/// without finalising the job. The job is still eligible (pending or
/// retry_wait) for another worker pool to claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_unknown_task_skips_and_leaves_job_eligible() {
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
    queue
        .enqueue_typed(
            "not_declared",
            serde_json::json!([]),
            None,
            1,
            0.0,
            Some("test".to_string()),
            Some("rk:c1-skip".to_string()),
        )
        .unwrap();

    let runtime_handle = Arc::new(build_runtime());
    let executor: Arc<dyn JobRuntimeExecutor> = Arc::new(DefaultJobRuntimeExecutor::new(ir));
    let job_executor = into_pool_executor(executor, runtime_handle);

    let pool = WorkerPool::new(queue.clone(), 1)
        .with_executor(job_executor)
        .with_lease_ttl_ms(5_000)
        .with_idle_poll_ms(10);
    let drain = pool.drain_handle();
    let counters = pool.counters();
    let handles = pool.spawn();

    // Let the pool try the job at least once.
    let start = std::time::Instant::now();
    while counters.total() < 1 {
        if start.elapsed().as_millis() > 2_000 {
            panic!(
                "timed out waiting for at least one skip (succeeded={}, failed={}, skipped={})",
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

    assert!(
        counters.skipped() >= 1,
        "unknown task counted as skip (succeeded={}, failed={}, skipped={})",
        counters.succeeded(),
        counters.failed(),
        counters.skipped()
    );
    assert_eq!(counters.succeeded(), 0);
    assert_eq!(counters.failed(), 0);

    // The job did not reach a terminal state. It is still pending /
    // retry_wait / leased so another per-task pool can claim it.
    let jobs = queue.list().unwrap();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert!(
        !matches!(
            job.status,
            QueueJobStatus::Succeeded | QueueJobStatus::DeadLettered | QueueJobStatus::Canceled
        ),
        "skipped job must not be in a terminal state, got {:?}",
        job.status
    );
}
