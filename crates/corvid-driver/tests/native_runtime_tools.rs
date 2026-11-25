//! The standard library's runtime tools in native code (native dispatch
//! N2): the time tools, the random draws, the UUID tools and
//! `secret_read`. Native code calls them through `corvid_runtime_call`,
//! which runs `Runtime::call_tool` as the interpreter does.
//!
//! Every tool `is_native_runtime_tool` names is called here. A tool with
//! a fixed answer gives the same value on both tiers. A clock read, a
//! UUID or a draw differs per run, so for those both tiers must record
//! the same calls, and replaying a recording must give back exactly the
//! recorded values without reading the clock or drawing again.

use std::path::{Path, PathBuf};
use std::process::Command;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, native_ability, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_trace_schema::read_events_from_path;
use corvid_vm::Value;

const SECRET_ENV: &str = "CORVID_TEST_NATIVE_RUNTIME_TOOLS_SECRET";
const SECRET_TEXT: &str = "s3cret-n2-value";

/// The tools whose answer does not depend on the run.
const FIXED_TOOLS: [&str; 4] = ["time_parse_iso", "time_format_iso", "random_int", "secret_read"];

/// Every tool `is_native_runtime_tool` names, so a tool added there
/// without a call here fails `every_native_runtime_tool_is_covered`.
const TOOLS: [&str; 10] = [
    "time_now",
    "time_now_utc",
    "time_monotonic_ms",
    "time_parse_iso",
    "time_format_iso",
    "random_float",
    "random_int",
    "uuid_v4",
    "uuid_v7",
    "secret_read",
];

const PROGRAM: &str = r#"
import "./std/time" use time_now, time_now_utc, time_monotonic_ms, time_parse_iso, time_format_iso
import "./std/random" use random_float, random_int
import "./std/uuid" use uuid_v4, uuid_v7
import "./std/secrets" use secret_read

agent parsed(text: String) -> Int:
    return match time_parse_iso(text):
        Ok(ms) -> ms
        Err(e) -> 0

agent secret_text(name: String) -> String:
    return match secret_read(name):
        Ok(envelope) -> envelope.value
        Err(e) -> "error: " + e

agent main() -> String:
    fixed = [parsed("2026-01-01T00:00:00Z"), random_int(7, 7)].to_json() + " " + time_format_iso(0) + " " + secret_text("CORVID_TEST_NATIVE_RUNTIME_TOOLS_SECRET")
    drawn = uuid_v4() + " " + uuid_v7() + " " + time_now().to_string() + " " + time_now_utc().iso + " " + [time_monotonic_ms()].to_json() + " " + [random_float()].to_json()
    return fixed + " | " + drawn
"#;

struct Project {
    _dir: tempfile::TempDir,
    source: PathBuf,
    ir: corvid_ir::IrFile,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn project() -> Project {
    std::env::set_var(SECRET_ENV, SECRET_TEXT);
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    copy_dir(&corvid_test_paths::workspace_root().join("std"), &src.join("std"));
    let source = src.join("main.cor");
    std::fs::write(&source, PROGRAM).unwrap();
    let ir = compile_to_ir_with_config_at_path(PROGRAM, &source, None).expect("program compiles");
    Project { _dir: dir, source, ir }
}

/// Links against the release `corvid-test-tools` staticlib, which bundles
/// the current `corvid-runtime`, as the codegen native tests do.
fn link_current_runtime() {
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "corvid-test-tools", "--release"])
            .current_dir(corvid_test_paths::workspace_root())
            .status()
            .expect("build corvid-test-tools");
        assert!(status.success(), "building corvid-test-tools failed");
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    });
}

fn native_binary(project: &Project) -> PathBuf {
    native_ability(&project.ir).expect("the program compiles to native code");
    link_current_runtime();
    build_or_get_cached_native(&project.source, PROGRAM, &project.ir, None).expect("native build").path
}

fn run_native(binary: &Path, env: &[(&str, &Path)]) -> String {
    let mut command = Command::new(binary);
    command.env(SECRET_ENV, SECRET_TEXT).env_remove("CORVID_TRACE_PATH").env_remove("CORVID_REPLAY_TRACE_PATH");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run native binary");
    assert!(
        output.status.success(),
        "native run failed: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

async fn interpreter_record(project: &Project, trace_dir: &Path) -> (String, PathBuf) {
    let runtime = Runtime::builder().trace_to(trace_dir).build();
    let value = run_ir_with_runtime(&project.ir, None, vec![], &runtime).await.expect("interpreter run");
    let Value::String(text) = value else { panic!("expected a String, got {value:?}") };
    (text.to_string(), runtime.tracer().path().to_path_buf())
}

async fn interpreter_replay(project: &Project, trace: &Path) -> String {
    let runtime = Runtime::builder().replay_from(trace).build();
    let value = run_ir_with_runtime(&project.ir, None, vec![], &runtime).await.expect("interpreter replay");
    let Value::String(text) = value else { panic!("expected a String, got {value:?}") };
    text.to_string()
}

/// The tool events of a trace, with times and the run id removed, and
/// each result kept only for a tool whose answer is fixed.
fn tool_events(trace: &Path) -> Vec<serde_json::Value> {
    read_events_from_path(trace)
        .expect("trace reads")
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .filter(|event| matches!(event["kind"].as_str(), Some("tool_call" | "tool_result")))
        .map(|mut event| {
            let fields = event.as_object_mut().unwrap();
            fields.remove("ts_ms");
            fields.remove("run_id");
            let fixed = FIXED_TOOLS.contains(&fields["tool"].as_str().unwrap());
            if fields["kind"] == "tool_result" && !fixed {
                fields.insert("result".into(), serde_json::json!("<differs per run>"));
            }
            event
        })
        .collect()
}

fn fixed_part(output: &str) -> &str {
    output.split(" | ").next().unwrap()
}

#[test]
fn every_native_runtime_tool_is_covered() {
    for name in TOOLS {
        assert!(corvid_runtime::runtime::is_native_runtime_tool(name), "`{name}` is not a native runtime tool");
        assert!(PROGRAM.contains(&format!("{name}(")), "the program does not call `{name}`");
    }
    for name in ["secret", "random_token", "hash_password", "io_read_text", "rag_search", "mcp_call", "charge"] {
        assert!(!corvid_runtime::runtime::is_native_runtime_tool(name), "`{name}` is not part of N2");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_tools_give_the_same_values_and_record_the_same_calls_on_both_tiers() {
    let project = project();
    let traces = tempfile::tempdir().unwrap();

    let (interpreted, interpreter_trace) = interpreter_record(&project, traces.path()).await;
    let binary = native_binary(&project);
    let native_trace = traces.path().join("native.jsonl");
    let native = run_native(&binary, &[("CORVID_TRACE_PATH", &native_trace)]);

    let expected_fixed = format!("[1767225600000,7] 1970-01-01T00:00:00.000Z {SECRET_TEXT}");
    assert_eq!(fixed_part(&interpreted), expected_fixed, "{interpreted}");
    assert_eq!(fixed_part(&native), fixed_part(&interpreted), "native: {native}\ninterpreter: {interpreted}");
    let drawn = native.split(" | ").nth(1).unwrap();
    assert_eq!(drawn.split(' ').count(), interpreted.split(" | ").nth(1).unwrap().split(' ').count(), "{native}");

    let native_events = tool_events(&native_trace);
    assert_eq!(native_events, tool_events(&interpreter_trace), "both tiers record the same calls");
    let called: Vec<&str> = native_events
        .iter()
        .filter(|e| e["kind"] == "tool_call")
        .map(|e| e["tool"].as_str().unwrap())
        .collect();
    for name in TOOLS {
        assert!(called.contains(&name), "`{name}` was called: {called:?}");
    }
    // `secret_read` returns the text as a `String`, so the program's own
    // result holds it; the record of the call does not.
    let recorded_calls = serde_json::to_string(&native_events).unwrap();
    assert!(!recorded_calls.contains(SECRET_TEXT), "the recorded calls never hold the secret's text");
    assert!(recorded_calls.contains("<redacted"), "secret_read's recorded result is redacted: {recorded_calls}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_tier_replays_its_clock_reads_ids_and_draws_exactly() {
    let project = project();
    let traces = tempfile::tempdir().unwrap();
    let binary = native_binary(&project);

    let native_trace = traces.path().join("native.jsonl");
    let recorded = run_native(&binary, &[("CORVID_TRACE_PATH", &native_trace)]);
    let replayed = run_native(&binary, &[("CORVID_REPLAY_TRACE_PATH", &native_trace)]);
    assert_eq!(replayed, recorded, "native replay gives back the recorded values");

    let (interpreted, interpreter_trace) = interpreter_record(&project, traces.path()).await;
    assert_eq!(interpreter_replay(&project, &interpreter_trace).await, interpreted);
}
