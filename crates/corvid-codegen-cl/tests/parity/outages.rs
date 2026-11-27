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
fn an_outage_inside_a_match_arm_unwinds_from_the_arm_and_releases_what_it_held() {
    // The first arm is a lone call, checked after the `match`; the second
    // does more around its call, so it is lifted into an agent of its own.
    let out = assert_parity(
        r#"
agent plain(name: Option<String>, prefix: String) -> Int:
    return match name:
        Some(n) -> echo_string(prefix)
        None -> 0

agent around(name: Option<String>, prefix: String) -> Int:
    return match name:
        Some(n) -> echo_string(prefix + n) + n.length()
        None -> 0

agent main() -> String:
    held = "held"
    a = try plain(Some("x"), held) else 1
    b = try around(Some("other"), held) else 2
    c = try around(None, held) else 3
    return held + " " + a.to_string() + " " + b.to_string() + " " + c.to_string()
"#,
    );
    assert_eq!(out, "held 1 2 0");
}

#[test]
fn a_try_that_reaches_a_failing_call_in_a_match_guard_is_refused_by_name() {
    let src = format!(
        "{TOOLS}{}",
        r#"
agent pick(name: Option<String>) -> Int:
    return match name:
        Some(n) if echo_string(n) > 0 -> 1
        _ -> 0

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
        message.contains("runs on the interpreter") && message.contains("`pick`") && message.contains("guard"),
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


/// A prompt whose reply never fits its declared type is an unusable
/// reply, an outage on both tiers (recoverable failures TE-3c): the mock
/// model answers text where an `Int` is declared, through every repair
/// attempt.
#[test]
fn a_prompt_whose_reply_never_fits_takes_the_fallback_on_both_tiers() {
    let src = r#"
prompt estimate(item: String) -> Int:
    "How many minutes does {item} take?"

agent plan(item: String) -> Int:
    held = "plan for " + item
    minutes = estimate(item)
    return minutes + held.length()

agent main() -> Int:
    return (try plan("laundry") else 15) + (try estimate("dishes") else 5)
"#;
    let ir = ir_of(src);
    let mock = corvid_runtime::MockAdapter::new("mock-1").reply("estimate", serde_json::json!("about an hour"));
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(mock))
        .default_model("mock-1")
        .build();
    let interpreted = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(&ir, entry_name(&ir), vec![], &runtime).await })
        .expect("interpreter run");
    assert_eq!(interpreted, corvid_vm::Value::Int(20));

    let tmp = tempfile::tempdir().expect("tempdir");
    let output = crate::native_command(compile(&ir, &tmp))
        .env("CORVID_DEBUG_ALLOC", "1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_RESPONSE", "about an hour")
        .env("CORVID_MODEL", "mock-1")
        .output()
        .expect("run compiled binary");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "compiled binary failed: {stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "20", "stderr={stderr}");
    assert_no_leaks(&stderr, src);
}

/// A Rust `#[tool]` that returns `Err` is a failed call, an outage, on
/// both tiers: natively the test tools library's `lookup_or_fail`
/// returns `Err` for an empty name, and on the interpreter its handler
/// fails the same way.
#[test]
fn a_rust_tool_that_returns_err_is_caught_and_one_that_returns_ok_is_not() {
    let src = r#"
tool lookup_or_fail(name: String) -> Int

agent main() -> String:
    found = try lookup_or_fail("abc") else -1
    missing = try lookup_or_fail("") else -1
    return found.to_string() + " " + missing.to_string()
"#;
    let ir = ir_of(src);
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("lookup_or_fail", |args| async move {
            let name = args[0].as_str().unwrap_or_default().to_string();
            if name.is_empty() {
                Err(RuntimeError::ToolFailed { tool: "lookup_or_fail".into(), message: "the lookup service is down".into() })
            } else {
                Ok(serde_json::json!(name.chars().count()))
            }
        })
        .build();
    let interpreted = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(&ir, entry_name(&ir), vec![], &runtime).await })
        .expect("interpreter run");
    assert_eq!(interpreted, corvid_vm::Value::String("3 -1".into()));
    let tmp = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr, status) = run_with_leak_detector(&compile(&ir, &tmp));
    assert!(status.success(), "compiled binary failed: {stderr}");
    assert_eq!(stdout.trim(), "3 -1", "stderr={stderr}");
    assert_no_leaks(&stderr, src);
}
