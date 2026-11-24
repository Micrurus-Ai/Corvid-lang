//! Ed25519 verification on the interpreter, run from Corvid source. The
//! rules, the strictness and every RFC 8032 section 7.1 vector are
//! tested in `corvid-crypto`; these tests pin the method surface: keys
//! and signatures decoded from hex and base64, every kind of tampering
//! rejected, wrong lengths answered `false` rather than an error, and
//! nothing traced.

use super::*;
use crate::value::BoxedValue;

// RFC 8032 section 7.1, TEST 1 (an empty message), TEST 2 (the byte
// 0x72, "r") and TEST 3 (the bytes af82).
const TEST_1_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const TEST_1_SIGNATURE: &str = "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b";
const TEST_2_KEY: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
const TEST_2_SIGNATURE: &str = "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00";
const TEST_3_KEY_BASE64: &str = "/FHNjmIYoaONpH7QAjDwWAgW7RO6MwOsXeuRFUiQgCU=";
const TEST_3_KEY_BASE64_URL: &str = "_FHNjmIYoaONpH7QAjDwWAgW7RO6MwOsXeuRFUiQgCU";
const TEST_3_SIGNATURE_BASE64: &str = "YpHWV97sJAJIJ+acOr4BowzlSKKEdDpEXjaA19taw6wY/5tTjRbykK5n92CYTcZZSnwV6XFu0o3AJ77O6h7ECg==";
const TEST_3_SIGNATURE_BASE64_URL: &str = "YpHWV97sJAJIJ-acOr4BowzlSKKEdDpEXjaA19taw6wY_5tTjRbykK5n92CYTcZZSnwV6XFu0o3AJ77O6h7ECg";

/// The program every test runs: `check` decodes a hex message, a hex
/// signature and a hex key and answers whether the signature verifies.
const CHECKER_PROGRAM: &str = "\
agent check(message_hex: String, signature_hex: String, key_hex: String) -> Result<Bool, String>:
    message = Bytes.from_hex(message_hex)?
    signature = Bytes.from_hex(signature_hex)?
    public_key = Bytes.from_hex(key_hex)?
    return Ok(message.verify_ed25519(signature, public_key))
";

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

async fn verifies(message_hex: &str, signature_hex: &str, key_hex: &str) -> bool {
    let ir = ir_of(CHECKER_PROGRAM);
    let args = vec![text(message_hex), text(signature_hex), text(key_hex)];
    match run_agent(&ir, "check", args, &empty_runtime()).await.expect("run") {
        Value::ResultOk(inner) => match inner.get() {
            Value::Bool(answer) => answer,
            other => panic!("expected a Bool, got {other:?}"),
        },
        other => panic!("expected Ok, got {other:?}"),
    }
}

fn bools(value: Value) -> Vec<bool> {
    let value = match value {
        Value::ResultOk(inner) => inner.get(),
        other => other,
    };
    let Value::List(items) = value else { panic!("expected a list, got {value:?}") };
    items
        .iter_cloned()
        .into_iter()
        .map(|item| match item {
            Value::Bool(answer) => answer,
            other => panic!("expected a Bool, got {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn the_rfc_8032_signatures_verify() {
    let src = format!(
        "\
agent main() -> Result<List<Bool>, String>:
    test_1 = \"\".utf8().verify_ed25519(Bytes.from_hex(\"{TEST_1_SIGNATURE}\")?, Bytes.from_hex(\"{TEST_1_KEY}\")?)
    test_2 = \"r\".utf8().verify_ed25519(Bytes.from_hex(\"{TEST_2_SIGNATURE}\")?, Bytes.from_hex(\"{TEST_2_KEY}\")?)
    test_3 = Bytes.from_hex(\"af82\")?.verify_ed25519(Bytes.from_base64(\"{TEST_3_SIGNATURE_BASE64}\")?, Bytes.from_base64(\"{TEST_3_KEY_BASE64}\")?)
    test_3_url = Bytes.from_hex(\"AF82\")?.verify_ed25519(Bytes.from_base64_url(\"{TEST_3_SIGNATURE_BASE64_URL}\")?, Bytes.from_base64_url(\"{TEST_3_KEY_BASE64_URL}\")?)
    return Ok([test_1, test_2, test_3, test_3_url])
"
    );
    let ir = ir_of(&src);
    let result = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run");
    assert_eq!(bools(result), vec![true, true, true, true]);
}

#[tokio::test]
async fn a_valid_signature_verifies_and_every_change_is_rejected() {
    assert!(verifies("72", TEST_2_SIGNATURE, TEST_2_KEY).await);
    // One bit of the message: "s" instead of "r".
    assert!(!verifies("73", TEST_2_SIGNATURE, TEST_2_KEY).await);
    // A longer message that starts with the signed one.
    assert!(!verifies("7200", TEST_2_SIGNATURE, TEST_2_KEY).await);
    // One bit of the signature, in R (the first half) and in S (the last byte).
    let flipped_r = format!("93{}", &TEST_2_SIGNATURE[2..]);
    let flipped_s = format!("{}01", &TEST_2_SIGNATURE[..126]);
    assert!(!verifies("72", &flipped_r, TEST_2_KEY).await);
    assert!(!verifies("72", &flipped_s, TEST_2_KEY).await);
    // Another signer's key, and the right key with one bit changed.
    assert!(!verifies("72", TEST_2_SIGNATURE, TEST_1_KEY).await);
    let flipped_key = format!("3c{}", &TEST_2_KEY[2..]);
    assert!(!verifies("72", TEST_2_SIGNATURE, &flipped_key).await);
    // Another vector's signature over its own message, under this key.
    assert!(!verifies("", TEST_1_SIGNATURE, TEST_2_KEY).await);
}

#[tokio::test]
async fn wrong_lengths_are_false_not_errors() {
    let short_signature = &TEST_2_SIGNATURE[..126];
    let long_signature = format!("{TEST_2_SIGNATURE}00");
    let short_key = &TEST_2_KEY[..62];
    let long_key = format!("{TEST_2_KEY}{TEST_2_KEY}");
    assert!(!verifies("72", short_signature, TEST_2_KEY).await);
    assert!(!verifies("72", &long_signature, TEST_2_KEY).await);
    assert!(!verifies("72", "", TEST_2_KEY).await);
    assert!(!verifies("72", TEST_2_SIGNATURE, short_key).await);
    assert!(!verifies("72", TEST_2_SIGNATURE, &long_key).await);
    assert!(!verifies("72", TEST_2_SIGNATURE, "").await);
}

#[tokio::test]
async fn a_small_order_key_never_verifies() {
    // The identity point as the key and as R, with S = 0: a permissive
    // verifier accepts this for any message.
    let identity = format!("01{}", "00".repeat(31));
    let forged = format!("{identity}{}", "00".repeat(32));
    assert!(!verifies("72", &forged, &identity).await);
    assert!(!verifies("", &forged, &identity).await);
}

#[tokio::test]
async fn verification_is_not_traced() {
    let src = format!(
        "\
@deterministic
agent main(body: String) -> Result<Bool, String>:
    signature = Bytes.from_hex(\"{TEST_2_SIGNATURE}\")?
    public_key = Bytes.from_hex(\"{TEST_2_KEY}\")?
    return Ok(body.utf8().verify_ed25519(signature, public_key))
"
    );
    let ir = ir_of(&src);
    let dir = tempfile::tempdir().unwrap();
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tracer(corvid_runtime::Tracer::open(dir.path(), "ed25519-trace"))
        .build();
    let result = run_agent(&ir, "main", vec![text("r")], &rt).await.expect("run");
    assert_eq!(result, Value::ResultOk(BoxedValue::new(Value::Bool(true))));
    let trace = std::fs::read_to_string(dir.path().join("ed25519-trace.jsonl")).unwrap();
    let kinds: Vec<String> = trace
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["kind"].as_str().unwrap().to_string())
        .filter(|kind| !matches!(kind.as_str(), "schema_header" | "seed_read"))
        .collect();
    assert_eq!(kinds, ["run_started", "run_completed"], "{trace}");
}
