//! Ed25519 verification in the checker: `message.verify_ed25519(signature,
//! public_key)` takes three `Bytes`, returns `Bool` and is pure.

use super::check;
use crate::errors::TypeErrorKind;

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn a_signature_check_takes_bytes_and_returns_bool() {
    assert_clean(
        "agent signed(body: String, signature_hex: String, public_key: Bytes) -> Bool:
    return match Bytes.from_hex(signature_hex):
        Ok(signature) -> body.utf8().verify_ed25519(signature, public_key)
        Err(problem) -> false
",
    );
}

#[test]
fn a_signature_check_is_allowed_in_deterministic_agents() {
    assert_clean(
        "@deterministic
agent signed(message: Bytes, signature: Bytes, public_key: Bytes) -> Bool:
    return message.verify_ed25519(signature, public_key)
",
    );
}

#[test]
fn the_signature_and_the_key_must_be_bytes() {
    for (signature, public_key) in [("\"e556\"", "key"), ("signature", "\"d75a\""), ("signature", "secret_key")] {
        let errors = check(&format!(
            "agent signed(message: Bytes, signature: Bytes, key: Bytes, secret_key: Secret) -> Bool:
    return message.verify_ed25519({signature}, {public_key})
"
        ))
        .errors;
        assert!(errors.iter().any(|e| matches!(e.kind, TypeErrorKind::TypeMismatch { .. })), "{signature}, {public_key}: {errors:?}");
    }
}

#[test]
fn a_signature_check_takes_exactly_two_arguments() {
    let errors = check("agent signed(message: Bytes, signature: Bytes) -> Bool:\n    return message.verify_ed25519(signature)\n").errors;
    assert!(
        errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::ArityMismatch { expected: 2, got: 1, .. })),
        "{errors:?}"
    );
}

#[test]
fn only_bytes_are_verified() {
    let errors = check("agent signed(message: String, signature: Bytes, key: Bytes) -> Bool:\n    return message.verify_ed25519(signature, key)\n").errors;
    assert!(!errors.is_empty(), "a String has no `verify_ed25519`; the message's encoding is written with `utf8()`");
}
