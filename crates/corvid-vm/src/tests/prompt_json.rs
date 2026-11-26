//! A prompt exchanges the public JSON form with a model (native scalar
//! boundaries B1): the rendered text, the recorded arguments, the reply
//! schema and the reply it reads all use the form `T.to_json()` writes.

use super::*;
use std::sync::Mutex;

/// What one model request carried.
#[derive(Clone, Debug, Default)]
struct Seen {
    rendered: String,
    args: Vec<serde_json::Value>,
    schema: Option<serde_json::Value>,
}

/// Answers every request with `reply` and keeps what it was sent.
#[derive(Clone)]
struct RecordingModel {
    reply: serde_json::Value,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl corvid_runtime::LlmAdapter for RecordingModel {
    fn name(&self) -> &str {
        "recording"
    }

    fn handles(&self, model: &str) -> bool {
        model == "recording"
    }

    fn call<'a>(
        &'a self,
        req: &'a corvid_runtime::llm::LlmRequestRef<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<corvid_runtime::LlmResponse, RuntimeError>> + Send + 'a>,
    > {
        self.seen.lock().unwrap().push(Seen {
            rendered: req.rendered.to_string(),
            args: req.args.to_vec(),
            schema: req.output_schema.cloned(),
        });
        let reply = self.reply.clone();
        Box::pin(async move { Ok(corvid_runtime::LlmResponse::new(reply, TokenUsage::default())) })
    }
}

async fn ask(src: &str, agent: &str, args: Vec<Value>, reply: serde_json::Value) -> (Result<Value, InterpError>, Seen) {
    let ir = ir_of(src);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(RecordingModel { reply, seen: seen.clone() }))
        .default_model("recording")
        .build();
    let result = run_agent(&ir, agent, args, &rt).await;
    let first = seen.lock().unwrap().first().cloned().unwrap_or_default();
    (result, first)
}

const TRIAGE: &str = r#"
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

prompt triage(ticket: Ticket, hint: Option<String>) -> Verdict:
    "Triage {ticket} with hint {hint}"

agent run_triage(owner: Option<String>, hint: Option<String>) -> Verdict:
    return triage(Ticket("Login fails", owner, High("outage")), hint)
"#;

fn verdict_reply() -> serde_json::Value {
    json!({
        "summary": "auth outage",
        "assignee": null,
        "severity": {"tag": "variant", "type": "Severity", "variant": "High", "fields": ["outage"]}
    })
}

/// An `Option` is its value or `null`, and a variant carries its tag,
/// type and fields, in the text the model reads and in the recorded
/// arguments, never the interpreter's `{"tag": "some", ...}` envelope.
#[tokio::test]
async fn a_prompt_shows_the_model_its_arguments_in_public_json() {
    let none = Value::OptionNone;
    let some = |text: &str| Value::OptionSome(crate::value::BoxedValue::new(Value::String(Arc::from(text))));
    let (result, seen) = ask(TRIAGE, "run_triage", vec![none, some("check SSO")], verdict_reply()).await;
    result.expect("run");

    let ticket = json!({
        "title": "Login fails",
        "owner": null,
        "severity": {"tag": "variant", "type": "Severity", "variant": "High", "fields": ["outage"]}
    });
    assert_eq!(seen.args, vec![ticket.clone(), json!("check SSO")]);
    assert_eq!(seen.rendered, format!("Triage {ticket} with hint \"check SSO\""));
    assert!(!seen.rendered.contains("\"some\""), "{}", seen.rendered);

    let (_, seen) = ask(TRIAGE, "run_triage", vec![some("ana"), Value::OptionNone], verdict_reply()).await;
    assert_eq!(seen.args[0]["owner"], json!("ana"));
    assert_eq!(seen.args[1], serde_json::Value::Null);
    assert!(seen.rendered.ends_with("with hint null"), "{}", seen.rendered);
}

/// The reply schema describes the same form: an `Option` field is
/// nullable and may be left out, a sum type names its variants with
/// `tag`, `type` and `fields`.
#[tokio::test]
async fn the_reply_schema_describes_the_public_form() {
    let (_, seen) = ask(TRIAGE, "run_triage", vec![Value::OptionNone, Value::OptionNone], verdict_reply()).await;
    let schema = seen.schema.expect("a structured reply has a schema");
    assert_eq!(schema["required"], json!(["summary", "severity"]));
    assert_eq!(schema["properties"]["assignee"], json!({"type": ["string", "null"]}));
    let variants = schema["properties"]["severity"]["oneOf"].as_array().expect("one alternative per variant");
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[1]["properties"]["type"], json!({"const": "Severity"}));
    assert_eq!(variants[1]["properties"]["variant"], json!({"const": "High"}));
    assert_eq!(variants[1]["properties"]["fields"]["prefixItems"], json!([{"type": "string"}]));
    assert!(!schema.to_string().contains("\"some\""), "{schema}");
}

/// The reply is read in the public form: `null` and an absent field are
/// `None`. The interpreter's old tagged envelope is not an `Option`
/// any more, and the error names the field.
#[tokio::test]
async fn a_reply_is_read_in_the_public_form() {
    let (result, _) = ask(TRIAGE, "run_triage", vec![Value::OptionNone, Value::OptionNone], verdict_reply()).await;
    let Value::Struct(verdict) = result.expect("run") else { panic!("a Verdict") };
    assert_eq!(verdict.get_field("assignee"), Some(Value::OptionNone));

    let absent = json!({"summary": "s", "severity": {"variant": "Low"}});
    let (result, _) = ask(TRIAGE, "run_triage", vec![Value::OptionNone, Value::OptionNone], absent).await;
    let Value::Struct(verdict) = result.expect("an absent Option field is None") else { panic!("a Verdict") };
    assert_eq!(verdict.get_field("assignee"), Some(Value::OptionNone));

    let tagged = json!({"summary": "s", "assignee": {"tag": "some", "value": "ana"}, "severity": {"variant": "Low"}});
    let (result, _) = ask(TRIAGE, "run_triage", vec![Value::OptionNone, Value::OptionNone], tagged).await;
    let error = result.expect_err("the internal envelope is not a String");
    let message = error.to_string();
    assert!(message.contains("prompt `triage`") && message.contains("assignee"), "{message}");
}

/// Scalar arguments read the same in both forms; `None` is `null`, in the
/// text and in the recorded arguments.
#[tokio::test]
async fn a_none_argument_reads_as_null() {
    let src = r#"
prompt pick(n: Int, label: Option<String>) -> String:
    "Pick {n} {label}"

agent run_pick() -> String:
    return pick(3, None)
"#;
    let (result, seen) = ask(src, "run_pick", vec![], json!("ok")).await;
    assert_eq!(result.expect("run"), Value::String(Arc::from("ok")));
    assert_eq!(seen.args, vec![json!(3), serde_json::Value::Null]);
    assert_eq!(seen.rendered, "Pick 3 null");
}
