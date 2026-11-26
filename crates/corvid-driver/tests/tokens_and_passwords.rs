//! End to end: a program imports std/crypto and std/secrets, hashes a
//! password read with `secret(name)` at the declared OWASP 2023 cost,
//! verifies the right and a wrong password, and issues an API key with
//! `random_token`. The recorded trace holds neither the password nor
//! the key; replay substitutes the recorded hash and draws a fresh
//! token with a warning, and reports a divergence when the run's result
//! depends on the token's value. `@deterministic` refuses the two tools
//! and allows `verify_password`.

#[path = "support/std_project.rs"]
mod std_project;

use corvid_driver::run_ir_with_runtime;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::{Arc, Mutex};

const STD_FILES: &[&str] = &["effects.cor", "secrets.cor", "crypto.cor"];

const SOURCE: &str = r#"
import "./std/secrets" use secret
import "./std/crypto" use random_token, hash_password, owasp_2023, PasswordCost

tool deliver_key(owner: String, key: Secret) -> Bool

# Sign-up: store the hash, never the password.
agent register(name: String) -> Result<String, String>:
    password = secret(name)?
    return Ok(hash_password(password, owasp_2023()))

# Log-in: check a password against the stored hash.
agent login(name: String, stored_hash: String) -> Result<Bool, String>:
    password = secret(name)?
    return Ok(password.verify_password(stored_hash))

# API-key issuance: deliver the key to its owner once, through a tool
# that takes a Secret, and keep only its hash.
agent issue_api_key(owner: String) -> String:
    key = random_token(32)
    delivered = deliver_key(owner, key)
    return hash_password(key, owasp_2023())

# A run whose result depends on the token's value.
agent issue_and_check() -> Bool:
    key = random_token(32)
    hash = hash_password(key, owasp_2023())
    return key.verify_password(hash)

agent declared_cost() -> PasswordCost:
    return owasp_2023()
"#;

const PASSWORD: &str = "correct horse battery staple, from the test";

fn compile() -> corvid_ir::IrFile {
    std_project::compile_with_std(SOURCE, STD_FILES)
}

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn approving() -> corvid_runtime::RuntimeBuilder {
    Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes()))
}

fn ok(value: Value) -> Value {
    match value {
        Value::ResultOk(inner) => inner.get(),
        other => panic!("expected Ok, got {other:?}"),
    }
}

fn string(value: Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_password_is_hashed_and_verified_and_never_reaches_the_trace() {
    std::env::set_var("CORVID_TOKENS_TEST_PASSWORD", PASSWORD);
    std::env::set_var("CORVID_TOKENS_TEST_WRONG_PASSWORD", "correct horse battery staple, from someone else");
    let ir = compile();

    let trace_dir = tempfile::tempdir().unwrap();
    let record = approving().trace_to(trace_dir.path()).build();
    let hash = string(ok(run_ir_with_runtime(&ir, Some("register"), vec![text("CORVID_TOKENS_TEST_PASSWORD")], &record)
        .await
        .expect("the registration runs")));
    assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{hash}");
    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    let trace = std::fs::read_to_string(&trace_path).expect("the trace file");
    assert!(!trace.contains(PASSWORD), "the trace must never hold the password:\n{trace}");
    assert!(!trace.contains("battery staple"), "not even part of it:\n{trace}");
    assert!(trace.contains(r#""args":["<redacted>",{"#), "the hash_password call records <redacted>:\n{trace}");
    assert!(trace.contains(&hash), "the hash, a verifier, is recorded:\n{trace}");

    let rt = approving().build();
    let login = |name: &'static str, stored: String| {
        let ir = &ir;
        let rt = &rt;
        async move {
            ok(run_ir_with_runtime(ir, Some("login"), vec![text(name), text(&stored)], rt).await.expect("the login runs"))
        }
    };
    assert_eq!(login("CORVID_TOKENS_TEST_PASSWORD", hash.clone()).await, Value::Bool(true), "the right password verifies");
    assert_eq!(login("CORVID_TOKENS_TEST_WRONG_PASSWORD", hash.clone()).await, Value::Bool(false), "a wrong one does not");
    assert_eq!(login("CORVID_TOKENS_TEST_PASSWORD", "$argon2id$garbage".into()).await, Value::Bool(false), "a malformed hash verifies nothing");

    // Replay re-reads the password and substitutes the recorded hash:
    // the recorded and replayed calls both hold `<redacted>`.
    let replay = approving().replay_from(&trace_path).build();
    let replayed = string(ok(run_ir_with_runtime(&ir, Some("register"), vec![text("CORVID_TOKENS_TEST_PASSWORD")], &replay)
        .await
        .expect("the registration replays")));
    assert_eq!(replayed, hash);
    assert!(replay.replay_redrawn_token_warnings().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_issued_api_key_reaches_its_owner_once_and_replay_draws_a_fresh_one_with_a_warning() {
    let ir = compile();
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let seen = delivered.clone();
    let trace_dir = tempfile::tempdir().unwrap();
    let record = approving()
        .tool("deliver_key", move |args| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push(args.clone());
                Ok(serde_json::json!(true))
            }
        })
        .trace_to(trace_dir.path())
        .build();
    let hash = string(run_ir_with_runtime(&ir, Some("issue_api_key"), vec![text("ada@example.com")], &record)
        .await
        .expect("the issuance runs"));
    let received = delivered.lock().unwrap().clone();
    assert_eq!(received.len(), 1, "the key is delivered once");
    let key = received[0][1].as_str().expect("the tool receives the key's text").to_string();
    assert_eq!(key.len(), 43);
    assert_eq!(corvid_crypto::verify_password(key.as_bytes(), &hash), Ok(true), "the stored hash is the key's");

    let trace_path = record.tracer().path().to_path_buf();
    drop(record);
    let trace = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!trace.contains(&key), "the trace must never hold the key:\n{trace}");
    assert!(trace.contains(r#""tool":"random_token","result":"<redacted>""#), "{trace}");
    assert!(trace.contains(r#""args":["ada@example.com","<redacted>"]"#), "{trace}");

    // Replay has no token to substitute, so it draws a fresh one and
    // warns. The delivery and the hash are substituted (their recorded
    // arguments hold `<redacted>` where the token was), so the replayed
    // run returns the recorded hash and calls no tool.
    let replay = approving()
        .tool("deliver_key", |_args| async move { panic!("replay must substitute the recorded delivery") })
        .replay_from(&trace_path)
        .build();
    let replayed = string(run_ir_with_runtime(&ir, Some("issue_api_key"), vec![text("ada@example.com")], &replay)
        .await
        .expect("the issuance replays"));
    assert_eq!(replayed, hash);
    let warnings = replay.replay_redrawn_token_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("`random_token`") && warnings[0].contains("never stored"), "{}", warnings[0]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_reports_a_divergence_when_the_result_depends_on_the_token() {
    let ir = compile();
    let trace_dir = tempfile::tempdir().unwrap();
    let record = approving().trace_to(trace_dir.path()).build();
    let live = run_ir_with_runtime(&ir, Some("issue_and_check"), vec![], &record).await.expect("the live run");
    assert_eq!(live, Value::Bool(true));
    let trace_path = record.tracer().path().to_path_buf();
    drop(record);

    // The replayed token is fresh, the substituted hash is the recorded
    // token's, so verification fails and the result differs from the
    // recording. Replay says so instead of passing silently.
    let replay = approving().replay_from(&trace_path).build();
    let error = run_ir_with_runtime(&ir, Some("issue_and_check"), vec![], &replay)
        .await
        .expect_err("the replayed result differs from the recording")
        .to_string();
    assert!(error.contains("run_completed") && error.contains("false"), "{error}");
    assert_eq!(replay.replay_redrawn_token_warnings().len(), 1);
}

/// std/crypto writes the OWASP 2023 cost as Corvid source; corvid-crypto
/// holds the same numbers for the runtime. The two must stay equal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_declared_owasp_cost_matches_the_one_the_runtime_uses() {
    let ir = compile();
    let value = run_ir_with_runtime(&ir, Some("declared_cost"), vec![], &approving().build()).await.expect("runs");
    let json = corvid_vm::value_to_json(&value);
    let expected = corvid_crypto::PasswordCost::OWASP_2023;
    assert_eq!(
        json,
        serde_json::json!({
            "memory_kib": expected.memory_kib(),
            "iterations": expected.iterations(),
            "lanes": expected.lanes(),
        })
    );
    assert_eq!(corvid_runtime::auth::API_KEY_HASH_COST, expected, "the API-key store hashes at the same cost");
}

#[test]
fn deterministic_agents_refuse_the_tools_and_allow_verify_password() {
    let allowed = r#"
import "./std/crypto" use owasp_2023

@deterministic
agent login(password: Secret, stored_hash: String) -> Bool:
    cost = owasp_2023()
    return cost.lanes == 1 and password.verify_password(stored_hash)
"#;
    std_project::try_compile_with_std(allowed, STD_FILES).expect("verify_password is pure");

    for (tool, call) in [("random_token", "random_token(32) == password"), ("hash_password", "hash_password(password, owasp_2023()) == \"\"")] {
        let source = format!(
            "import \"./std/crypto\" use random_token, hash_password, owasp_2023\n\n@deterministic\nagent draw(password: Secret) -> Bool:\n    return {call}\n"
        );
        let errors = std_project::try_compile_with_std(&source, STD_FILES).expect_err("refused");
        assert!(
            errors.iter().any(|e| e.message.contains("`@deterministic`") && e.message.contains(&format!("`{tool}` (tool)"))),
            "{errors:?}"
        );
    }
}
