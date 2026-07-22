//! `parallel:` block tests (slice 46e).
//!
//! The load-bearing proof: a concurrent run records an ARM-ORDERED
//! trace (buffers flush at the join in arm order, so the trace is
//! indistinguishable from sequential arm-order execution), and a
//! Substitute-mode replay of that trace reproduces the identical
//! result while consuming events with the ordinary sequential
//! cursor — zero schema changes, zero new matching rules.

use super::*;
use corvid_runtime::tracing::Tracer;
use corvid_runtime::{MockAdapter, RuntimeError};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};

fn tool_boom(msg: &str) -> RuntimeError {
    RuntimeError::ToolFailed {
        tool: "boom".into(),
        message: msg.into(),
    }
}

/// Read the per-arm outcome tags recorded by the `parallel.outcomes`
/// host event (slice 52d-2) from a trace file: `(arm_name, outcome,
/// crossed_irreversible)` in arm order.
fn parallel_outcomes(trace_path: &std::path::Path) -> Vec<(String, String, bool)> {
    let text = std::fs::read_to_string(trace_path).expect("trace readable");
    for line in text.lines() {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("name").and_then(|n| n.as_str()) != Some("parallel.outcomes") {
            continue;
        }
        let arms = v["payload"]["arms"].as_array().expect("arms array");
        return arms
            .iter()
            .map(|a| {
                (
                    a["name"].as_str().unwrap_or("").to_string(),
                    a["outcome"].as_str().unwrap_or("").to_string(),
                    a["crossed_irreversible"].as_bool().unwrap_or(false),
                )
            })
            .collect();
    }
    panic!("no parallel.outcomes event in trace");
}

const PARALLEL_SRC: &str = "\
public effect llm_call:
    cost: $0.01
    reversible: true

prompt ask_weather(city: String) -> String uses llm_call:
    \"Weather in {city}?\"

prompt ask_news(city: String) -> String uses llm_call:
    \"News in {city}?\"

agent main(city: String) -> String:
    parallel:
        weather = ask_weather(city)
        news = ask_news(city)
    return weather + \"|\" + news
";

fn mock() -> MockAdapter {
    MockAdapter::new("mock-1")
        .reply("ask_weather", serde_json::json!("sunny"))
        .reply("ask_news", serde_json::json!("quiet"))
}

#[tokio::test]
async fn parallel_arms_join_and_bind() {
    let ir = ir_of(PARALLEL_SRC);
    let rt = Runtime::builder()
        .llm(Arc::new(mock()))
        .default_model("mock-1")
        .build();
    let out = run_agent(&ir, "main", vec![Value::String(Arc::from("Nairobi"))], &rt)
        .await
        .expect("parallel run");
    let Value::String(s) = out else {
        panic!("expected String, got {out:?}");
    };
    assert_eq!(&*s, "sunny|quiet");
}

#[tokio::test]
async fn parallel_trace_is_arm_ordered_and_replays_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace_path = dir.path().join("parallel.jsonl");

    // 1. RECORD: concurrent arms, arm-ordered flush.
    let ir = ir_of(PARALLEL_SRC);
    {
        let tracer = Tracer::open_path(&trace_path, "r-parallel");
        let rt = Runtime::builder()
            .llm(Arc::new(mock()))
            .default_model("mock-1")
            .tracer(tracer)
            .build();
        let out = run_agent(&ir, "main", vec![Value::String(Arc::from("Nairobi"))], &rt)
            .await
            .expect("recorded run");
        let Value::String(s) = out else {
            panic!("expected String");
        };
        assert_eq!(&*s, "sunny|quiet");
    }

    // 2. The trace's llm events appear IN ARM ORDER regardless of
    //    completion order: ask_weather strictly before ask_news.
    let text = std::fs::read_to_string(&trace_path).expect("trace readable");
    let weather_pos = text.find("ask_weather").expect("weather event recorded");
    let news_pos = text.find("ask_news").expect("news event recorded");
    assert!(
        weather_pos < news_pos,
        "arm buffers must flush in arm order"
    );

    // 3. REPLAY: Substitute mode reproduces the identical result
    //    through the ordinary sequential cursor.
    let replay_rt = Runtime::builder()
        .llm(Arc::new(mock()))
        .default_model("mock-1")
        .replay_from(&trace_path)
        .build();
    let out = run_agent(
        &ir,
        "main",
        vec![Value::String(Arc::from("Nairobi"))],
        &replay_rt,
    )
    .await
    .expect("replayed run");
    let Value::String(s) = out else {
        panic!("expected String");
    };
    assert_eq!(&*s, "sunny|quiet", "replay must reproduce the join");
}

// ---- Reversibility-guarded cancellation (slice 52d-2) ----------------

const CANCEL_SRC: &str = "\
effect risky:
    cost: $0.0
    trust: autonomous
    reversible: false

effect safe:
    cost: $0.0
    trust: autonomous

tool commit_write() -> Bool uses risky
tool boom() -> Bool uses safe
tool after_commit() -> Bool uses safe
tool tick() -> Bool uses safe

agent arm_committed() -> Bool:
    x = commit_write()
    y = after_commit()
    return y

agent arm_loop() -> Bool:
    result = true
    for i in range(0, 2000):
        result = tick()
    return result

agent worker_rule() -> Bool:
    parallel:
        a = boom_wait()
        b = arm_committed()
    return b

agent worker_cancel() -> Bool:
    parallel:
        a = boom()
        b = arm_loop()
    return b

tool boom_wait() -> Bool uses safe
";

/// THE RULE (deterministic): an arm that has crossed a non-reversible
/// effect boundary runs to completion even when a sibling fails fast.
/// `boom_wait` (arm `a`) blocks until arm `b` has dispatched its
/// irreversible `commit_write`, then fails — so `b` is provably past
/// the boundary when the failure fires. `b` must still complete.
#[tokio::test]
async fn arm_past_irreversible_boundary_is_not_cancelled() {
    let ir = ir_of(CANCEL_SRC);
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("rule.jsonl");

    let committed = Arc::new(tokio::sync::Notify::new());
    let committed_signal = committed.clone();
    let committed_wait = committed.clone();
    let after_ran = Arc::new(AtomicBool::new(false));
    let after_ran_h = after_ran.clone();

    let rt = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "r-rule"))
        // Arm b: commit_write crosses the irreversible boundary, then
        // signals arm a that it may fail.
        .tool("commit_write", move |_| {
            let signal = committed_signal.clone();
            async move {
                signal.notify_one();
                Ok(json!(true))
            }
        })
        // Arm b's second call — proves b kept running past the boundary.
        .tool("after_commit", move |_| {
            let after = after_ran_h.clone();
            async move {
                after.store(true, Ordering::SeqCst);
                Ok(json!(true))
            }
        })
        // Arm a: wait until b has committed, THEN fail.
        .tool("boom_wait", move |_| {
            let wait = committed_wait.clone();
            async move {
                wait.notified().await;
                Err(tool_boom("boom after b committed"))
            }
        })
        .build();

    // The block errors (arm a failed), but arm b must have completed.
    let out = run_agent(&ir, "worker_rule", vec![], &rt).await;
    assert!(out.is_err(), "block errors because arm a failed: {out:?}");
    assert!(
        after_ran.load(Ordering::SeqCst),
        "arm b must run PAST its irreversible boundary to completion"
    );

    let outcomes = parallel_outcomes(&trace_path);
    let b = outcomes
        .iter()
        .find(|(name, ..)| name == "b")
        .expect("arm b recorded");
    assert_eq!(b.1, "completed", "the crossed arm must complete: {outcomes:?}");
    assert!(b.2, "arm b must be marked as having crossed the boundary");
    let a = outcomes.iter().find(|(name, ..)| name == "a").unwrap();
    assert_eq!(a.1, "errored", "arm a is the failing arm: {outcomes:?}");
}

/// A REVERSIBLE in-flight arm IS cancelled when a sibling fails fast.
/// Arm `a` fails immediately; arm `b` loops calling a reversible tool
/// that yields each iteration, so the scheduler sets the cancel flag
/// and `b` stops cooperatively at its next tool dispatch — long before
/// its 2000 iterations finish.
#[tokio::test]
async fn reversible_arm_is_cancelled_after_a_sibling_fails() {
    let ir = ir_of(CANCEL_SRC);
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("cancel.jsonl");

    let rt = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "r-cancel"))
        .tool("boom", |_| async move {
            Err(tool_boom("boom immediately"))
        })
        .tool("tick", |_| async move {
            tokio::task::yield_now().await;
            Ok(json!(true))
        })
        .build();

    let out = run_agent(&ir, "worker_cancel", vec![], &rt).await;
    assert!(out.is_err(), "block errors because arm a failed");

    let outcomes = parallel_outcomes(&trace_path);
    let b = outcomes.iter().find(|(name, ..)| name == "b").unwrap();
    assert_eq!(
        b.1, "cancelled",
        "the reversible looping arm must be cancelled, not run to completion: {outcomes:?}"
    );
    assert!(!b.2, "a cancelled reversible arm never crossed the boundary");
}

/// 52d-3 acceptance test (IGNORED until 52d-3 lands): replaying a
/// recorded cancelling run must REPRODUCE the recorded cancellation
/// deterministically — arm `a` errors, arm `b` stops at its recorded
/// point (cancelled) — instead of diverging.
///
/// Empirically today (post-52d-2) replay of a cancelling run DIVERGES:
/// `ReplayDivergence { step: 0, expected: RunCompleted{error}, got:
/// tool_result boom }`. The live cancelling run is timing-dependent
/// (arm `b` runs some number of ticks before stopping), so re-running
/// it concurrently under Substitute mode consumes the recorded cursor
/// in a different order / for a different arm-event count. 52d-3 makes
/// the `parallel` block, on replay, READ the recorded per-arm outcomes
/// and reproduce them: a `cancelled` arm replays to its recorded event
/// count and stops (returning the cancellation sentinel) instead of
/// running live and diverging.
#[tokio::test]
#[ignore = "52d-3: replay reproduction of parallel cancellation not yet implemented"]
async fn replay_reproduces_a_recorded_cancellation() {
    let ir = ir_of(CANCEL_SRC);
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("cancel_replay.jsonl");

    // RECORD a cancelling run.
    {
        let rt = Runtime::builder()
            .tracer(Tracer::open_path(&trace_path, "r-cancel-replay"))
            .tool("boom", |_| async move { Err(tool_boom("boom")) })
            .tool("tick", |_| async move {
                tokio::task::yield_now().await;
                Ok(json!(true))
            })
            .build();
        let out = run_agent(&ir, "worker_cancel", vec![], &rt).await;
        assert!(out.is_err(), "recorded run errors (arm a failed)");
    }

    // REPLAY must reproduce, not diverge.
    let replay_rt = Runtime::builder().replay_from(&trace_path).build();
    let out = run_agent(&ir, "worker_cancel", vec![], &replay_rt).await;
    assert!(
        out.is_err(),
        "replay must reproduce the recorded error, not diverge: {out:?}"
    );
}
