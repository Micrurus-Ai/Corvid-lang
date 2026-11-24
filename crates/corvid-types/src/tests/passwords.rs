//! Tokens and passwords in the checker: the std/crypto module checks,
//! its tools are refused in `@deterministic` agents, and
//! `Secret.verify_password(hash)` is a pure method returning `Bool`.

use super::check;
use crate::errors::TypeErrorKind;

/// The standard library's std/crypto.cor, checked as part of each
/// program so the tests exercise the declarations programs import.
const STD_CRYPTO: &str = include_str!("../../../../std/crypto.cor");

fn with_std_crypto(program: &str) -> String {
    format!("{STD_CRYPTO}\n{program}")
}

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn the_std_crypto_module_checks() {
    assert_clean(STD_CRYPTO);
}

#[test]
fn a_program_issues_a_token_and_checks_a_password() {
    assert_clean(&with_std_crypto(
        "tool deliver_key(owner: String, key: Secret) -> Bool

agent issue_api_key(owner: String) -> String:
    key = random_token(32)
    hash = hash_password(key, owasp_2023())
    delivered = deliver_key(owner, key)
    return hash

agent login(password: Secret, stored_hash: String) -> Bool:
    return password.verify_password(stored_hash)

agent stronger(password: Secret) -> String:
    return hash_password(password, PasswordCost(65536, 3, 1))
",
    ));
}

#[test]
fn verify_password_is_allowed_in_deterministic_agents() {
    assert_clean(&with_std_crypto(
        "@deterministic
agent login(password: Secret, stored_hash: String) -> Bool:
    cost = owasp_2023()
    return password.verify_password(stored_hash) and cost.lanes == 1
",
    ));
}

#[test]
fn deterministic_agents_refuse_the_tools_that_draw_entropy() {
    for call in ["random_token(32) == password", "hash_password(password, owasp_2023()) == \"\""] {
        let errors = check(&with_std_crypto(&format!(
            "@deterministic\nagent draw(password: Secret) -> Bool:\n    return {call}\n"
        )))
        .errors;
        assert!(
            errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::NonDeterministicCall { .. })),
            "{call}: {errors:?}"
        );
    }
}

#[test]
fn verify_password_takes_a_string_hash_and_returns_bool() {
    let errors = check(&with_std_crypto(
        "agent login(password: Secret, stored: Bytes) -> Bool:
    return password.verify_password(stored)
",
    ))
    .errors;
    assert!(!errors.is_empty(), "a Bytes hash is refused");
    let errors = check(&with_std_crypto(
        "agent login(password: Secret, stored_hash: String) -> String:
    return password.verify_password(stored_hash)
",
    ))
    .errors;
    assert!(!errors.is_empty(), "the result is a Bool, not text");
}

#[test]
fn a_token_is_a_secret_that_cannot_become_text() {
    let errors = check(&with_std_crypto(
        "agent leak() -> String:
    return \"key: \" + random_token(32)
",
    ))
    .errors;
    assert!(
        errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::SecretOperatorMismatch { .. })),
        "{errors:?}"
    );
    let errors = check(&with_std_crypto(
        "agent leak() -> String:
    return random_token(32).to_string()
",
    ))
    .errors;
    assert!(errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::SecretMethod { .. })), "{errors:?}");
}

#[test]
fn a_password_must_be_a_secret_not_text() {
    let errors = check(&with_std_crypto(
        "agent careless(password: String) -> String:
    return hash_password(password, owasp_2023())
",
    ))
    .errors;
    assert!(!errors.is_empty(), "a String password is refused");
}
