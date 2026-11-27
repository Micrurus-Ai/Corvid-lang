use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use corvid_codegen_cl::build_native_to_disk;
use corvid_ir::lower;
use corvid_resolve::resolve;
use corvid_runtime::{llm::mock::MockAdapter, ProgrammaticApprover, Runtime, TraceEvent};
use corvid_syntax::{lex, parse_file};
use corvid_trace_schema::{read_events_from_path, serialize_event_line, write_events_to_path};
use corvid_types::typecheck;
use corvid_vm::{run_agent, Value};
use serde_json::json;

const REPLAY_NATIVE_SRC: &str = r#"
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
    corvid_test_paths::linked_test_tools_staticlib()
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
    let ir = ir_of(REPLAY_NATIVE_SRC);
    let result = run_agent(&ir, entry_name(&ir), vec![], &runtime)
        .await
        .expect("interpreter run");
    let trace_path = runtime.tracer().path().to_path_buf();
    (result, trace_path)
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
            model,
            model_version,
            rendered,
            args,
            ..
        } => TraceEvent::LlmCall {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            prompt: prompt.clone(),
            model: model.clone(),
            model_version: model_version.clone(),
            rendered: rendered.clone(),
            args: args.clone(),
        sampling: None,
        },
        TraceEvent::LlmResult {
            prompt,
            model,
            model_version,
            result,
            ..
        } => TraceEvent::LlmResult {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            prompt: prompt.clone(),
            model: model.clone(),
            model_version: model_version.clone(),
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
            token_id: "normalized-token".into(),
            label: label.clone(),
            args: args.clone(),
            scope: scope.clone(),
            issued_at_ms: 0,
            expires_at_ms: 0,
        },
        TraceEvent::HostEvent { name, payload, .. } => TraceEvent::HostEvent {
            ts_ms: 0,
            run_id: "normalized-run".into(),
            name: name.clone(),
            payload: payload.clone(),
        },
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
        other => panic!("unexpected event in replay_native fixture: {other:?}"),
    }
}

fn normalize_trace(path: &Path) -> Vec<String> {
    read_events_from_path(path)
        .expect("read trace")
        .iter()
        // Drop the host-side `llm.usage` HostEvent: it carries
        // adapter-name-specific telemetry (`env-mock-llm` on the
        // record run vs none on the replay run) that is orthogonal
        // to the record/replay determinism this test is asserting.
        .filter(|event| {
            !matches!(event, TraceEvent::HostEvent { name, .. } if name == "llm.usage")
        })
        .map(normalize_event)
        .map(|event| serialize_event_line(&event).expect("serialize normalized event"))
        .collect()
}

fn run_native_binary(
    bin: &Path,
    trace_path: &Path,
    replay_trace: Option<&Path>,
) -> std::process::Output {
    let mut cmd = Command::new(bin);
    cmd.env("CORVID_TRACE_PATH", trace_path)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", "{\"decide_refund\":true}")
        .env("CORVID_APPROVE_AUTO", "1")
        .env("CORVID_TEST_TOOL_ANSWER", "42")
        .env("CORVID_ROLLOUT_SEED", "12345");
    if let Some(replay_trace) = replay_trace {
        cmd.env("CORVID_REPLAY_TRACE_PATH", replay_trace);
    }
    cmd.output().expect("run native binary")
}

fn build_native_refund_bot(ir: &corvid_ir::IrFile, bin_path: &Path) -> PathBuf {
    build_native_to_disk(
        ir,
        "corvid_replay_native",
        bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("compile native binary")
}

#[test]
fn native_record_then_replay_matches_output_and_trace_shape() {
    let ir = ir_of(REPLAY_NATIVE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("refund_native");
    let produced = build_native_refund_bot(&ir, &bin_path);

    let recorded_trace = tmp.path().join("recorded.jsonl");
    let recorded = run_native_binary(&produced, &recorded_trace, None);
    assert!(
        recorded.status.success(),
        "native record run failed: status={:?}\nstdout={}\nstderr={}",
        recorded.status.code(),
        String::from_utf8_lossy(&recorded.stdout),
        String::from_utf8_lossy(&recorded.stderr)
    );

    let replay_trace = tmp.path().join("replayed.jsonl");
    let replayed = run_native_binary(&produced, &replay_trace, Some(&recorded_trace));
    assert!(
        replayed.status.success(),
        "native replay run failed: status={:?}\nstdout={}\nstderr={}",
        replayed.status.code(),
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&replayed.stderr)
    );

    assert_eq!(recorded.stdout, replayed.stdout);
    assert_eq!(normalize_trace(&recorded_trace), normalize_trace(&replay_trace));
}

#[test]
fn native_replay_divergence_reports_mutated_step() {
    let ir = ir_of(REPLAY_NATIVE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("refund_native");
    let produced = build_native_refund_bot(&ir, &bin_path);

    let recorded_trace = tmp.path().join("recorded.jsonl");
    let recorded = run_native_binary(&produced, &recorded_trace, None);
    assert!(recorded.status.success(), "record run should succeed");

    let mut events = read_events_from_path(&recorded_trace).expect("read recorded trace");
    let tool_step = events
        .iter()
        .position(|event| matches!(event, TraceEvent::ToolCall { .. }))
        .expect("recorded trace should contain a tool call");
    events[tool_step] = match &events[tool_step] {
        TraceEvent::ToolCall {
            ts_ms,
            run_id,
            tool,
            args,
        } => TraceEvent::ToolCall {
            ts_ms: *ts_ms,
            run_id: run_id.clone(),
            tool: format!("{tool}_mutated"),
            args: args.clone(),
        },
        other => panic!("expected tool call, got {other:?}"),
    };
    let mutated_trace = tmp.path().join("mutated.jsonl");
    write_events_to_path(&mutated_trace, &events).expect("write mutated trace");

    let replay_trace = tmp.path().join("mutated-replay.jsonl");
    let replayed = run_native_binary(&produced, &replay_trace, Some(&mutated_trace));
    assert!(
        !replayed.status.success(),
        "mutated replay should fail\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&replayed.stdout),
        String::from_utf8_lossy(&replayed.stderr)
    );
    let stderr = String::from_utf8_lossy(&replayed.stderr);
    assert!(stderr.contains("replay divergence at step"));
    assert!(stderr.contains(&tool_step.to_string()));
}

#[test]
fn native_replay_empty_and_malformed_traces_fail_cleanly() {
    let ir = ir_of(REPLAY_NATIVE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("refund_native");
    let produced = build_native_refund_bot(&ir, &bin_path);

    let empty_trace = tmp.path().join("empty.jsonl");
    std::fs::write(&empty_trace, "").unwrap();
    let empty_replay_trace = tmp.path().join("empty-replay.jsonl");
    let empty = run_native_binary(&produced, &empty_replay_trace, Some(&empty_trace));
    assert!(!empty.status.success(), "empty replay should fail");
    assert!(
        String::from_utf8_lossy(&empty.stderr).contains("failed to load replay trace"),
        "stderr should mention trace load failure: {}",
        String::from_utf8_lossy(&empty.stderr)
    );

    let malformed_trace = tmp.path().join("malformed.jsonl");
    std::fs::write(&malformed_trace, "{not-json}\n").unwrap();
    let malformed_replay_trace = tmp.path().join("malformed-replay.jsonl");
    let malformed = run_native_binary(&produced, &malformed_replay_trace, Some(&malformed_trace));
    assert!(!malformed.status.success(), "malformed replay should fail");
    assert!(
        String::from_utf8_lossy(&malformed.stderr).contains("failed to load replay trace"),
        "stderr should mention malformed replay trace: {}",
        String::from_utf8_lossy(&malformed.stderr)
    );
}

/// A native binary replays an interpreter recording (native dispatch
/// N5) and gives its recorded result.
#[tokio::test]
async fn native_replay_replays_an_interpreter_trace() {
    let trace_dir = tempfile::tempdir().unwrap();
    let (interp_result, interpreter_trace) = run_interpreter(trace_dir.path()).await;
    assert_eq!(interp_result, Value::Bool(true));

    let ir = ir_of(REPLAY_NATIVE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("refund_native");
    let produced = build_native_refund_bot(&ir, &bin_path);
    let native_replay_trace = tmp.path().join("native-replay.jsonl");
    let replayed = run_native_binary(&produced, &native_replay_trace, Some(&interpreter_trace));
    assert!(
        replayed.status.success(),
        "native replay of an interpreter recording should succeed: {}",
        String::from_utf8_lossy(&replayed.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&replayed.stdout).trim(), "true");
}

/// A program that reads zone rules natively. The tool call makes the
/// binary start the runtime, which records and replays the trace.
const ZONE_RULES_SRC: &str = r#"
tool answer() -> Int

agent opening_hour() -> Int:
    start = Instant.from_epoch_milliseconds(answer() * 42245828571)
    local = start.in_zone("Europe/Berlin")
    return local.hour() * 100 + local.add_days(1).hour()
"#;

#[test]
fn native_replay_warns_when_the_trace_used_another_tz_database() {
    let ir = ir_of(ZONE_RULES_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let produced = build_native_to_disk(&ir, "corvid_zone_rules_replay", &tmp.path().join("zone_rules"), &[test_tools_lib_path().as_path()])
        .expect("compile native binary");

    let recorded_trace = tmp.path().join("recorded.jsonl");
    let recorded = run_native_binary(&produced, &recorded_trace, None);
    assert!(recorded.status.success(), "record run failed: {}", String::from_utf8_lossy(&recorded.stderr));
    let mut events = read_events_from_path(&recorded_trace).expect("read recorded trace");
    assert!(
        matches!(&events[0], TraceEvent::SchemaHeader { tzdb_version: Some(v), .. } if v == corvid_time::TZDB_VERSION),
        "a native trace records its tz database: {:?}",
        events[0]
    );

    // The same database: replay says nothing about zones.
    let same = run_native_binary(&produced, &tmp.path().join("same-replay.jsonl"), Some(&recorded_trace));
    assert!(same.status.success(), "replay failed: {}", String::from_utf8_lossy(&same.stderr));
    assert_eq!(same.stdout, recorded.stdout);
    assert!(!String::from_utf8_lossy(&same.stderr).contains("tz database"), "{}", String::from_utf8_lossy(&same.stderr));

    // Another database: each zone-dependent operation warns once.
    if let TraceEvent::SchemaHeader { tzdb_version, .. } = &mut events[0] {
        *tzdb_version = Some("1999z".into());
    }
    let older_trace = tmp.path().join("older.jsonl");
    write_events_to_path(&older_trace, &events).expect("write trace");
    let replayed = run_native_binary(&produced, &tmp.path().join("older-replay.jsonl"), Some(&older_trace));
    let stderr = String::from_utf8_lossy(&replayed.stderr);
    assert!(replayed.status.success(), "replay failed: {stderr}");
    assert_eq!(replayed.stdout, recorded.stdout);
    for operation in ["`hour` in Europe/Berlin", "`add_days` in Europe/Berlin"] {
        assert_eq!(stderr.matches(operation).count(), 1, "{operation} warns once: {stderr}");
    }
    assert!(stderr.contains("recorded with tz database 1999z"), "{stderr}");
}

/// A caught outage replays to the same fallback (recoverable failures
/// TE-3d). The tool is declared to answer `Int`; natively the test tools
/// library answers it with text, which does not decode, and on the
/// interpreter its handler fails. Either way it is an outage, recorded
/// with its `outage` flag.
const CAUGHT_OUTAGE_SRC: &str = r#"
tool echo_string(s: String) -> Int

agent lookup(name: String) -> Int:
    return echo_string(name) + 1

agent main() -> Int:
    return try lookup("a") else 7
"#;

fn run_caught_outage(bin: &Path, trace_path: &Path, replay_trace: Option<&Path>) -> std::process::Output {
    let mut cmd = Command::new(bin);
    cmd.env("CORVID_TRACE_PATH", trace_path);
    if let Some(replay_trace) = replay_trace {
        cmd.env("CORVID_REPLAY_TRACE_PATH", replay_trace);
    }
    cmd.output().expect("run native binary")
}

#[test]
fn a_native_run_that_caught_an_outage_replays_to_the_same_fallback() {
    let ir = ir_of(CAUGHT_OUTAGE_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let produced = build_native_to_disk(&ir, "corvid_replay_native", &tmp.path().join("caught"), &[test_tools_lib_path().as_path()])
        .expect("compile native binary");
    let recorded_trace = tmp.path().join("recorded.jsonl");
    let recorded = run_caught_outage(&produced, &recorded_trace, None);
    assert!(recorded.status.success(), "record run: {}", String::from_utf8_lossy(&recorded.stderr));
    assert_eq!(String::from_utf8_lossy(&recorded.stdout).trim(), "7");

    let replay_trace = tmp.path().join("replayed.jsonl");
    let replayed = run_caught_outage(&produced, &replay_trace, Some(&recorded_trace));
    assert!(replayed.status.success(), "replay run: {}", String::from_utf8_lossy(&replayed.stderr));
    assert_eq!(recorded.stdout, replayed.stdout);
    assert_eq!(normalize_trace(&recorded_trace), normalize_trace(&replay_trace));
}

/// The tool succeeds when the binary runs live, so only the recorded
/// failure can send the replay to the fallback: it is served from the
/// interpreter's trace, never called.
const REPLAYED_FAILURE_SRC: &str = r#"
tool greet_string(name: String) -> String

agent main() -> String:
    return try greet_string("a") else "fallback"
"#;

#[tokio::test]
async fn a_native_replay_of_an_interpreter_run_takes_the_fallback_it_took() {
    let trace_dir = tempfile::tempdir().unwrap();
    let runtime = Runtime::builder()
        .tool("greet_string", |_args| async move {
            Err(corvid_runtime::RuntimeError::ToolFailed { tool: "greet_string".into(), message: "the service is down".into() })
        })
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .trace_to(trace_dir.path())
        .build();
    let ir = ir_of(REPLAYED_FAILURE_SRC);
    let result = run_agent(&ir, entry_name(&ir), vec![], &runtime).await.expect("interpreter run");
    assert_eq!(result, Value::String("fallback".into()));
    let interpreter_trace = runtime.tracer().path().to_path_buf();

    let tmp = tempfile::tempdir().unwrap();
    let produced = build_native_to_disk(&ir, "corvid_replay_native", &tmp.path().join("replayed"), &[test_tools_lib_path().as_path()])
        .expect("compile native binary");
    let live = run_caught_outage(&produced, &tmp.path().join("live.jsonl"), None);
    assert_eq!(String::from_utf8_lossy(&live.stdout).trim(), "hi a", "live, the tool answers: {}", String::from_utf8_lossy(&live.stderr));
    let replayed = run_caught_outage(&produced, &tmp.path().join("native-replay.jsonl"), Some(&interpreter_trace));
    assert!(replayed.status.success(), "native replay: {}", String::from_utf8_lossy(&replayed.stderr));
    assert_eq!(String::from_utf8_lossy(&replayed.stdout).trim(), "fallback");
}
