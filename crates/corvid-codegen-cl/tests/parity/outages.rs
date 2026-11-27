//! `try ... else` catching outages in native code (recoverable failures
//! TE-3). Every program runs on the interpreter and as a compiled binary,
//! and the binary must release every allocation, the outage path
//! included.
//!
//! The failing tool is `echo_string` declared to return `Int`. Natively,
//! the test tools library answers it with text, which does not decode:
//! an unusable reply, an outage. On the interpreter its handler fails
//! with `ToolFailed`, also an outage.

use super::{assert_no_leaks, entry_name, ir_of, run_with_leak_detector, test_tools_lib_path};
use corvid_codegen_cl::build_native_to_disk;
use corvid_runtime::{ProgrammaticApprover, Runtime, RuntimeError};
use std::sync::Arc;

const TOOLS: &str = "tool echo_string(s: String) -> Int\ntool greet_string(name: String) -> String\ntool grounded_echo(s: String) -> Grounded<Int>\n\n";

fn interpret(ir: &corvid_ir::IrFile) -> Result<corvid_vm::Value, corvid_vm::InterpError> {
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("echo_string", |_args| async move {
            Err(RuntimeError::ToolFailed { tool: "echo_string".into(), message: "the lookup service is down".into() })
        })
        .tool("grounded_echo", |_args| async move {
            Err(RuntimeError::ToolFailed { tool: "grounded_echo".into(), message: "the source is down".into() })
        })
        .tool("greet_string", |args| async move {
            Ok(serde_json::json!(format!("hi {}", args[0].as_str().unwrap_or_default())))
        })
        .build();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(ir, entry_name(ir), vec![], &runtime).await })
}

fn compile(ir: &corvid_ir::IrFile, dir: &tempfile::TempDir) -> std::path::PathBuf {
    build_native_to_disk(ir, "corvid_parity_test", &dir.path().join("prog"), &[test_tools_lib_path().as_path()])
        .expect("compile + link")
}

/// Both tiers print the same text, and the binary leaks nothing.
#[track_caller]
fn assert_parity(body: &str) -> String {
    let src = format!("{TOOLS}{body}");
    let ir = ir_of(&src);
    let value = interpret(&ir).expect("interpreter run");
    let agent = ir.agents.iter().find(|agent| agent.name == entry_name(&ir)).expect("entry agent");
    let interpreted = corvid_vm::printed_result(&ir, agent, &value);
    let tmp = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr, status) = run_with_leak_detector(&compile(&ir, &tmp));
    assert!(status.success(), "compiled binary failed: {status:?} stderr={stderr} src=\n{src}");
    let compiled = stdout.trim_end_matches(['\r', '\n']).to_string();
    assert_eq!(compiled, interpreted, "the tiers disagree; src=\n{src}");
    assert_no_leaks(&stderr, &src);
    compiled
}

#[test]
fn an_outage_two_agent_calls_deep_takes_the_fallback_and_releases_what_was_held() {
    let out = assert_parity(
        r#"
agent broken(name: String) -> Int:
    held = "held " + name
    n = echo_string(held)
    return n + held.length()

agent cover(name: String) -> String:
    prefix = "cover-" + name
    return prefix + (broken(name) + 1).to_string()

agent main() -> String:
    a = try cover("x") else "fallback"
    b = try greet_string("ok") else "unused"
    c = try broken("y") on error retry 3 times backoff linear 1 else -1
    return a + " | " + b + " | " + c.to_string()
"#,
    );
    assert_eq!(out, "fallback | hi ok | -1");
}

#[test]
fn a_fallback_can_call_a_tool_and_an_outage_inside_an_expression_releases_its_operands() {
    let out = assert_parity(
        r#"
agent broken(name: String) -> Int:
    return echo_string(name)

agent main() -> String:
    label = "total: " + (try (broken("a") + broken("b")).to_string() else greet_string("fallback"))
    nested = try (try broken("c") else 5) + broken("d") else 7
    return label + " " + nested.to_string()
"#,
    );
    assert_eq!(out, "total: hi fallback 7");
}

#[test]
fn a_try_whose_body_cannot_fail_returns_its_value() {
    let out = assert_parity(
        r#"
agent main() -> String:
    greeting = try greet_string("there") else "unreachable"
    return greeting + "!"
"#,
    );
    assert_eq!(out, "hi there!");
}

#[test]
fn an_outage_nothing_catches_stops_the_program_with_the_failing_call_named() {
    let src = format!(
        "{TOOLS}{}",
        r#"
agent broken(name: String) -> Int:
    return echo_string(name)

agent main() -> Int:
    caught = try broken("a") else 1
    return caught + broken("b")
"#
    );
    let ir = ir_of(&src);
    let error = interpret(&ir).expect_err("the interpreter stops on the uncaught outage");
    assert!(error.is_outage(), "{error}");
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = crate::native_command(compile(&ir, &tmp)).output().expect("run compiled binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr={stderr}");
    assert!(output.stdout.is_empty(), "nothing is printed: {}", String::from_utf8_lossy(&output.stdout));
    assert!(stderr.contains("corvid: `echo_string`:"), "the failing call is named: {stderr}");
}

#[test]
fn a_try_that_reaches_a_failing_call_in_a_match_arm_is_refused_by_name() {
    let src = format!(
        "{TOOLS}{}",
        r#"
agent pick(name: Option<String>) -> Int:
    return match name:
        Some(n) -> echo_string(n)
        None -> 0

agent main() -> Int:
    return try pick(Some("x")) else 3
"#
    );
    let ir = ir_of(&src);
    let tmp = tempfile::tempdir().expect("tempdir");
    let error = build_native_to_disk(&ir, "corvid_parity_test", &tmp.path().join("prog"), &[test_tools_lib_path().as_path()])
        .expect_err("native code refuses this try");
    let message = error.to_string();
    assert!(
        message.contains("runs on the interpreter") && message.contains("`pick`") && message.contains("match"),
        "{message}"
    );
}

#[test]
fn a_grounded_answer_that_fails_takes_the_fallback_without_attesting_a_value() {
    let out = assert_parity(
        r#"
agent sourced(name: String) -> Int:
    doc = grounded_echo(name)
    return doc.unwrap_discarding_sources() + 1

agent main() -> String:
    return (try sourced("a") else 9).to_string()
"#,
    );
    assert_eq!(out, "9");
}

