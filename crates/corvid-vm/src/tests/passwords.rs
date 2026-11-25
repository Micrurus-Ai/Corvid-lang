//! Tokens and passwords on the interpreter, run from Corvid source with
//! the declarations of std/crypto: `random_token` produces a `Secret`
//! that tools receive as text and the trace records as `<redacted>`;
//! `hash_password` hashes at the declared cost with the password
//! redacted in its record; `verify_password` checks a secret against a
//! hash.

use super::*;

const STD_CRYPTO: &str = include_str!("../../../../std/crypto.cor");

fn traced_runtime(dir: &std::path::Path, run_id: &str, tools: impl FnOnce(corvid_runtime::RuntimeBuilder) -> corvid_runtime::RuntimeBuilder) -> Runtime {
    tools(
        Runtime::builder()
            .approver(Arc::new(ProgrammaticApprover::always_yes()))
            .tracer(corvid_runtime::Tracer::open(dir, run_id)),
    )
    .build()
}

#[tokio::test]
async fn an_issued_token_is_delivered_hashed_and_verified_without_reaching_the_trace() {
    let src = format!(
        "{STD_CRYPTO}
tool deliver_key(key: Secret) -> Bool

agent main() -> List<Bool>:
    key = random_token(32)
    other = random_token(32)
    hash = hash_password(key, owasp_2023())
    delivered = deliver_key(key)
    return [delivered, key.verify_password(hash), other.verify_password(hash), key == other, key.verify_password(\"not a hash\")]
"
    );
    let ir = ir_of(&src);
    let dir = tempfile::tempdir().unwrap();
    let delivered = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = delivered.clone();
    let rt = traced_runtime(dir.path(), "issued-token", |builder| {
        builder.tool("deliver_key", move |args| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(args[0].as_str().unwrap().to_string());
                Ok(json!(true))
            }
        })
    });
    let result = run_agent(&ir, "main", vec![], &rt).await.expect("run");
    let Value::List(items) = result else { panic!("expected a list, got {result:?}") };
    assert_eq!(
        items.iter_cloned(),
        vec![Value::Bool(true), Value::Bool(true), Value::Bool(false), Value::Bool(false), Value::Bool(false)]
    );
    let token = delivered.lock().unwrap()[0].clone();
    assert_eq!(token.len(), 43, "32 bytes in unpadded base64url: {token}");
    assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{token}");

    let trace = std::fs::read_to_string(dir.path().join("issued-token.jsonl")).unwrap();
    assert!(!trace.contains(&token), "the trace holds the token:\n{trace}");
    let token_results: Vec<&str> = trace
        .lines()
        .filter(|line| line.contains(r#""kind":"tool_result""#) && line.contains(r#""tool":"random_token""#))
        .collect();
    assert_eq!(token_results.len(), 2, "{trace}");
    for line in token_results {
        assert!(line.contains(r#""result":"<redacted>""#), "{line}");
    }
    let hash_call = trace
        .lines()
        .find(|line| line.contains(r#""kind":"tool_call""#) && line.contains(r#""tool":"hash_password""#))
        .expect("the hash_password call is recorded");
    assert!(hash_call.contains(r#""args":["<redacted>","#), "{hash_call}");
    assert!(hash_call.contains(r#""memory_kib":19456"#), "the declared cost is recorded: {hash_call}");
    assert!(trace.contains("$argon2id$v=19$m=19456,t=2,p=1$"), "the hash, a verifier, is recorded:\n{trace}");
}

#[tokio::test]
async fn a_token_shorter_than_sixteen_bytes_stops_the_run() {
    let src = format!(
        "{STD_CRYPTO}
agent main() -> Bool:
    key = random_token(8)
    return key == key
"
    );
    let ir = ir_of(&src);
    let error = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect_err("refused").to_string();
    assert!(error.contains("random_token(8) is too short to be safe"), "{error}");
    assert!(error.contains("at least 16 random bytes"), "{error}");
}

#[tokio::test]
async fn a_cost_below_the_owasp_minimum_stops_the_hash() {
    let src = format!(
        "{STD_CRYPTO}
agent main() -> String:
    key = random_token(32)
    return hash_password(key, PasswordCost(4096, 1, 1))
"
    );
    let ir = ir_of(&src);
    let error = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect_err("refused").to_string();
    assert!(error.contains("below the OWASP 2023 minimum for Argon2id"), "{error}");
    assert!(error.contains("owasp_2023()"), "{error}");
}

#[tokio::test]
async fn a_higher_declared_cost_is_recorded_in_the_hash_and_still_verifies() {
    let src = format!(
        "{STD_CRYPTO}
agent main() -> Bool:
    key = random_token(16)
    hash = hash_password(key, PasswordCost(47104, 1, 1))
    return hash.starts_with(\"$argon2id$v=19$m=47104,t=1,p=1$\") and key.verify_password(hash)
"
    );
    let ir = ir_of(&src);
    assert_eq!(run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run"), Value::Bool(true));
}
