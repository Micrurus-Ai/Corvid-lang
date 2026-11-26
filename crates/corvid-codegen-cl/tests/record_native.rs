use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use corvid_codegen_cl::build_native_to_disk;
use corvid_ir::lower;
use corvid_resolve::resolve;
use corvid_runtime::{
    llm::mock::MockAdapter, ProgrammaticApprover, Runtime, TraceEvent,
};
use corvid_syntax::{lex, parse_file};
use corvid_trace_schema::{read_events_from_path, serialize_event_line, validate_supported_schema};
use corvid_types::typecheck;
use corvid_vm::{run_agent, Value};
use serde_json::json;

const RECORD_NATIVE_SRC: &str = r#"
tool answer() -> Int

prompt decide_refund(amount: Int) -> Bool:
    """Should refund {amount}?"""

agent refund_bot() -> Bool:
    amount = answer()
    approve IssueRefund(amount)
    return decide_refund(amount)
"#;

fn ir_of(src: &str) -> corvid_ir::IrFile {
    let tokens = lex(src).expect("lex");
    let (file, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let resolved = resolve(&file);
    assert!(resolved.errors.is_empty(), "resolve errors: {:?}", resolved.errors);
    let checked = typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "type errors: {:?}", checked.errors);
    lower(&file, &resolved, &checked)
}

fn entry_name(ir: &corvid_ir::IrFile) -> &str {
    if ir.agents.len() == 1 {
        &ir.agents[0].name
    } else {
        ir.agents
            .iter()
            .find(|agent| agent.name == "main")
            .map(|agent| agent.name.as_str())
            .expect("entry agent")
    }
}

fn test_tools_lib_path() -> PathBuf {
    let workspace_root = corvid_test_paths::workspace_root();
    let path = corvid_test_paths::test_tools_staticlib();
    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("corvid-test-tools")
        .arg("--release")
        .current_dir(&workspace_root)
        .status()
        .expect("build corvid-test-tools");
    assert!(status.success(), "building corvid-test-tools failed");
    // `corvid_test_tools.lib` already bundles `corvid-runtime` as a
    // transitive Rust dep (along with the C runtime objects) because
    // `corvid-test-tools` lists `corvid-runtime` as a workspace dep.
    // Routing the linker through this single Rust staticlib avoids
    // the duplicate-`std` LNK2005 that linking it alongside the
    // standalone `corvid_runtime.lib` produces on MSVC.
    unsafe {
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &path);
    }
    path
}

fn interpreter_runtime(trace_dir: &Path) -> Runtime {
    Runtime::builder()
        .tool("answer", |_args| async move { Ok(json!(42)) })
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(MockAdapter::new("mock-1").reply("decide_refund", json!(true))))
        .default_model("mock-1")
        .rollout_seed(12345)
        .trace_to(trace_dir)
        .build()
}

async fn run_interpreter(trace_dir: &Path) -> (Value, PathBuf) {
    let runtime = interpreter_runtime(trace_dir);
    let ir = ir_of(RECORD_NATIVE_SRC);
    let result = run_agent(&ir, entry_name(&ir), vec![], &runtime)
        .await
        .expect("interpreter run");
    let trace_path = runtime.tracer().path().to_path_buf();
    (result, trace_path)
}

fn run_native_binary(bin: &Path, trace_path: &Path) -> std::process::Output {
    Command::new(bin)
        .env("CORVID_TRACE_PATH", trace_path)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", "{\"decide_refund\":true}")
        .env("CORVID_APPROVE_AUTO", "1")
        .env("CORVID_TEST_TOOL_ANSWER", "42")
        .env("CORVID_ROLLOUT_SEED", "12345")
        .output()
        .expect("run native binary")
}

fn normalize_event(event: &TraceEvent) -> TraceEvent {
    match event {
        TraceEvent::SchemaHeader {
            version,
            commit_sha,
            source_path,
            ..
        } => TraceEvent::SchemaHeader {
            version: *version,
            writer: "normalized-writer".into(),
            commit_sha: commit_sha.clone(),
            source_path: source_path.clone(),
            tzdb_version: None,
            ts_ms: 0,
            run_id: "normalized-run".into(),
        },
        TraceEvent::RunStarted { agent, args, .. } => TraceEvent::RunStarted {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            agent: agent.clone(),
            args: args.clone(),
        },
        TraceEvent::RunCompleted {
            ok,
            result,
            error,
            ..
        } => TraceEvent::RunCompleted {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            ok: *ok,
            result: result.clone(),
            error: error.clone(),
        },
        TraceEvent::ToolCall { tool, args, .. } => TraceEvent::ToolCall {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            tool: tool.clone(),
            args: args.clone(),
        },
        TraceEvent::ToolResult { tool, result, .. } => TraceEvent::ToolResult {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            tool: tool.clone(),
            result: result.clone(),
        },
        TraceEvent::LlmCall {
            prompt,
            rendered,
            args,
            ..
        } => TraceEvent::LlmCall {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            prompt: prompt.clone(),
            model: None,
            model_version: None,
            rendered: rendered.clone(),
            args: args.clone(),
        sampling: None,
        },
        TraceEvent::LlmResult {
            prompt,
            result,
            ..
        } => TraceEvent::LlmResult {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            prompt: prompt.clone(),
            model: None,
            model_version: None,
            result: result.clone(),
            chunk_boundaries: None,
        },
        TraceEvent::ApprovalRequest { label, args, .. } => TraceEvent::ApprovalRequest {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            label: label.clone(),
            args: args.clone(),
        },
        TraceEvent::ApprovalResponse {
            label, approved, ..
        } => TraceEvent::ApprovalResponse {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            label: label.clone(),
            approved: *approved,
        },
        TraceEvent::ApprovalDecision {
            site,
            args,
            accepted,
            decider,
            rationale,
            ..
        } => TraceEvent::ApprovalDecision {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            site: site.clone(),
            args: args.clone(),
            accepted: *accepted,
            decider: decider.clone(),
            rationale: rationale.clone(),
        },
        TraceEvent::ApprovalTokenIssued {
            label, args, scope, ..
        } => TraceEvent::ApprovalTokenIssued {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            // The token id and timestamps are non-deterministic;
            // collapse to canonical placeholders so two runs of
            // the same program compare equal under normalization.
            token_id: "normalized-token".into(),
            label: label.clone(),
            args: args.clone(),
            scope: scope.clone(),
            issued_at_ms: 0,
            expires_at_ms: 0,
        },
        TraceEvent::HostEvent { name, payload, .. } => {
            // The fixture gives each tier its own mock: a `MockAdapter`
            // named `mock-1` in the interpreter, the environment mock in
            // the native binary. Everything else in the usage record is
            // compared.
            let mut payload = payload.clone();
            if name == "llm.usage" {
                payload["adapter"] = json!("normalized-adapter");
            }
            TraceEvent::HostEvent {
                ts_ms: 0,
                run_id: "normalized-run".into(),
                name: name.clone(),
                payload,
            }
        }
        TraceEvent::SeedRead { purpose, value, .. } => TraceEvent::SeedRead {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            purpose: purpose.clone(),
            value: *value,
        },
        TraceEvent::ClockRead { source, value, .. } => TraceEvent::ClockRead {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            source: source.clone(),
            value: *value,
        },
        other => panic!("unexpected event in record_native fixture: {other:?}"),
    }
}

#[tokio::test]
async fn refund_bot_native_trace_matches_interpreter_shape() {
    let ir = ir_of(RECORD_NATIVE_SRC);

    let interp_trace_dir = tempfile::tempdir().unwrap();
    let (interp_result, interp_trace_path) = run_interpreter(interp_trace_dir.path()).await;
    assert_eq!(interp_result, Value::Bool(true));

    let native_tmp = tempfile::tempdir().unwrap();
    let bin_path = native_tmp.path().join("refund_native");
    let produced = build_native_to_disk(
        &ir,
        "corvid_record_native",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");
    let native_trace_path = native_tmp.path().join("native-trace.jsonl");
    let output = run_native_binary(&produced, &native_trace_path);
    assert!(
        output.status.success(),
        "native run failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let interp_events = read_events_from_path(&interp_trace_path).expect("read interpreter trace");
    validate_supported_schema(&interp_events).expect("validate interpreter trace");
    let native_events = read_events_from_path(&native_trace_path).expect("read native trace");
    validate_supported_schema(&native_events).expect("validate native trace");

    assert!(matches!(native_events.first(), Some(TraceEvent::SchemaHeader { .. })));
    assert!(matches!(native_events.last(), Some(TraceEvent::RunCompleted { .. })));
    assert!(
        native_events
            .iter()
            .any(|event| matches!(event, TraceEvent::ToolCall { .. })),
        "native trace should include ToolCall"
    );
    assert!(
        native_events
            .iter()
            .any(|event| matches!(event, TraceEvent::LlmCall { .. })),
        "native trace should include LlmCall"
    );
    assert!(
        native_events
            .iter()
            .any(|event| matches!(event, TraceEvent::ApprovalRequest { .. })),
        "native trace should include ApprovalRequest"
    );
    assert!(
        native_events.iter().any(
            |event| matches!(event, TraceEvent::SeedRead { purpose, .. } if purpose == "rollout_default_seed")
        ),
        "native trace should include rollout_default_seed"
    );

    // The whole trace is compared, host events included: both tiers
    // record the `llm.usage` record from the runtime's one LLM call path.
    let normalized_interp = interp_events
        .iter()
        .map(normalize_event)
        .map(|event| serialize_event_line(&event).expect("serialize interp event"))
        .collect::<Vec<_>>();
    let normalized_native = native_events
        .iter()
        .map(normalize_event)
        .map(|event| serialize_event_line(&event).expect("serialize native event"))
        .collect::<Vec<_>>();
    assert_eq!(normalized_native, normalized_interp);
}

#[test]
#[ignore = "manual acceptance measurement for 21-B-rec-native"]
fn native_recording_overhead_smoke() {
    let ir = ir_of(RECORD_NATIVE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("refund_native");
    let produced = build_native_to_disk(
        &ir,
        "corvid_record_native",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    let run_once = |trace_path: Option<&Path>| {
        let start = std::time::Instant::now();
        for idx in 0..10 {
            let path = trace_path.map(|base| base.join(format!("trace-{idx}.jsonl")));
            let mut cmd = Command::new(&produced);
            cmd.env("CORVID_MODEL", "mock-1")
                .env("CORVID_TEST_MOCK_LLM", "1")
                .env("CORVID_TEST_MOCK_LLM_REPLIES", "{\"decide_refund\":true}")
                .env("CORVID_APPROVE_AUTO", "1")
                .env("CORVID_TEST_TOOL_ANSWER", "42")
                .env("CORVID_ROLLOUT_SEED", "12345");
            if let Some(path) = path.as_ref() {
                cmd.env("CORVID_TRACE_PATH", path);
            } else {
                cmd.env("CORVID_TRACE_DISABLE", "1");
            }
            let output = cmd.output().expect("run native binary");
            assert!(output.status.success(), "native run failed");
        }
        start.elapsed()
    };

    let unrecorded = run_once(None);
    let trace_dir = tempfile::tempdir().unwrap();
    let recorded = run_once(Some(trace_dir.path()));
    let delta = (recorded.as_secs_f64() / unrecorded.as_secs_f64()) - 1.0;
    eprintln!(
        "native recording overhead: unrecorded={:?} recorded={:?} delta={:.2}%",
        unrecorded,
        recorded,
        delta * 100.0
    );
}


/// A tool call, an approval and a prompt call replay on the other tier
/// (native dispatch N5): the interpreter replays a native recording and
/// the native binary replays the interpreter's, each giving the recorded
/// result.
#[tokio::test]
async fn refund_bot_replays_on_the_other_tier() {
    let ir = ir_of(RECORD_NATIVE_SRC);
    let native_tmp = tempfile::tempdir().unwrap();
    let produced = build_native_to_disk(
        &ir,
        "corvid_record_native_cross",
        &native_tmp.path().join("refund_native"),
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");

    let native_trace_path = native_tmp.path().join("native-trace.jsonl");
    assert!(run_native_binary(&produced, &native_trace_path).status.success());
    let replay = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .replay_from(&native_trace_path)
        .build();
    let replayed = run_agent(&ir, entry_name(&ir), vec![], &replay).await.expect("the interpreter replays a native recording");
    assert_eq!(replayed, Value::Bool(true));

    let interp_trace_dir = tempfile::tempdir().unwrap();
    let (_, interp_trace_path) = run_interpreter(interp_trace_dir.path()).await;
    let output = Command::new(&produced)
        .env("CORVID_REPLAY_TRACE_PATH", &interp_trace_path)
        .env("CORVID_APPROVE_AUTO", "1")
        .env("CORVID_TEST_TOOL_ANSWER", "42")
        .output()
        .expect("run native replay");
    assert!(
        output.status.success(),
        "native replay of an interpreter recording failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
}

const RECORD_RESULT_SRC: &str = r#"
type Refund:
    @json(key: "order-id")
    order_id: String
    amount: Int
    approved: Bool

agent refund_bot() -> Refund:
    refund = Refund("ord-7", 42, true)
    approve IssueRefund(refund)
    return refund
"#;

fn approval_and_result(events: &[TraceEvent]) -> (Vec<serde_json::Value>, Option<serde_json::Value>, Option<serde_json::Value>) {
    let approval = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::ApprovalRequest { args, .. } => Some(args.clone()),
            _ => None,
        })
        .expect("an approval request");
    let started = events.iter().find_map(|event| match event {
        TraceEvent::RunStarted { args, .. } => Some(serde_json::Value::Array(args.clone())),
        _ => None,
    });
    let result = events.iter().find_map(|event| match event {
        TraceEvent::RunCompleted { result, .. } => result.clone(),
        _ => None,
    });
    (approval, started, result)
}

/// A record argument to `approve` and a record result are recorded in the
/// public JSON form on both tiers: under the renamed key, the same values
/// the interpreter records.
#[tokio::test]
async fn a_record_result_and_approval_record_the_same_public_form_on_both_tiers() {
    let ir = ir_of(RECORD_RESULT_SRC);

    let interp_trace_dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .trace_to(interp_trace_dir.path())
        .build();
    run_agent(&ir, entry_name(&ir), vec![], &runtime).await.expect("interpreter run");
    let interp_events = read_events_from_path(runtime.tracer().path()).expect("read interpreter trace");

    let native_tmp = tempfile::tempdir().unwrap();
    let produced = build_native_to_disk(
        &ir,
        "corvid_record_result_native",
        &native_tmp.path().join("record_result_native"),
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary");
    let native_trace_path = native_tmp.path().join("native-trace.jsonl");
    let output = Command::new(&produced)
        .env("CORVID_TRACE_PATH", &native_trace_path)
        .env("CORVID_APPROVE_AUTO", "1")
        .output()
        .expect("run native binary");
    assert!(
        output.status.success(),
        "native run failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let native_events = read_events_from_path(&native_trace_path).expect("read native trace");

    let expected = json!({"order-id": "ord-7", "amount": 42, "approved": true});
    let (interp_approval, interp_started, interp_result) = approval_and_result(&interp_events);
    assert_eq!(interp_approval, vec![expected.clone()]);
    assert_eq!(interp_result, Some(expected));
    assert_eq!(approval_and_result(&native_events), (interp_approval, interp_started, interp_result));
}

