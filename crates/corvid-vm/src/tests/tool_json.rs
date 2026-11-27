//! A tool call exchanges the public JSON form (native scalar boundaries
//! B2a): each argument by its parameter's declared type, the result read
//! by the declared return type, so a tool sees what a frontend, `to_json`
//! and a model see, and a wrongly shaped answer is refused.

use super::*;
use std::sync::Mutex;

const TRIAGE: &str = r#"
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

tool assign(ticket: Ticket, hint: Option<String>) -> Assignment

agent main(hint: Option<String>) -> Assignment:
    return assign(Ticket("Login fails", None, High("outage")), hint)
"#;

fn runtime_answering(answer: serde_json::Value, seen: Arc<Mutex<Vec<serde_json::Value>>>) -> Runtime {
    Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("assign", move |args| {
            let seen = seen.clone();
            let answer = answer.clone();
            async move {
                seen.lock().unwrap().extend(args);
                Ok(answer)
            }
        })
        .build()
}

/// The tool receives `None` as `null` and a variant with its tag, type and
/// fields, never the interpreter's `{"tag": "some", ...}` envelope, and its
/// answer is read the same way.
#[tokio::test]
async fn a_tool_receives_and_answers_in_the_public_form() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let answer = json!({"owner": null, "severity": {"variant": "High", "fields": ["outage"]}, "extra": 1});
    let rt = runtime_answering(answer, seen.clone());
    let hint = Value::OptionSome(crate::value::BoxedValue::new(Value::String(Arc::from("check SSO"))));
    let result = run_agent(&ir_of(TRIAGE), "main", vec![hint], &rt).await.expect("run");

    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            json!({
                "title": "Login fails",
                "owner": null,
                "severity": {"tag": "variant", "type": "Severity", "variant": "High", "fields": ["outage"]}
            }),
            json!("check SSO"),
        ]
    );
    let Value::Struct(assignment) = result else { panic!("an Assignment") };
    assert_eq!(assignment.get_field("owner"), Some(Value::OptionNone));
}

/// An answer that does not fit the declared return type is refused with
/// the field path, instead of building a wrong value.
#[tokio::test]
async fn a_wrongly_shaped_answer_is_refused_with_its_path() {
    let rt = runtime_answering(json!({"owner": 7, "severity": {"variant": "Low"}}), Arc::new(Mutex::new(Vec::new())));
    let error = run_agent(&ir_of(TRIAGE), "main", vec![Value::OptionNone], &rt).await.expect_err("owner is not text");
    assert!(matches!(error.kind, InterpErrorKind::UnusableReply(_)), "{error:?}");
    let message = error.to_string();
    assert!(message.contains("tool `assign`") && message.contains("owner"), "{message}");
}
