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

/// Slice 33S1b — the executing `io.write_text` tool dispatch
/// (33S1a's interception inside `Runtime::call_tool` for any
/// `io.*` name) is gated by the replay substitution path. In
/// substitute-mode replay, ANY `io.*` tool call goes through
/// `replay.replay_tool_call` before reaching the dispatch
/// interception — so the call either substitutes from the
/// recorded trace OR diverges (when the trace doesn't carry
/// the expected event), but never reaches the filesystem.
/// This fixture proves the dispatch path doesn't open a bypass:
/// the minimal trace has no recorded tool_call, so the executing
/// write returns ReplayDivergence and the file never appears
/// on disk. The IoRuntime write-quarantine guards the DIRECT
/// `runtime.io().write_text(...)` path (see
/// `replay_quarantines_io_writes` above) — together the two
/// fixtures cover both routes into IoRuntime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_blocks_executing_io_write_tool_dispatch_from_escaping_to_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let policy = corvid_runtime::IoToolPolicy::new(dir.path().to_str(), None);
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .replay_from(&trace_path)
        .io_policy(policy)
        .build();
    assert!(runtime.is_replay_mode());
    assert!(runtime.io().is_write_quarantined());

    let new_path = dir.path().join("dispatch-blocked.txt");
    let err = runtime
        .call_tool(
            "io_write_text",
            vec![
                serde_json::json!("dispatch-blocked.txt"),
                serde_json::json!("blocked"),
            ],
        )
        .await
        .expect_err("executing dispatch write must refuse during replay");
    // The dispatch reaches the replay-substitution path first;
    // with no recorded tool_call in the minimal trace the
    // substitution diverges. EITHER divergence OR quarantine is
    // acceptable here — both prove the call doesn't escape to
    // the filesystem, which is the load-bearing safety property.
    match err {
        RuntimeError::ReplayDivergence(_) => {}
        RuntimeError::QuarantineViolation { surface, .. } => assert_eq!(surface, "io"),
        other => panic!("expected ReplayDivergence or io QuarantineViolation, got {other:?}"),
    }
    assert!(
        !new_path.exists(),
        "executing io.write_text dispatch must not touch the filesystem during replay: {new_path:?}"
    );
}

/// Slice 33S1b — companion: executing `io.read_text` is ALSO
/// gated by the replay substitution path. With the minimal
/// trace lacking a recorded tool_call event, the call diverges
/// (no read happens). This proves reads ALSO don't open a
/// bypass — they're constrained by the same trace contract
/// as writes, just without the additional write-quarantine
/// layer that `replay_quarantines_io_writes` exercises.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_blocks_executing_io_read_tool_dispatch_without_recorded_event() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let seed_path = dir.path().join("seed-via-dispatch.txt");
    std::fs::write(&seed_path, "passthrough").unwrap();

    let policy = corvid_runtime::IoToolPolicy::new(dir.path().to_str(), None);
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .replay_from(&trace_path)
        .io_policy(policy)
        .build();
    assert!(runtime.is_replay_mode());

    let err = runtime
        .call_tool(
            "io_read_text",
            vec![serde_json::json!("seed-via-dispatch.txt")],
        )
        .await
        .expect_err("executing read must go through replay substitution, not the live FS");
    match err {
        RuntimeError::ReplayDivergence(_) => {}
        other => panic!("expected ReplayDivergence (no recorded io.read_text event), got {other:?}"),
    }
}

/// Slice 33S2b — the executing `http_post_json` tool dispatch
/// (33S2a's interception inside `Runtime::call_tool` for any
/// `http_*` name) is gated by the replay substitution path
/// AND by the `HttpClient::quarantine` flag the builder flips
/// on when entering Substitute-mode replay. Together these
/// form two independent layers: the trace-substitution path
/// runs FIRST (so even a hypothetical bypass of the policy
/// check would land on it), and the quarantine flag is the
/// floor underneath. This fixture proves the executing POST
/// path cannot reach the network during replay, regardless of
/// allowlist contents — the load-bearing safety property for
/// the executing HTTP-client surface's replay claim. Mirrors
/// `replay_blocks_executing_io_write_tool_dispatch_from_escaping_to_filesystem`
/// from 33S1b.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_blocks_executing_http_post_tool_dispatch_from_escaping_to_network() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    // Allowlist deliberately includes the host the test URL
    // names — to prove that even a fully-configured policy
    // cannot bypass the replay quarantine. The safety property
    // here is "replay refuses the network call regardless of
    // allowlist," not "the policy blocks every test URL."
    let policy = corvid_runtime::HttpEgressPolicy::new(Some(&[
        "api.example.com".to_string(),
    ]));
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .replay_from(&trace_path)
        .http_policy(policy)
        .build();
    assert!(runtime.is_replay_mode());
    assert!(runtime.http().is_quarantined());

    let err = runtime
        .call_tool(
            "http_post_json",
            vec![
                serde_json::json!("https://api.example.com/should-not-reach"),
                serde_json::json!(r#"{"payload":"blocked"}"#),
            ],
        )
        .await
        .expect_err("executing dispatch POST must refuse during replay");
    // The dispatch reaches the replay-substitution path first;
    // with no recorded tool_call in the minimal trace the
    // substitution diverges. EITHER divergence (no recorded
    // event) OR quarantine (HttpClient flag) is acceptable —
    // both prove the call doesn't escape to the network, which
    // is the load-bearing safety property.
    match err {
        RuntimeError::ReplayDivergence(_) => {}
        RuntimeError::QuarantineViolation { surface, .. } => assert_eq!(surface, "http"),
        other => panic!(
            "expected ReplayDivergence or http QuarantineViolation, got {other:?}"
        ),
    }
}

/// Slice 33S2b — companion: executing `http_get` is also
/// gated by the replay path. With the minimal trace lacking a
/// recorded tool_call event, the call diverges (no GET
/// happens). This proves reads (GETs) ALSO don't open a
/// network bypass during replay — same contract as POST,
/// without the additional `HttpClient::quarantine` test on
/// the response side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_blocks_executing_http_get_tool_dispatch_without_recorded_event() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = write_minimal_trace(dir.path());
    let policy = corvid_runtime::HttpEgressPolicy::new(Some(&[
        "api.example.com".to_string(),
    ]));
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(
            MockAdapter::new("mock-1").reply("p", serde_json::json!("ok")),
        ))
        .replay_from(&trace_path)
        .http_policy(policy)
        .build();
    assert!(runtime.is_replay_mode());

    let err = runtime
        .call_tool(
            "http_get",
            vec![serde_json::json!("https://api.example.com/probe")],
        )
        .await
        .expect_err("executing GET must go through replay substitution, not the live network");
    match err {
        RuntimeError::ReplayDivergence(_) => {}
        other => panic!("expected ReplayDivergence (no recorded http_get event), got {other:?}"),
    }
}
