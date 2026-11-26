//! Native prompts exchange the public JSON form with the model through
//! the plan the compiler embeds (native scalar boundaries B1b): a record
//! with an `Option` and a sum type, a list, a `Decimal` and a `Duration`
//! reach the model as the interpreter shows them, and a structured reply
//! is read the way the interpreter reads it. Both tiers record the same
//! prompt text, arguments and reply, and return the same value.

use std::process::Command;
use std::sync::Arc;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{MockAdapter, Runtime};
use corvid_trace_schema::{read_events_from_path, TraceEvent};

const PROGRAM: &str = r#"
type Severity:
    | Low
    | High(reason: String)

type Ticket:
    title: String
    owner: Option<String>
    severity: Severity

type Verdict:
    summary: String
    assignee: Option<String>
    severity: Severity
    refund: Decimal

prompt triage(ticket: Ticket, tags: List<String>, budget: Decimal, wait: Duration) -> Verdict:
    """Triage {ticket} tagged {tags} within {budget} and {wait}"""

agent main() -> String:
    verdict = triage(Ticket("Login fails", None, High("outage")), ["auth", "sso"], 12.50, 90 seconds)
    return verdict.to_json()
"#;

const REPLY: &str = r#"{"summary":"auth outage","assignee":null,"severity":{"tag":"variant","type":"Severity","variant":"High","fields":["outage"]},"refund":"12.50","note":"ignored"}"#;

/// What a run recorded about its one prompt call.
#[derive(Debug, PartialEq)]
struct Recorded {
    rendered: Option<String>,
    args: Vec<serde_json::Value>,
    result: serde_json::Value,
}

fn recorded(trace: &std::path::Path) -> Recorded {
    let events = read_events_from_path(trace).unwrap();
    let (rendered, args) = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::LlmCall { rendered, args, .. } => Some((rendered.clone(), args.clone())),
            _ => None,
        })
        .expect("the prompt call is recorded");
    let result = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::LlmResult { result, .. } => Some(result.clone()),
            _ => None,
        })
        .expect("the reply is recorded");
    Recorded { rendered, args, result }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn structured_prompts_read_the_same_on_both_tiers() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("main.cor");
    std::fs::write(&source, PROGRAM).unwrap();
    let ir = compile_to_ir_with_config_at_path(PROGRAM, &source, None).expect("program compiles");
    assert_eq!(corvid_driver::native_ability(&ir), Ok(()), "the program runs natively");

    let traces = tempfile::tempdir().unwrap();
    let reply: serde_json::Value = serde_json::from_str(REPLY).unwrap();
    let runtime = Runtime::builder()
        .llm(Arc::new(MockAdapter::new("mock-1").reply("triage", reply)))
        .default_model("mock-1")
        .trace_to(traces.path())
        .build();
    let interpreted = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("interpreter run");
    let corvid_driver::Value::String(interpreted) = interpreted else { panic!("a String, got {interpreted:?}") };
    let interpreted_record = recorded(runtime.tracer().path());
    assert_eq!(
        interpreted_record.rendered.as_deref(),
        Some(r#"Triage {"owner":null,"severity":{"fields":["outage"],"tag":"variant","type":"Severity","variant":"High"},"title":"Login fails"} tagged ["auth","sso"] within "12.50" and "PT1M30S""#)
    );

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .unwrap();
    assert!(status.success());
    std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    let binary = build_or_get_cached_native(&source, PROGRAM, &ir, None).expect("native build").path;
    let native_trace = traces.path().join("native.jsonl");
    let output = Command::new(&binary)
        .env("CORVID_TRACE_PATH", &native_trace)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", format!(r#"{{"triage": {REPLY}}}"#))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let native = String::from_utf8_lossy(&output.stdout).trim_end().to_string();

    assert_eq!(native, interpreted.as_ref(), "both tiers read the reply into the same value");
    assert_eq!(
        interpreted.as_ref(),
        r#"{"assignee":null,"refund":"12.50","severity":{"fields":["outage"],"tag":"variant","type":"Severity","variant":"High"},"summary":"auth outage"}"#
    );
    assert_eq!(recorded(&native_trace), interpreted_record, "both tiers record the same call");
}

/// A reply that does not fit the declared type is refused on both tiers
/// with the field it lacks: the interpreter's unusable-reply error, and
/// native code's stop after its retries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_missing_a_field_is_refused_on_both_tiers() {
    let program = "type Answer:\n    text: String\n    score: Int\n\nprompt grade(q: List<Int>) -> Answer:\n    \"\"\"Grade {q}\"\"\"\n\nagent main() -> String:\n    return grade([1, 2]).text\n";
    let reply = r#"{"text":"fine"}"#;
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("main.cor");
    std::fs::write(&source, program).unwrap();
    let ir = compile_to_ir_with_config_at_path(program, &source, None).expect("program compiles");

    let runtime = Runtime::builder()
        .llm(Arc::new(MockAdapter::new("mock-1").reply("grade", serde_json::from_str(reply).unwrap())))
        .default_model("mock-1")
        .build();
    let error = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect_err("the reply lacks `score`");
    assert!(error.to_string().contains("score"), "{error}");

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .unwrap();
    assert!(status.success());
    std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    let binary = build_or_get_cached_native(&source, program, &ir, None).expect("native build").path;
    let output = Command::new(&binary)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_REPLIES", format!(r#"{{"grade": {reply}}}"#))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the native run must stop");
    assert!(stderr.contains("does not decode") && stderr.contains("score"), "{stderr}");
}
