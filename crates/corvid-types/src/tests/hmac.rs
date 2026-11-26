//! HMAC and signing keys in the checker: `hmac_sha256` and
//! `hmac_sha512` take a `Bytes` or `Secret` key and are pure, and
//! `Secret.decode_base64_after` needs a literal prefix and returns a
//! secret, never text.

use super::check;
use crate::errors::{TypeError, TypeErrorKind};

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

/// The one error `src` produces that `pick` selects.
fn the_error(src: &str, pick: impl Fn(&TypeErrorKind) -> bool) -> TypeError {
    let errors = check(src).errors;
    let mut matching = errors.iter().filter(|e| pick(&e.kind));
    let error = matching.next().unwrap_or_else(|| panic!("no matching error; got {errors:?}")).clone();
    assert!(matching.next().is_none(), "more than one matching error: {errors:?}");
    error
}

#[test]
fn an_hmac_takes_a_bytes_or_a_secret_key_and_returns_bytes() {
    assert_clean(
        "agent tag_with_bytes(body: String, key: Bytes) -> Bytes:
    return body.utf8().hmac_sha256(key)

agent tag_with_secret(body: String, key: Secret) -> Bytes:
    return body.utf8().hmac_sha512(key)

agent signature_matches(body: String, key: Secret, received_hex: String) -> Bool:
    return match Bytes.from_hex(received_hex):
        Ok(received) -> body.utf8().hmac_sha256(key) == received
        Err(problem) -> false
",
    );
}

#[test]
fn hmac_is_allowed_in_deterministic_agents() {
    assert_clean(
        "@deterministic
agent tag(body: String, key: Secret) -> String:
    return body.utf8().hmac_sha256(key).to_hex()
",
    );
}

#[test]
fn a_text_or_other_key_is_refused_with_a_hint() {
    for (key, got) in [("\"whsec_test\"", "String"), ("42", "Int"), ("[key]", "List<Bytes>")] {
        let error = the_error(
            &format!("agent tag(body: Bytes, key: Bytes) -> Bytes:\n    return body.hmac_sha256({key})\n"),
            |k| matches!(k, TypeErrorKind::HmacKeyType { .. }),
        );
        assert_eq!(
            error.kind.message(),
            format!("the key of `hmac_sha256` must be `Bytes` or a `Secret`, not `{got}`")
        );
        assert!(error.kind.hint().unwrap_or_default().contains("`secret(name)`"), "{error:?}");
    }
}

#[test]
fn an_hmac_takes_exactly_one_key() {
    let errors = check("agent tag(body: Bytes) -> Bytes:\n    return body.hmac_sha512()\n").errors;
    assert!(
        errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::ArityMismatch { expected: 1, got: 0, .. })),
        "{errors:?}"
    );
}

#[test]
fn a_base64_key_after_a_literal_prefix_is_another_secret() {
    assert_clean(
        "agent signing_key(text: Secret) -> Result<Secret, String>:
    return text.decode_base64_after(\"whsec_\")

agent tag(body: Bytes, text: Secret) -> Result<Bytes, String>:
    key = text.decode_base64_after(\"whsec_\")?
    return Ok(body.hmac_sha256(key))
",
    );
    let errors = check(
        "agent reveal(text: Secret) -> Result<String, String>:\n    return text.decode_base64_after(\"whsec_\")\n",
    )
    .errors;
    assert!(errors.iter().any(|e| matches!(e.kind, TypeErrorKind::ReturnTypeMismatch { .. })), "a key is never text: {errors:?}");
}

#[test]
fn the_prefix_must_be_a_literal() {
    let error = the_error(
        "agent probe(text: Secret, guess: String) -> Bool:\n    return text.decode_base64_after(guess).is_ok()\n",
        |k| matches!(k, TypeErrorKind::SecretKeyPrefixNotLiteral),
    );
    assert_eq!(error.kind.message(), "the prefix passed to `decode_base64_after` must be a string literal");
    assert!(error.kind.hint().unwrap_or_default().contains("one guess at a time"), "{error:?}");
}

#[test]
fn decoding_is_only_for_secrets() {
    let errors = check("agent decode(text: String) -> Bool:\n    return text.decode_base64_after(\"whsec_\").is_ok()\n").errors;
    assert!(!errors.is_empty(), "a String has no `decode_base64_after`; use `Bytes.from_base64`");
}
