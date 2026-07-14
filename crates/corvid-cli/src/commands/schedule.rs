//! `corvid schedule run` — the governed-cron runner.
//!
//! Language-level `schedule "<cron>" zone "<zone>" -> agent(args)`
//! declarations become durable-queue schedule manifests, and a
//! scheduler tick loop + the existing worker pool fire them: due
//! fires enqueue idempotently (the queue's dedup + missed-fire
//! policies apply), leased jobs execute the target agent through the
//! interpreter, and `@replayable` agents record per-job traces —
//! scheduled work inherits tracing, retries, dead-letters, and
//! replay from the durable-jobs machinery instead of reinventing
//! any of it.

use anyhow::{Context, Result};
use corvid_ast::{Decl, Expr, Literal, ScheduleDecl};
use corvid_runtime::queue::{DurableQueueRuntime, QueueScheduleManifest, ScheduleMissedPolicy};
use std::path::Path;

pub(crate) fn cmd_schedule_run(
    source: &Path,
    state: &Path,
    workers: usize,
    lease_ttl_ms: u64,
    poll_ms: u64,
    max_runtime_ms: u64,
    max_missed_per_schedule: usize,
) -> Result<u8> {
    use corvid_driver::compile_to_ir_with_config_at_path;
    use corvid_runtime::worker_pool::WorkerPool;
    use corvid_runtime::Runtime;
    use corvid_vm::{into_pool_executor, DefaultJobRuntimeExecutor, JobRuntimeExecutor};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    let source_text = std::fs::read_to_string(source)
        .with_context(|| format!("failed to read schedule source `{}`", source.display()))?;

    // Schedules live in the AST only (they are audit/runtime
    // manifests, not executable IR) — parse the file to collect them.
    let tokens = corvid_syntax::lex(&source_text).map_err(|errors| {
        anyhow::anyhow!("cannot lex `{}`: {errors:?}", source.display())
    })?;
    let (file, parse_errors) = corvid_syntax::parse_file(&tokens);
    if !parse_errors.is_empty() {
        anyhow::bail!("cannot parse `{}`: {parse_errors:?}", source.display());
    }
    let schedules: Vec<&ScheduleDecl> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Schedule(schedule) => Some(schedule),
            _ => None,
        })
        .collect();
    if schedules.is_empty() {
        anyhow::bail!(
            "`{}` declares no `schedule` blocks — nothing to run.\n\
             Declare one like:\n\
             \n    \
             schedule \"0 8 * * *\" zone \"UTC\" -> daily_brief(\"morning\")",
            source.display()
        );
    }

    // The executor resolves agent bodies from the compiled IR — the
    // same pipeline `corvid jobs run` uses.
    let ir = compile_to_ir_with_config_at_path(&source_text, source, None).map_err(|diags| {
        let rendered = diags
            .into_iter()
            .map(|d| format!("- {d}"))
            .collect::<Vec<_>>()
            .join("\n");
        anyhow::anyhow!(
            "failed to compile schedule source `{}`:\n{}",
            source.display(),
            rendered
        )
    })?;

    let queue = Arc::new(DurableQueueRuntime::open(state)?);

    println!("corvid schedule run");
    println!("state: {}", state.display());
    println!("source: {}", source.display());
    println!("workers: {workers}  poll_ms: {poll_ms}  lease_ttl_ms: {lease_ttl_ms}");
    if max_runtime_ms > 0 {
        println!("max_runtime_ms: {max_runtime_ms}");
    } else {
        println!("max_runtime_ms: 0 (run until Ctrl-C)");
    }

    // Register every declaration as a durable schedule manifest.
    // Upsert keyed on a stable id, so re-running the command after an
    // edit updates the cron/args in place and the fire cursor
    // (`last_checked_ms` / dedup table) carries over.
    for (index, schedule) in schedules.iter().enumerate() {
        let manifest = manifest_for(schedule, index)?;
        let stored = queue.upsert_schedule(manifest)?;
        println!(
            "schedule registered: {} — \"{}\" zone \"{}\" -> {}({} arg{})",
            stored.id,
            stored.cron,
            stored.zone,
            stored.task,
            schedule.args.len(),
            if schedule.args.len() == 1 { "" } else { "s" },
        );
    }

    let runtime_handle = Arc::new(Runtime::builder().build());
    let executor: Arc<dyn JobRuntimeExecutor> =
        Arc::new(DefaultJobRuntimeExecutor::new(Arc::new(ir)));
    let job_executor = into_pool_executor(executor, runtime_handle);

    let pool = WorkerPool::new(queue.clone(), workers)
        .with_executor(job_executor)
        .with_lease_ttl_ms(lease_ttl_ms)
        .with_idle_poll_ms(poll_ms.min(200).max(50));
    let drain = pool.drain_handle();
    let counters = pool.counters();
    let drain_for_signal = drain.clone();
    let drain_for_ticker = drain.clone();

    // Scheduler tick loop on its own OS thread (the queue is a
    // blocking SQLite surface). Each tick enqueues due fires through
    // the same recovery primitive `corvid jobs schedule recover`
    // uses — idempotent per (schedule, fire-time), missed-fire
    // policies honored.
    let ticker_queue = queue.clone();
    let ticker = std::thread::spawn(move || {
        while !drain_for_ticker.load(Ordering::SeqCst) {
            match ticker_queue.recover_schedules(max_missed_per_schedule) {
                Ok(report) => {
                    if report.enqueued > 0 {
                        println!(
                            "tick: enqueued {} due fire(s) across {} schedule(s)",
                            report.enqueued, report.scanned
                        );
                    }
                }
                Err(err) => eprintln!("scheduler tick failed: {err}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(poll_ms));
        }
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        tokio::spawn(async move {
            if (tokio::signal::ctrl_c().await).is_ok() {
                drain_for_signal.store(true, Ordering::SeqCst);
            }
        });
        let handles = pool.spawn();
        if max_runtime_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(max_runtime_ms)).await;
            drain.store(true, Ordering::SeqCst);
        }
        for h in handles {
            let _ = h.await;
        }
    });
    let _ = ticker.join();

    println!(
        "result: succeeded={} failed={} skipped={} total={}",
        counters.succeeded(),
        counters.failed(),
        counters.skipped(),
        counters.total()
    );
    Ok(0)
}

/// Build the durable-queue manifest for one language `schedule`
/// declaration. The manifest id is stable across runs (target name +
/// declaration position) so upsert updates in place.
fn manifest_for(schedule: &ScheduleDecl, index: usize) -> Result<QueueScheduleManifest> {
    let payload = serde_json::Value::Array(
        schedule
            .args
            .iter()
            .enumerate()
            .map(|(pos, arg)| literal_arg_to_json(schedule, pos, arg))
            .collect::<Result<Vec<_>>>()?,
    );
    let id = format!("sched_{}_{index}", schedule.target.name);
    let effect_summary = if schedule.effect_row.effects.is_empty() {
        None
    } else {
        Some(
            schedule
                .effect_row
                .effects
                .iter()
                .map(|effect| effect.name.name.clone())
                .collect::<Vec<_>>()
                .join(","),
        )
    };
    Ok(QueueScheduleManifest {
        id: id.clone(),
        cron: schedule.cron.clone(),
        zone: schedule.zone.clone(),
        task: schedule.target.name.clone(),
        payload,
        max_retries: 3,
        budget_usd: 0.0,
        effect_summary,
        replay_key_prefix: Some(id),
        missed_policy: ScheduleMissedPolicy::FireOnceOnRecovery,
        last_checked_ms: 0,
        last_fire_ms: None,
        created_ms: 0,
        updated_ms: 0,
    })
}

/// Schedule args must be literals: the manifest is a static, durable
/// artifact evaluated long after (and independently of) any program
/// run, so there is no frame to evaluate an arbitrary expression in.
/// Computation belongs inside the target agent.
fn literal_arg_to_json(
    schedule: &ScheduleDecl,
    position: usize,
    arg: &Expr,
) -> Result<serde_json::Value> {
    let Expr::Literal { value, .. } = arg else {
        anyhow::bail!(
            "schedule \"{}\" -> {}(...): argument {} is not a literal.\n\
             Schedule arguments become a durable manifest evaluated at fire time, \
             so they must be Int/Float/String/Bool literals — move computation \
             into the target agent's body.",
            schedule.cron,
            schedule.target.name,
            position + 1,
        );
    };
    Ok(match value {
        Literal::Int(n) => serde_json::json!(n),
        Literal::Float(f) => serde_json::json!(f),
        Literal::String(s) => serde_json::json!(s),
        Literal::Bool(b) => serde_json::json!(b),
        Literal::Nothing => serde_json::Value::Null,
    })
}
