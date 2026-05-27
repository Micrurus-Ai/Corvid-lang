//! Cross-surface replay-quarantine integration corpus.
//!
//! Slice `35V2-P38-C-6` — closes the audit-correction track
//! `35V2-P38-C-replay-quarantine` and promotes the registry row
//! `jobs.replayable_side_effects` from `OutOfScope` to
//! `RuntimeChecked`. Tests are referenced by the registry row so
//! `corvid-guarantees`'s cross-reference sentinel keeps them
//! reachable.
//!
//! Layout — one positive + one adversarial per surface where the
//! surface has a read/write distinction (store + IO), one
//! adversarial-only per surface where every call is side-effecting
//! (LLM + HTTP), plus two negative controls that prove the wrap
//! does NOT fire in differential-replay mode or in live (non-replay)
//! mode. Eight tests total.

use corvid_runtime::approvals::ProgrammaticApprover;
use corvid_runtime::errors::RuntimeError;
use corvid_runtime::http::HttpRequest;
use corvid_runtime::llm::{mock::MockAdapter, LlmRequestRef};
use corvid_runtime::store::StoreKind;
use corvid_runtime::Runtime;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Write a minimal valid JSONL trace (`SchemaHeader` + `RunStarted`
/// + `RunCompleted`) into `dir`. Returns the file path. Used as the
/// `replay_from` source for every test that needs a Substitute-mode
/// runtime — the trace contents are irrelevant for the quarantine
/// tests; only the schema validity matters.
fn write_minimal_trace(dir: &Path) -> PathBuf {
    let path = dir.join("trace.jsonl");
    // `ReplaySource::load` requires (in order): a `schema_header`
    // matching the replay writer (`corvid-vm`), a
    // `seed_read{purpose:"rollout_default_seed"}` so the runtime can
    // recover its deterministic PRNG seed, and at least one
    // executable event (`run_started` here). Everything else is
    // optional; trace content is irrelevant for the quarantine tests
    // — only schema validity matters.
    let contents = concat!(
        r#"{"kind":"schema_header","version":2,"writer":"corvid-vm","ts_ms":0,"run_id":"r"}"#,
        "\n",
        r#"{"kind":"seed_read","ts_ms":1,"run_id":"r","purpose":"rollout_default_seed","value":12345}"#,
        "\n",
        r#"{"kind":"run_started","ts_ms":2,"run_id":"r","agent":"noop","args":[]}"#,
        "\n",
        r#"{"kind":"run_completed","ts_ms":3,"run_id":"r","ok":true,"result":"ok"}"#,
        "\n",
    );
    std::fs::write(&path, contents).unwrap();
    path
}

fn build_substitute_replay_runtime(trace_path: &Path) -> Runtime {
    Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .replay_from(trace_path)
        .build()
}

/// Slice 35V2-P38-C-6 adversarial: a runtime in Substitute-mode
/// replay refuses direct LLM registry calls with
/// `QuarantineViolation { surface: "llm", .. }`. The recorded
/// substitution path (`Runtime::call_llm`) is unchanged; only the
/// registry-layer bypass is blocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_quarantines_llm_registry_direct_calls() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = build_substitute_replay_runtime(&trace_path);
    assert!(runtime.is_replay_mode());
    assert!(!runtime.replay_uses_live_llm());

    let req = LlmRequestRef {
        prompt: "p",
        model: "mock-1",
        rendered: "Hello",
        args: &[],
        output_schema: None,
    };
    let err = runtime.llms().call(&req).await.expect_err("must refuse");
    match err {
        RuntimeError::QuarantineViolation { surface, .. } => assert_eq!(surface, "llm"),
        other => panic!("expected llm QuarantineViolation, got {other:?}"),
    }
}

/// Slice 35V2-P38-C-6 adversarial: HTTP send during replay fails
/// closed with `QuarantineViolation { surface: "http", .. }`.
/// The wrap names the method + URL in the detail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_quarantines_http_client_send() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = build_substitute_replay_runtime(&trace_path);

    let req = HttpRequest::get("https://example.invalid/should-not-reach");
    let err = runtime.http().send(&req).await.expect_err("must refuse");
    match err {
        RuntimeError::QuarantineViolation { surface, detail } => {
            assert_eq!(surface, "http");
            assert!(
                detail.contains("example.invalid/should-not-reach"),
                "detail must name URL: {detail}"
            );
        }
        other => panic!("expected http QuarantineViolation, got {other:?}"),
    }
}

/// Slice 35V2-P38-C-6 adversarial: store writes during replay
/// refuse with `QuarantineViolation { surface: "store", .. }`.
#[test]
fn replay_quarantines_store_writes() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = build_substitute_replay_runtime(&trace_path);
    let err = runtime
        .stores()
        .put(StoreKind::Session, "S", "k", serde_json::json!({"x": 1}))
        .expect_err("write must refuse");
    match err {
        RuntimeError::QuarantineViolation { surface, detail } => {
            assert_eq!(surface, "store");
            assert!(detail.contains("session"), "detail must name kind: {detail}");
        }
        other => panic!("expected store QuarantineViolation, got {other:?}"),
    }
}

/// Slice 35V2-P38-C-6 positive: store reads pass through during
/// replay. The runtime's `StoreManager` starts empty (per
/// `StoreManager::default`), so the get returns `None` — but the
/// CALL succeeds, proving reads aren't quarantined.
#[test]
fn replay_passes_through_store_reads() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = build_substitute_replay_runtime(&trace_path);
    let value = runtime
        .stores()
        .get(StoreKind::Session, "S", "missing-key")
        .expect("read passes through during replay");
    assert_eq!(value, None);
}

/// Slice 35V2-P38-C-6 adversarial: file writes during replay
/// refuse with `QuarantineViolation { surface: "io", .. }`. The
/// quarantine fires BEFORE the filesystem is touched — the test
/// asserts the file does not appear on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_quarantines_io_writes() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = build_substitute_replay_runtime(&trace_path);

    let new_path = dir.path().join("should-not-write.txt");
    let err = runtime
        .io()
        .write_text(&new_path, "blocked")
        .await
        .expect_err("write must refuse");
    match err {
        RuntimeError::QuarantineViolation { surface, .. } => assert_eq!(surface, "io"),
        other => panic!("expected io QuarantineViolation, got {other:?}"),
    }
    assert!(
        !new_path.exists(),
        "io quarantine must not touch the filesystem: {new_path:?}"
    );
}

/// Slice 35V2-P38-C-6 positive: file reads pass through during
/// replay. Seed a file before building the replay runtime, then
/// read it from inside replay — the read returns the seeded
/// contents without surfacing a quarantine error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_passes_through_io_reads() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let seed_path = dir.path().join("seed.txt");
    std::fs::write(&seed_path, "hello").unwrap();
    let runtime = build_substitute_replay_runtime(&trace_path);
    let read = runtime
        .io()
        .read_text(&seed_path)
        .await
        .expect("read passes through during replay");
    assert_eq!(read.contents, "hello");
}

/// Slice 35V2-P38-C-6 negative control: differential-replay mode
/// (`!source.uses_live_llm() == false`, i.e. `uses_live_llm == true`)
/// does NOT install any quarantine. The mock adapter responds to a
/// direct registry call instead of refusing. This locks the
/// "differential mode keeps live adapters / clients" contract so a
/// future change cannot accidentally flip the quarantine on in
/// differential and break live-LLM comparison.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn differential_replay_does_not_quarantine_llm_registry() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .differential_replay_from(&trace_path, "mock-1")
        .build();
    assert!(runtime.is_replay_mode());
    assert!(
        runtime.replay_uses_live_llm(),
        "differential replay must mark uses_live_llm = true"
    );
    assert!(!runtime.http().is_quarantined());
    assert!(!runtime.stores().is_write_quarantined());
    assert!(!runtime.io().is_write_quarantined());

    let req = LlmRequestRef {
        prompt: "p",
        model: "mock-1",
        rendered: "Hello",
        args: &[],
        output_schema: None,
    };
    let resp = runtime
        .llms()
        .call(&req)
        .await
        .expect("differential replay must reach the live adapter");
    assert_eq!(resp.value, serde_json::json!("ok"));
}

/// Slice 35V2-P38-C-6 negative control: a live (non-replay) runtime
/// installs NO quarantine on any surface. Locks the contract so the
/// quarantine cannot accidentally fire outside replay mode.
#[test]
fn live_mode_does_not_quarantine_any_surface() {
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .build();
    assert!(!runtime.is_replay_mode());
    assert!(!runtime.http().is_quarantined());
    assert!(!runtime.stores().is_write_quarantined());
    assert!(!runtime.io().is_write_quarantined());
}
