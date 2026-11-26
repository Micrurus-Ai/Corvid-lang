//! B03 text methods on the interpreter: encoders, decoders and UUID
//! parsing, run from Corvid source. The rules themselves are tested in
//! `corvid-text`; these tests pin the method surface and the Result
//! values the decoders return.

use super::*;

async fn run_main(src: &str) -> Value {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run")
}

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn ok(value: &str) -> Value {
    Value::ResultOk(crate::value::BoxedValue::new(text(value)))
}

fn err(value: &str) -> Value {
    Value::ResultErr(crate::value::BoxedValue::new(text(value)))
}

#[tokio::test]
async fn encoders_return_the_encoded_text() {
    let src = "\
agent main() -> List<String>:
    word = \"café ☕\"
    return [word.encode_base64(), word.encode_base64_url(), word.encode_hex(), \"a b/c+d\".encode_url_component()]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(
        items.iter_cloned(),
        vec![text("Y2Fmw6kg4piV"), text("Y2Fmw6kg4piV"), text("636166c3a920e29895"), text("a%20b%2Fc%2Bd")]
    );
}

#[tokio::test]
async fn decoders_round_trip_through_result() {
    let src = "\
agent main() -> Bool:
    word = \"naïve résumé\"
    base = word.encode_base64().decode_base64() == Ok(word)
    url_safe = word.encode_base64_url().decode_base64_url() == Ok(word)
    hex = word.encode_hex().decode_hex() == Ok(word)
    component = word.encode_url_component().decode_url_component() == Ok(word)
    return base and url_safe and hex and component
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}

#[tokio::test]
async fn malformed_input_is_an_err_value_not_a_trap() {
    for (call, expected) in [
        ("\"Zm9v!A==\".decode_base64()", err("not base64: unexpected character `!` at position 4")),
        ("\"4g\".decode_hex()", err("not hex: unexpected character `g` at position 1")),
        (
            "\"100%\".decode_url_component()",
            err("not URL-encoded: `%` at position 3 must be followed by two hex digits"),
        ),
        ("\"a+b\".decode_url_component()", ok("a+b")),
    ] {
        let src = format!("agent main() -> Result<String, String>:\n    return {call}\n");
        assert_eq!(run_main(&src).await, expected, "{call}");
    }
}

#[tokio::test]
async fn parse_uuid_returns_the_canonical_form() {
    let src = "\
agent main() -> List<Result<String, String>>:
    return [\"{F81D4FAE-7DEC-11D0-A765-00A0C91E6BF6}\".parse_uuid(), \"not-a-uuid\".parse_uuid()]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(
        items.iter_cloned(),
        vec![
            ok("f81d4fae-7dec-11d0-a765-00a0c91e6bf6"),
            err("not a UUID: `not-a-uuid` (expected 32 hex digits, optionally hyphenated 8-4-4-4-12)"),
        ]
    );
}

#[tokio::test]
async fn the_reference_examples_run() {
    // docs/reference/text-and-encoding.md
    let src = "\
agent basic_auth(user: String, password: String) -> String:
    return \"Basic \" + (user + \":\" + password).encode_base64()

agent read_token(payload: String) -> Result<String, String>:
    return payload.decode_base64_url()

agent order_key(raw: String) -> Result<String, String>:
    return raw.parse_uuid()

agent main() -> Bool:
    auth = basic_auth(\"ada\", \"lovelace\") == \"Basic YWRhOmxvdmVsYWNl\"
    token = read_token(\"eyJzdWIiOiJhZGEifQ\") == Ok(\"{\\\"sub\\\":\\\"ada\\\"}\")
    key = order_key(\" urn:uuid:F81D4FAE-7DEC-11D0-A765-00A0C91E6BF6 \") == Ok(\"f81d4fae-7dec-11d0-a765-00a0c91e6bf6\")
    hex = \"café\".encode_hex() == \"636166c3a9\"
    return auth and token and key and hex
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}

#[tokio::test]
async fn text_methods_are_allowed_in_deterministic_agents() {
    let src = "\
@deterministic
agent token(user: String) -> String:
    return user.encode_base64_url()

agent main() -> String:
    return token(\"ada\")
";
    assert_eq!(run_main(src).await, text("YWRh"));
}
