//! HMAC on the interpreter, run from Corvid source. The rules are
//! tested against RFC 4231 in `corvid-crypto`; these tests pin the
//! method surface: a `Bytes` or `Secret` key, a base64 key decoded from
//! a secret, constant-time comparison of tags, and that neither the key
//! nor the HMAC reaches a trace.

use super::*;

const SECRET_TOOL: &str = "\
effect secrets_read:
    reversible: true

tool secret(name: String) -> Result<Secret, String> uses secrets_read
";

/// RFC 4231 test case 2: key "Jefe", message "what do ya want for nothing?".
const CASE_2_SHA256: &str = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
const CASE_2_SHA512: &str = "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737";

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn texts(value: Value) -> Vec<Value> {
    let value = match value {
        Value::ResultOk(inner) => inner.get(),
        other => other,
    };
    let Value::List(items) = value else { panic!("expected a list, got {value:?}") };
    items.iter_cloned()
}

#[tokio::test]
async fn bytes_keys_give_the_rfc_4231_tags() {
    let src = "\
agent main() -> Result<List<String>, String>:
    case_1_key = Bytes.from_hex(\"0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b\")?
    case_1 = \"Hi There\".utf8()
    case_2 = \"what do ya want for nothing?\".utf8()
    return Ok([case_1.hmac_sha256(case_1_key).to_hex(), case_2.hmac_sha256(\"Jefe\".utf8()).to_hex(), case_2.hmac_sha512(\"Jefe\".utf8()).to_hex()])
";
    let ir = ir_of(src);
    let result = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run");
    assert_eq!(
        texts(result),
        vec![text("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"), text(CASE_2_SHA256), text(CASE_2_SHA512)]
    );
}

#[tokio::test]
async fn a_secret_keys_the_hmac_with_its_text_or_its_decoded_bytes() {
    std::env::set_var("CORVID_VM_HMAC_TEXT_KEY", "Jefe");
    // "SmVmZQ==" is the base64 of "Jefe".
    std::env::set_var("CORVID_VM_HMAC_ENCODED_KEY", "whsec_SmVmZQ==");
    let src = format!(
        "{SECRET_TOOL}
agent main() -> Result<List<String>, String>:
    message = \"what do ya want for nothing?\".utf8()
    text_key = secret(\"CORVID_VM_HMAC_TEXT_KEY\")?
    encoded = secret(\"CORVID_VM_HMAC_ENCODED_KEY\")?
    decoded_key = encoded.decode_base64_after(\"whsec_\")?
    received = Bytes.from_hex(\"{CASE_2_SHA256}\")?
    same = message.hmac_sha256(decoded_key) == received and message.hmac_sha256(text_key) == received
    if not same:
        return Err(\"the text key and the decoded key must agree\")
    return Ok([message.hmac_sha256(text_key).to_hex(), message.hmac_sha512(decoded_key).to_hex()])
"
    );
    let ir = ir_of(&src);
    let result = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run");
    assert_eq!(texts(result), vec![text(CASE_2_SHA256), text(CASE_2_SHA512)]);
}

#[tokio::test]
async fn decoding_a_key_is_an_err_that_never_quotes_the_secret() {
    std::env::set_var("CORVID_VM_HMAC_WRONG_PREFIX", "sk_SmVmZQ==");
    std::env::set_var("CORVID_VM_HMAC_NOT_BASE64", "whsec_Sm!mZQ==");
    let src = format!(
        "{SECRET_TOOL}
agent problem(name: String) -> String:
    return match secret(name):
        Ok(encoded) -> match encoded.decode_base64_after(\"whsec_\"):
            Ok(key) -> \"decoded\"
            Err(problem) -> problem
        Err(problem) -> problem
"
    );
    let ir = ir_of(&src);
    let rt = empty_runtime();
    let wrong_prefix = run_agent(&ir, "problem", vec![text("CORVID_VM_HMAC_WRONG_PREFIX")], &rt).await.expect("run");
    assert_eq!(wrong_prefix, text("the secret does not start with `whsec_`"));
    let not_base64 = run_agent(&ir, "problem", vec![text("CORVID_VM_HMAC_NOT_BASE64")], &rt).await.expect("run");
    assert_eq!(not_base64, text("the secret is not base64 after `whsec_`"));
}

#[tokio::test]
async fn a_decoded_key_never_reaches_a_tool() {
    std::env::set_var("CORVID_VM_HMAC_TOOL_KEY", "whsec_SmVmZQ==");
    let src = format!(
        "{SECRET_TOOL}
tool sign(key: Secret, body: String) -> String

agent main() -> Result<String, String>:
    encoded = secret(\"CORVID_VM_HMAC_TOOL_KEY\")?
    key = encoded.decode_base64_after(\"whsec_\")?
    return Ok(sign(key, \"payload\"))
"
    );
    let ir = ir_of(&src);
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("sign", |_args| async move { panic!("a decoded key must not reach a tool") })
        .build();
    let error = run_agent(&ir, "main", vec![], &rt).await.expect_err("refused").to_string();
    assert!(error.contains("tool `sign`") && error.contains("bytes, not text"), "{error}");
    assert!(!error.contains("SmVmZQ") && !error.contains("Jefe"), "{error}");
}

#[tokio::test]
async fn neither_the_key_nor_the_hmac_is_traced() {
    std::env::set_var("CORVID_VM_HMAC_TRACED_KEY", "whsec_trace_me_not_2718");
    let src = format!(
        "{SECRET_TOOL}
agent main(body: String) -> Result<String, String>:
    key = secret(\"CORVID_VM_HMAC_TRACED_KEY\")?
    return Ok(body.utf8().hmac_sha256(key).to_hex())
"
    );
    let ir = ir_of(&src);
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tracer(corvid_runtime::Tracer::open(dir.path(), "hmac-trace"))
        .build();
    run_agent(&ir, "main", vec![text("order.paid")], &rt).await.expect("run");
    let trace = std::fs::read_to_string(dir.path().join("hmac-trace.jsonl")).unwrap();
    assert!(!trace.contains("trace_me_not"), "the trace holds the key:\n{trace}");
    // A pure method records nothing: the only events between the run's
    // start and end are the secret read's call and result.
    let kinds: Vec<String> = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["kind"].as_str().unwrap().to_string())
        .filter(|kind| !matches!(kind.as_str(), "schema_header" | "seed_read"))
        .collect();
    assert_eq!(kinds, ["run_started", "tool_call", "tool_result", "run_completed"], "{trace}");
    assert!(trace.contains(r#""tool":"secret","args":["CORVID_VM_HMAC_TRACED_KEY"]"#), "{trace}");
}
