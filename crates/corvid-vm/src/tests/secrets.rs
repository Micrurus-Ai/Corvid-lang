//! `Secret` on the interpreter, run from Corvid source: a program reads
//! a secret from the environment, compares secrets in constant time and
//! passes them to tools, which receive the text; displaying a secret
//! shows `<secret>`, and every record of a tool call holds `<redacted>`.

use super::*;
use crate::conv::{json_to_value, value_to_json};

const SECRET_TOOL: &str = "\
effect secrets_read:
    reversible: true

tool secret(name: String) -> Result<Secret, String> uses secrets_read
";

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn traced_runtime(dir: &std::path::Path, run_id: &str, tools: impl FnOnce(corvid_runtime::RuntimeBuilder) -> corvid_runtime::RuntimeBuilder) -> Runtime {
    tools(
        Runtime::builder()
            .approver(Arc::new(ProgrammaticApprover::always_yes()))
            .tracer(corvid_runtime::Tracer::open(dir, run_id)),
    )
    .build()
}

#[tokio::test]
async fn a_program_reads_and_compares_secrets() {
    std::env::set_var("CORVID_VM_SECRET_A", "correct horse battery staple");
    std::env::set_var("CORVID_VM_SECRET_B", "correct horse battery staple");
    std::env::set_var("CORVID_VM_SECRET_C", "correct horse battery stapler");
    let src = format!(
        "{SECRET_TOOL}
agent main() -> Result<List<Bool>, String>:
    a = secret(\"CORVID_VM_SECRET_A\")?
    b = secret(\"CORVID_VM_SECRET_B\")?
    c = secret(\"CORVID_VM_SECRET_C\")?
    return Ok([a == b, a != c, a == c, [a, b].contains(c)])
"
    );
    let ir = ir_of(&src);
    let result = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run");
    let Value::ResultOk(list) = result else { panic!("expected Ok, got {result:?}") };
    let Value::List(items) = list.get() else { panic!("expected a list") };
    assert_eq!(items.iter_cloned(), vec![Value::Bool(true), Value::Bool(true), Value::Bool(false), Value::Bool(false)]);
}

#[tokio::test]
async fn a_missing_secret_is_an_err() {
    std::env::remove_var("CORVID_VM_SECRET_MISSING");
    let src = format!(
        "{SECRET_TOOL}
agent main() -> Bool:
    return match secret(\"CORVID_VM_SECRET_MISSING\"):
        Ok(key) -> true
        Err(problem) -> false
"
    );
    let ir = ir_of(&src);
    assert_eq!(run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run"), Value::Bool(false));
}

#[tokio::test]
async fn a_secret_displays_as_secret_and_serializes_as_redacted() {
    std::env::set_var("CORVID_VM_SECRET_SHOWN", "sk_live_do_not_print");
    let src = format!(
        "{SECRET_TOOL}
agent load() -> Result<Secret, String>:
    return secret(\"CORVID_VM_SECRET_SHOWN\")
"
    );
    let ir = ir_of(&src);
    let result = run_agent(&ir, "load", vec![], &empty_runtime()).await.expect("run");
    let shown = format!("{result}");
    let debugged = format!("{result:?}");
    let rendered = crate::repl_display::render_value(&result);
    for form in [&shown, &debugged, &rendered] {
        assert!(form.contains("<secret>"), "{form}");
        assert!(!form.contains("sk_live"), "{form}");
    }
    assert_eq!(value_to_json(&result), json!({"tag": "ok", "ok": "<redacted>"}));
}

#[tokio::test]
async fn a_tool_receives_the_text_and_its_trace_holds_redacted() {
    std::env::set_var("CORVID_VM_WEBHOOK_KEY", "whsec_vm_9f8e7d6c5b4a");
    let src = format!(
        "{SECRET_TOOL}
type Credentials:
    user: String
    password: Secret

tool sign(key: Secret, body: String) -> String
tool log_in(credentials: Credentials) -> Bool

agent main() -> Result<Bool, String>:
    key = secret(\"CORVID_VM_WEBHOOK_KEY\")?
    signature = sign(key, \"payload\")
    return Ok(log_in(Credentials(\"ada\", key)) and signature == \"signed\")
"
    );
    let ir = ir_of(&src);
    let dir = tempfile::tempdir().unwrap();
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (sign_seen, log_in_seen) = (received.clone(), received.clone());
    let rt = traced_runtime(dir.path(), "secret-arguments", |builder| {
        builder
            .tool("sign", move |args| {
                let seen = sign_seen.clone();
                async move {
                    seen.lock().unwrap().push(args);
                    Ok(json!("signed"))
                }
            })
            .tool("log_in", move |args| {
                let seen = log_in_seen.clone();
                async move {
                    seen.lock().unwrap().push(args);
                    Ok(json!(true))
                }
            })
    });
    let result = run_agent(&ir, "main", vec![], &rt).await.expect("run");
    assert_eq!(result, Value::ResultOk(crate::value::BoxedValue::new(Value::Bool(true))));
    assert_eq!(
        received.lock().unwrap().as_slice(),
        [
            vec![json!("whsec_vm_9f8e7d6c5b4a"), json!("payload")],
            vec![json!({"user": "ada", "password": "whsec_vm_9f8e7d6c5b4a"})],
        ]
    );
    let trace = std::fs::read_to_string(dir.path().join("secret-arguments.jsonl")).unwrap();
    assert!(!trace.contains("whsec_vm"), "the trace holds the secret:\n{trace}");
    assert!(trace.contains(r#""args":["<redacted>","payload"]"#), "{trace}");
    assert!(trace.contains(r#""password":"<redacted>""#), "{trace}");
}

#[tokio::test]
async fn a_tool_that_echoes_a_secret_is_scrubbed_only_in_the_trace() {
    std::env::set_var("CORVID_VM_ECHOED_KEY", "tok_echo_31415");
    let src = format!(
        "{SECRET_TOOL}
tool echo(key: Secret) -> String
tool fail(key: Secret) -> String

agent echoed() -> Result<String, String>:
    key = secret(\"CORVID_VM_ECHOED_KEY\")?
    return Ok(echo(key))

agent failed() -> Result<String, String>:
    key = secret(\"CORVID_VM_ECHOED_KEY\")?
    return Ok(fail(key))
"
    );
    let ir = ir_of(&src);
    let dir = tempfile::tempdir().unwrap();
    let rt = traced_runtime(dir.path(), "secret-echo", |builder| {
        builder
            .tool("echo", |args| async move { Ok(json!(format!("you sent {}", args[0].as_str().unwrap()))) })
            .tool("fail", |args| async move {
                Err(RuntimeError::ToolFailed { tool: "fail".into(), message: format!("bad key {}", args[0].as_str().unwrap()) })
            })
    });
    // The program receives the tool's real result and error, unchanged.
    let echoed = run_agent(&ir, "echoed", vec![], &rt).await.expect("run");
    assert_eq!(echoed, Value::ResultOk(crate::value::BoxedValue::new(text("you sent tok_echo_31415"))));
    let error = run_agent(&ir, "failed", vec![], &rt).await.expect_err("the tool fails").to_string();
    assert!(error.contains("bad key tok_echo_31415"), "{error}");
    // The records of the tool calls hold `<redacted>`, marked as
    // scrubbed, and never the text. (The program's copy is an ordinary
    // `String` once the tool returns it, so the run's own recorded result
    // holds whatever the agent returns.)
    let trace = std::fs::read_to_string(dir.path().join("secret-echo.jsonl")).unwrap();
    let tool_records: Vec<&str> =
        trace.lines().filter(|line| line.contains(r#""kind":"tool_call""#) || line.contains(r#""kind":"tool_result""#)).collect();
    assert_eq!(tool_records.len(), 8, "{trace}");
    for line in &tool_records {
        assert!(!line.contains("tok_echo"), "{line}");
    }
    assert!(trace.contains(r#""result":{"__corvid_secret_scrubbed__":"you sent <redacted>"}"#), "{trace}");
    assert!(
        trace.contains(r#""result":{"__corvid_outage__":true,"__corvid_secret_scrubbed__":true,"__corvid_tool_error__":"tool `fail` failed: bad key <redacted>"}"#),
        "{trace}"
    );
}

#[test]
fn json_cannot_mint_a_secret() {
    let error = json_to_value(json!("hunter2"), &corvid_types::Type::Secret, &Default::default()).unwrap_err();
    assert!(error.to_string().contains("secret cannot be read from JSON"), "{error}");
}
