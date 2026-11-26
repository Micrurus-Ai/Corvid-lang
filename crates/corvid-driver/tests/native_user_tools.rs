//! A tool of the program's own runs through the runtime call in native
//! code (native scalar boundaries B2b): its record, `Option`, sum-type
//! and `Decimal` arguments cross in the public JSON form on both tiers,
//! its answer is decoded by the declared type, and both tiers record the
//! same call and return the same value.

use std::process::Command;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_trace_schema::{read_events_from_path, TraceEvent};

const PROGRAM: &str = r#"
type Severity:
    | Low
    | High(reason: String)

type Ticket:
    title: String
    owner: Option<String>
    severity: Severity

type Assignment:
    owner: Option<String>
    severity: Severity
    budget: Decimal
    title: String

tool assign_ticket(ticket: Ticket, budget: Decimal) -> Assignment

agent main() -> String:
    assignment = assign_ticket(Ticket("Login fails", None, High("outage")), 12.50)
    return assignment.to_json()
"#;

fn tool_call_args(trace: &std::path::Path) -> Vec<serde_json::Value> {
    read_events_from_path(trace)
        .unwrap()
        .into_iter()
        .find_map(|event| match event {
            TraceEvent::ToolCall { args, .. } => Some(args),
            _ => None,
        })
        .expect("the tool call is recorded")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_structured_user_tool_runs_the_same_on_both_tiers() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("main.cor");
    std::fs::write(&source, PROGRAM).unwrap();
    let ir = compile_to_ir_with_config_at_path(PROGRAM, &source, None).expect("program compiles");

    // The interpreter, with the same implementation the tools library has.
    let traces = tempfile::tempdir().unwrap();
    let runtime = Runtime::builder()
        .tool("assign_ticket", |args| async move {
            let ticket = &args[0];
            Ok(serde_json::json!({
                "owner": ticket["owner"],
                "severity": ticket["severity"],
                "budget": args[1],
                "title": ticket["title"],
            }))
        })
        .trace_to(traces.path())
        .build();
    let interpreted = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("interpreter run");
    let corvid_driver::Value::String(interpreted) = interpreted else { panic!("a String, got {interpreted:?}") };
    let expected_args = vec![
        serde_json::json!({
            "title": "Login fails",
            "owner": null,
            "severity": {"tag": "variant", "type": "Severity", "variant": "High", "fields": ["outage"]}
        }),
        serde_json::json!("12.50"),
    ];
    assert_eq!(tool_call_args(runtime.tracer().path()), expected_args);

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .unwrap();
    assert!(status.success());
    std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    let binary = build_or_get_cached_native(&source, PROGRAM, &ir, Some(&corvid_test_paths::test_tools_staticlib()))
        .expect("native build")
        .path;
    let native_trace = traces.path().join("native.jsonl");
    let output = Command::new(&binary).env("CORVID_TRACE_PATH", &native_trace).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let native = String::from_utf8_lossy(&output.stdout).trim_end().to_string();

    assert_eq!(native, interpreted.as_ref(), "both tiers read the answer into the same value");
    assert_eq!(
        interpreted.as_ref(),
        r#"{"budget":"12.50","owner":null,"severity":{"fields":["outage"],"tag":"variant","type":"Severity","variant":"High"},"title":"Login fails"}"#
    );
    assert_eq!(tool_call_args(&native_trace), expected_args, "both tiers record the same arguments");
}
