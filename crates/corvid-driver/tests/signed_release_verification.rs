//! End to end: verify an Ed25519-signed release attestation in Corvid,
//! and agree with the release tooling on every forgery.
//!
//! `corvid release` signs its manifest as a DSSE envelope through
//! `corvid_abi::sign_envelope`: the signature covers the
//! Pre-Authentication Encoding `DSSEv1 <type length> <type> <payload
//! length> <payload>`, so it cannot be moved to a payload of another
//! type. This test signs a release manifest with that same function and
//! a fixed key, then verifies it with ordinary Corvid run through the
//! driver: the encoding is built with string methods and checked with
//! `message.verify_ed25519(signature, public_key)`. For each case the
//! test also asks the tooling's own verifier, `corvid_abi::verify_envelope`,
//! and requires the same answer.
//!
//! The payload is JSON text, so the encoding is built as a string and
//! then `utf8()`; a binary payload would need byte concatenation, which
//! `Bytes` does not offer yet.

#[path = "support/std_project.rs"]
mod std_project;

use corvid_abi::{sign_envelope, verify_envelope, DsseEnvelope, SigningKey};
use corvid_driver::run_ir_with_runtime;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;

const SOURCE: &str = r#"
# The DSSE Pre-Authentication Encoding the signature covers. Lengths
# count bytes, so they are read from the UTF-8 bytes.
fn pre_authentication_encoding(payload_type: String, payload: String) -> Bytes:
    type_length = payload_type.utf8().length().to_string()
    payload_length = payload.utf8().length().to_string()
    return ("DSSEv1 " + type_length + " " + payload_type + " " + payload_length + " " + payload).utf8()

# Whether the envelope's signature is the signer's, for whatever type it names.
fn signed_by(payload_type: String, payload_base64: String, signature_base64: String, public_key_hex: String) -> Result<Bool, String>:
    payload = Bytes.from_base64(payload_base64)?.to_utf8()?
    signature = Bytes.from_base64(signature_base64)?
    public_key = Bytes.from_hex(public_key_hex)?
    return Ok(pre_authentication_encoding(payload_type, payload).verify_ed25519(signature, public_key))

agent check_signature(payload_type: String, payload_base64: String, signature_base64: String, public_key_hex: String) -> Result<Bool, String>:
    return signed_by(payload_type, payload_base64, signature_base64, public_key_hex)

# A release: the release manifest's type, signed by the release key.
@deterministic
agent verify_release(payload_type: String, payload_base64: String, signature_base64: String, public_key_hex: String) -> Result<Bool, String>:
    if payload_type != "application/vnd.corvid.release.manifest.v1+json":
        return Ok(false)
    return signed_by(payload_type, payload_base64, signature_base64, public_key_hex)
"#;

const RELEASE_TYPE: &str = "application/vnd.corvid.release.manifest.v1+json";
const MANIFEST: &str = r#"{"schema":"corvid.release.manifest.v1","channel":"stable","version":"1.4.0","artifacts":[{"name":"corvid-aarch64-apple-darwin.tar.gz","sha256":"9f2c1e0b7d4a5c3e8f6a1b2d3c4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f6a"}]}"#;
/// The manifest with one changed byte: version 1.4.0 becomes 1.4.1.
const TAMPERED_MANIFEST: &str = r#"{"schema":"corvid.release.manifest.v1","channel":"stable","version":"1.4.1","artifacts":[{"name":"corvid-aarch64-apple-darwin.tar.gz","sha256":"9f2c1e0b7d4a5c3e8f6a1b2d3c4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f6a"}]}"#;

fn release_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

fn public_key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// The fields of an envelope the Corvid program receives.
struct Fields {
    payload_type: String,
    payload: String,
    signature: String,
}

impl Fields {
    fn of(envelope: &DsseEnvelope) -> Self {
        Fields {
            payload_type: envelope.payload_type.clone(),
            payload: envelope.payload.clone(),
            signature: envelope.signatures[0].sig.clone(),
        }
    }

    fn envelope(&self) -> DsseEnvelope {
        let mut envelope = sign_envelope(b"", RELEASE_TYPE, &release_key(), "release-channel");
        envelope.payload_type = self.payload_type.clone();
        envelope.payload = self.payload.clone();
        envelope.signatures[0].sig = self.signature.clone();
        envelope
    }
}

fn signed(manifest: &str) -> Fields {
    Fields::of(&sign_envelope(manifest.as_bytes(), RELEASE_TYPE, &release_key(), "release-channel"))
}

async fn corvid_answer(ir: &corvid_ir::IrFile, agent: &str, fields: &Fields, key_hex: &str) -> bool {
    let args = [fields.payload_type.as_str(), fields.payload.as_str(), fields.signature.as_str(), key_hex]
        .into_iter()
        .map(|arg| Value::String(Arc::from(arg)))
        .collect();
    let runtime = Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes())).build();
    match run_ir_with_runtime(ir, Some(agent), args, &runtime).await.expect("the verification runs") {
        Value::ResultOk(inner) => match inner.get() {
            Value::Bool(answer) => answer,
            other => panic!("expected Ok(Bool), got Ok({other:?})"),
        },
        other => panic!("expected Ok, got {other:?}"),
    }
}

fn tooling_answer(fields: &Fields, key: &SigningKey) -> bool {
    let json = serde_json::to_vec(&fields.envelope()).unwrap();
    verify_envelope(&json, &[RELEASE_TYPE], &key.verifying_key()).is_ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_release_verifies_and_every_forgery_is_rejected_as_the_tooling_rejects_it() {
    let ir = std_project::compile_with_std(SOURCE, &[]);
    let key = release_key();
    let other_key = SigningKey::from_bytes(&[8; 32]);
    let genuine = signed(MANIFEST);

    let tampered_payload = Fields { payload: signed(TAMPERED_MANIFEST).payload, ..signed(MANIFEST) };
    let other_signature = Fields { signature: signed(TAMPERED_MANIFEST).signature, ..signed(MANIFEST) };
    let mut flipped = genuine.signature.clone().into_bytes();
    flipped[10] = if flipped[10] == b'A' { b'B' } else { b'A' };
    let flipped_signature = Fields { signature: String::from_utf8(flipped).unwrap(), ..signed(MANIFEST) };
    let retyped = Fields { payload_type: "application/vnd.corvid.receipt.v1+json".into(), ..signed(MANIFEST) };

    let cases: [(&str, &Fields, &SigningKey, bool); 6] = [
        ("the signed release", &genuine, &key, true),
        ("a changed manifest byte", &tampered_payload, &key, false),
        ("another manifest's signature", &other_signature, &key, false),
        ("a changed signature character", &flipped_signature, &key, false),
        ("another payload type", &retyped, &key, false),
        ("another signer's key", &genuine, &other_key, false),
    ];
    for (case, fields, verifying, expected) in cases {
        let corvid = corvid_answer(&ir, "verify_release", fields, &public_key_hex(verifying)).await;
        assert_eq!(corvid, expected, "{case}: Corvid");
        assert_eq!(tooling_answer(fields, verifying), expected, "{case}: the release tooling");
    }

    // The type is bound by the signature, not only by the type check: the
    // genuine signature does not verify under another type's encoding.
    assert!(corvid_answer(&ir, "check_signature", &genuine, &public_key_hex(&key)).await);
    assert!(!corvid_answer(&ir, "check_signature", &retyped, &public_key_hex(&key)).await);
}
