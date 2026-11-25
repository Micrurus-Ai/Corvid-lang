//! End to end: verify a recorded payment-provider webhook signature and
//! reject a tampered request.
//!
//! The program is ordinary Corvid run through the driver. It reads the
//! signing secret with `secret(name)`, splits the signature header with
//! string methods, computes the HMAC-SHA256 of the signed payload and
//! compares it with the received signature using `==` on `Bytes`, which
//! is constant-time. A reusable header parser, the timestamp tolerance
//! (which needs the clock) and raw request bodies in routes belong to
//! verified webhooks (B16); this test proves the cryptography they will
//! use.
//!
//! The expected signatures were computed outside Corvid, with Python's
//! `hmac` module and checked with `openssl dgst -sha256 -hmac`:
//!
//! - Stripe: `hmac.new(STRIPE_KEY.encode(), (TIMESTAMP + "." + BODY).encode(), sha256).hexdigest()`.
//! - Standard Webhooks: the key is the 24 bytes 0x10..=0x27, written as
//!   `whsec_` + base64, and the signature is
//!   `"v1," + base64(hmac.new(key, (MESSAGE_ID + "." + TIMESTAMP + "." + BODY).encode(), sha256).digest())`.

#[path = "support/std_project.rs"]
mod std_project;

use corvid_driver::run_ir_with_runtime;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;

const SOURCE: &str = r#"
import "./std/secrets" use secret

# Stripe signs `<timestamp>.<raw body>` with the signing secret's text and
# sends `Stripe-Signature: t=<timestamp>,v1=<hex>`, with one `v1` per
# active secret.
agent verify_stripe(signature_header: String, body: String) -> Result<Bool, String>:
    key = secret("STRIPE_WEBHOOK_SECRET")?
    timestamp = ""
    for part in signature_header.split(","):
        if part.starts_with("t="):
            timestamp = part.substring(2, part.length())
    if timestamp == "":
        return Err("the signature header has no timestamp")
    expected = (timestamp + "." + body).utf8().hmac_sha256(key)
    for part in signature_header.split(","):
        if part.starts_with("v1="):
            matched = match Bytes.from_hex(part.substring(3, part.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return Ok(true)
    return Ok(false)

# Standard Webhooks (Svix) signs `<id>.<timestamp>.<raw body>` with the
# bytes written in base64 after `whsec_`, and sends
# `webhook-signature: v1,<base64>`, space-separated when there are several.
agent verify_standard_webhook(message_id: String, timestamp: String, signature_header: String, body: String) -> Result<Bool, String>:
    encoded = secret("STANDARD_WEBHOOK_SECRET")?
    key = encoded.decode_base64_after("whsec_")?
    expected = (message_id + "." + timestamp + "." + body).utf8().hmac_sha256(key)
    for entry in signature_header.split(" "):
        if entry.starts_with("v1,"):
            matched = match Bytes.from_base64(entry.substring(3, entry.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return Ok(true)
    return Ok(false)
"#;

const BODY: &str = r#"{"id":"evt_1QZ","type":"payment_intent.succeeded","data":{"object":{"id":"pi_3Q","amount":2000,"currency":"usd"}}}"#;
const TIMESTAMP: &str = "1732480000";

const STRIPE_KEY: &str = "whsec_corvid_test_4eC39HqLyjWDarjtT1zdp7dc";
const STRIPE_SIGNATURE: &str = "9d4aa1ff5638dd3708a9c29539870fc8fc470059f7feeb7ff559ab641d7f6dce";

const STANDARD_KEY: &str = "whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYn";
const STANDARD_MESSAGE_ID: &str = "msg_2KWPBgLlAfxdpx2AI54pPJ85f4W";
const STANDARD_SIGNATURE: &str = "v1,+dJkrlylFBxSVAn/YI6mFfwStrqxrlMnbfCAVbmRja4=";

fn compile() -> corvid_ir::IrFile {
    std_project::compile_with_std(SOURCE, &["effects.cor", "secrets.cor"])
}

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn runtime() -> Runtime {
    Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes())).build()
}

async fn verified(ir: &corvid_ir::IrFile, agent: &str, args: &[&str], runtime: &Runtime) -> bool {
    let args = args.iter().map(|arg| text(arg)).collect();
    match run_ir_with_runtime(ir, Some(agent), args, runtime).await.expect("the verification runs") {
        Value::ResultOk(inner) => match inner.get() {
            Value::Bool(verified) => verified,
            other => panic!("expected Ok(Bool), got Ok({other:?})"),
        },
        other => panic!("expected Ok, got {other:?}"),
    }
}

/// One body byte changed: `2000` becomes `9000`.
fn tampered_body() -> String {
    let tampered = BODY.replacen("2000", "9000", 1);
    assert_eq!(tampered.len(), BODY.len());
    assert_eq!(tampered.bytes().zip(BODY.bytes()).filter(|(a, b)| a != b).count(), 1);
    tampered
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_stripe_webhook_verifies_and_a_tampered_one_is_rejected() {
    std::env::set_var("STRIPE_WEBHOOK_SECRET", STRIPE_KEY);
    let ir = compile();
    let header = format!("t={TIMESTAMP},v1={STRIPE_SIGNATURE}");

    // The recorded request verifies, and the run's trace never holds the
    // signing key: the secret read records `<redacted>`, and the HMAC is
    // pure, so it is never traced.
    let trace_dir = tempfile::tempdir().unwrap();
    let record = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .trace_to(trace_dir.path())
        .build();
    assert!(verified(&ir, "verify_stripe", &[&header, BODY], &record).await, "the recorded webhook verifies");
    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    let trace = std::fs::read_to_string(&trace_path).expect("the trace file");
    assert!(!trace.contains(STRIPE_KEY), "the trace must never hold the signing key:\n{trace}");
    assert!(!trace.contains("corvid_test_4eC39"), "not even part of it:\n{trace}");
    assert!(trace.contains(r#""ok":"<redacted>""#), "the secret read records <redacted>:\n{trace}");

    let rt = runtime();
    let tampered = tampered_body();
    assert!(!verified(&ir, "verify_stripe", &[&header, &tampered], &rt).await, "a changed body byte is rejected");
    let later = format!("t=1732480001,v1={STRIPE_SIGNATURE}");
    assert!(!verified(&ir, "verify_stripe", &[&later, BODY], &rt).await, "a changed timestamp is rejected");
    let flipped = format!("t={TIMESTAMP},v1={}{}", &STRIPE_SIGNATURE[..63], "f");
    assert!(!verified(&ir, "verify_stripe", &[&flipped, BODY], &rt).await, "a changed signature is rejected");
    let missing = format!("t={TIMESTAMP}");
    assert!(!verified(&ir, "verify_stripe", &[&missing, BODY], &rt).await, "no signature is rejected");
    // Stripe sends one `v1` per active secret while a secret is rolled.
    let rolled = format!("t={TIMESTAMP},v1={},v1={STRIPE_SIGNATURE}", "0".repeat(64));
    assert!(verified(&ir, "verify_stripe", &[&rolled, BODY], &rt).await, "any matching v1 verifies");

    std::env::set_var("STRIPE_WEBHOOK_SECRET", "whsec_corvid_test_someone_elses_key");
    assert!(!verified(&ir, "verify_stripe", &[&header, BODY], &runtime()).await, "a wrong key is rejected");
    std::env::set_var("STRIPE_WEBHOOK_SECRET", STRIPE_KEY);

    // Replay re-reads the key from the environment and recomputes the
    // HMAC, reaching the same verdict.
    let replay = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .replay_from(&trace_path)
        .build();
    assert!(verified(&ir, "verify_stripe", &[&header, BODY], &replay).await, "the recorded run replays");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_standard_webhook_verifies_with_its_base64_key_and_a_tampered_one_is_rejected() {
    std::env::set_var("STANDARD_WEBHOOK_SECRET", STANDARD_KEY);
    let ir = compile();
    let rt = runtime();
    let verify = |message_id: &'static str, timestamp: &'static str, header: &'static str, body: String| {
        let ir = &ir;
        let rt = &rt;
        async move { verified(ir, "verify_standard_webhook", &[message_id, timestamp, header, &body], rt).await }
    };
    assert!(verify(STANDARD_MESSAGE_ID, TIMESTAMP, STANDARD_SIGNATURE, BODY.to_string()).await, "the recorded webhook verifies");
    assert!(!verify(STANDARD_MESSAGE_ID, TIMESTAMP, STANDARD_SIGNATURE, tampered_body()).await, "a changed body byte is rejected");
    assert!(!verify(STANDARD_MESSAGE_ID, "1732480001", STANDARD_SIGNATURE, BODY.to_string()).await, "a changed timestamp is rejected");
    assert!(!verify("msg_2KWPBgLlAfxdpx2AI54pPJ85f4X", TIMESTAMP, STANDARD_SIGNATURE, BODY.to_string()).await, "a changed message id is rejected");
    let several = "v1,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= v1,+dJkrlylFBxSVAn/YI6mFfwStrqxrlMnbfCAVbmRja4=";
    assert!(verify(STANDARD_MESSAGE_ID, TIMESTAMP, several, BODY.to_string()).await, "any matching v1 verifies");

    // A key one byte off (the last decoded byte 0x27 becomes 0x28) is
    // rejected; a secret without the provider's prefix is an `Err`.
    std::env::set_var("STANDARD_WEBHOOK_SECRET", "whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYo");
    assert!(!verify(STANDARD_MESSAGE_ID, TIMESTAMP, STANDARD_SIGNATURE, BODY.to_string()).await, "a wrong key is rejected");
    std::env::set_var("STANDARD_WEBHOOK_SECRET", "EBESExQVFhcYGRobHB0eHyAhIiMkJSYn");
    let outcome = run_ir_with_runtime(
        &ir,
        Some("verify_standard_webhook"),
        [STANDARD_MESSAGE_ID, TIMESTAMP, STANDARD_SIGNATURE, BODY].iter().map(|arg| text(arg)).collect(),
        &rt,
    )
    .await
    .expect("the verification runs");
    assert!(
        matches!(&outcome, Value::ResultErr(problem) if problem.get() == text("the secret does not start with `whsec_`")),
        "a key without the provider's prefix is an Err that does not quote it: {outcome:?}"
    );
    std::env::set_var("STANDARD_WEBHOOK_SECRET", STANDARD_KEY);
}
