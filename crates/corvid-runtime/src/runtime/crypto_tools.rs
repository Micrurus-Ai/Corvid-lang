//! The executing std/crypto tools: `random_token` and `hash_password`.
//!
//! Every rule is `corvid_crypto`'s (the token bounds, the entropy
//! source, the password-cost floor, Argon2id); this file only reads the
//! tools' JSON arguments and reports failures as tool errors.
//!
//! The two tools record and replay differently, because of what their
//! records hold:
//!
//! - **`random_token`** returns a `Secret`, so its recorded result is
//!   `<redacted>` and the trace never holds the token. Replay cannot
//!   substitute a value it never stored, so it draws a fresh token and
//!   warns (`Runtime::replay_redrawn_token_warnings`): a replayed run
//!   continues with a different token than the recorded one, and where
//!   that changes what the run computes, replay reports the divergence.
//! - **`hash_password`** takes the password as a `Secret` (recorded as
//!   `<redacted>`) and returns the hash, a verifier rather than the
//!   password, so its result is recorded and replay substitutes it like
//!   any tool result.
//!
//! Hashing runs on the blocking pool: Argon2id at the OWASP cost holds
//! 19 MiB and a core for tens of milliseconds, which must not stall the
//! async workers serving other requests.

use crate::errors::RuntimeError;
use serde_json::Value;

/// Exact-name gate for `random_token(bytes) -> Secret`. The interpreter
/// reads it through `is_stdlib_secret_value_tool`, which includes it.
pub(super) fn is_random_token_tool(name: &str) -> bool {
    name == "random_token"
}

/// Exact-name gate for `hash_password(password, cost) -> String`.
pub(super) fn is_password_hash_tool(name: &str) -> bool {
    name == "hash_password"
}

/// `random_token(bytes)`: the token's text, which the interpreter holds
/// as a `Secret` and the trace records as `<redacted>`.
pub(super) fn random_token(args: &[Value]) -> Result<Value, RuntimeError> {
    let failed = |message: String| RuntimeError::ToolFailed { tool: "random_token".into(), message };
    let bytes = match args {
        [bytes] => bytes.as_i64().ok_or_else(|| failed("expected one Int argument (a number of bytes)".into()))?,
        _ => return Err(failed("expected one Int argument (a number of bytes)".into())),
    };
    let token = corvid_crypto::random_token(bytes).map_err(failed)?;
    let text = token.expose_text().expect("a random token is text").to_string();
    Ok(Value::String(text))
}

/// `hash_password(password, cost)`: the Argon2id PHC string. `args` are
/// the revealed arguments, so the password is its text.
pub(super) async fn hash_password(args: &[Value]) -> Result<Value, RuntimeError> {
    let failed = |message: String| RuntimeError::ToolFailed { tool: "hash_password".into(), message };
    let [password, cost] = args else {
        return Err(failed("expected (password: Secret, cost: PasswordCost)".into()));
    };
    let password = password
        .as_str()
        .ok_or_else(|| failed("the password must be a Secret holding text".into()))?
        .to_owned();
    let field = |name: &str| {
        cost.get(name)
            .and_then(Value::as_i64)
            .ok_or_else(|| failed(format!("the cost has no Int field `{name}`; build it with owasp_2023() or PasswordCost(memory_kib, iterations, lanes)")))
    };
    let cost = corvid_crypto::PasswordCost::new(field("memory_kib")?, field("iterations")?, field("lanes")?)
        .map_err(failed)?;
    let hashed = tokio::task::spawn_blocking(move || corvid_crypto::hash_password(password.as_bytes(), cost))
        .await
        .map_err(|problem| failed(format!("the hashing task stopped: {problem}")))?
        .map_err(failed)?;
    Ok(Value::String(hashed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owasp() -> Value {
        json!({"memory_kib": 19456, "iterations": 2, "lanes": 1})
    }

    #[test]
    fn a_token_is_text_of_the_requested_size() {
        let Value::String(token) = random_token(&[json!(32)]).unwrap() else { panic!() };
        assert_eq!(token.len(), 43);
        let problem = random_token(&[json!(8)]).unwrap_err().to_string();
        assert!(problem.contains("at least 16 random bytes"), "{problem}");
        assert!(random_token(&[json!("32")]).is_err());
        assert!(random_token(&[]).is_err());
    }

    #[tokio::test]
    async fn a_password_hashes_at_its_declared_cost_and_verifies() {
        let Value::String(hash) = hash_password(&[json!("correct horse"), owasp()]).await.unwrap() else { panic!() };
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{hash}");
        assert_eq!(corvid_crypto::verify_password(b"correct horse", &hash), Ok(true));
    }

    #[tokio::test]
    async fn a_cost_below_the_floor_or_missing_a_field_is_a_tool_error() {
        let weak = json!({"memory_kib": 4096, "iterations": 1, "lanes": 1});
        let problem = hash_password(&[json!("pw"), weak]).await.unwrap_err().to_string();
        assert!(problem.contains("below the OWASP 2023 minimum"), "{problem}");
        let partial = json!({"memory_kib": 19456, "iterations": 2});
        let problem = hash_password(&[json!("pw"), partial]).await.unwrap_err().to_string();
        assert!(problem.contains("`lanes`"), "{problem}");
    }
}
