//! User credentials C2 — keeping the person's provider token. A provider
//! declared with `keep_token: scopes(...)` keeps the token sign-in
//! returns, and `by.token("<provider>")` on a `Caller` reads it as an
//! `Option<Secret>`. The checker refuses a provider that keeps nothing.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const PROGRAM: &str = r#"identity users:
    provider google
    provider github:
        keep_token: scopes("repo", "read:org")
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        member: "issues.file"

action has_github(by: Caller) -> Bool requires permission("issues.file"):
    return match by.token("github"):
        Some(token) -> true
        None -> false
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

fn caller(id: &str) -> Value {
    let list = |items: &[&str]| Value::List(corvid_driver::ListValue::new(items.iter().map(|s| Value::String((*s).into()))));
    Value::Struct(corvid_driver::StructValue::new(
        corvid_resolve::DefId(0),
        "actor",
        vec![
            ("id".to_string(), Value::String(id.into())),
            ("tenant".to_string(), Value::String("public".into())),
            ("display_name".to_string(), Value::String(id.into())),
            ("roles".to_string(), list(&["member"])),
            ("permissions".to_string(), list(&["issues.file"])),
        ],
    ))
}

#[test]
fn a_kept_provider_publishes_its_scopes_and_a_plain_one_does_not() {
    assert!(errors(PROGRAM).is_empty(), "{:#?}", errors(PROGRAM));
    let contract = compile_to_application_contract_with_config(PROGRAM, "app.cor", "test", None).unwrap();
    let json = serde_json::to_value(&contract.identities[0].providers).unwrap();
    assert_eq!(json[0]["name"], "google");
    assert!(json[0].get("kept_token_scopes").is_none(), "a plain provider keeps nothing: {json}");
    assert_eq!(json[1]["kept_token_scopes"], serde_json::json!(["repo", "read:org"]), "{json}");
}

#[test]
fn reading_a_token_sign_in_never_keeps_is_refused() {
    let wrong = PROGRAM.replace("by.token(\"github\")", "by.token(\"google\")");
    assert!(errors(&wrong).iter().any(|e| e.contains("`by.token(\"google\")`") && e.contains("keep_token")), "{:#?}", errors(&wrong));
    let computed = PROGRAM.replace("by.token(\"github\")", "by.token(\"git\" + \"hub\")");
    assert!(errors(&computed).iter().any(|e| e.contains("keep_token")), "the provider is named literally: {:#?}", errors(&computed));
}

#[tokio::test]
async fn the_token_is_the_persons_own_and_absent_until_kept() {
    let ir = compile_to_ir(PROGRAM).unwrap_or_else(|e| panic!("{e:#?}"));
    let runtime = Runtime::builder().build();
    let run = |id: &str| run_ir_with_runtime(&ir, Some("has_github"), vec![caller(id)], &runtime);
    assert!(matches!(run("ada").await.expect("runs"), Value::Bool(false)), "nothing kept yet");
    runtime.caller_tokens().keep("ada", "github", corvid_crypto::Secret::from_text("gho_ada".to_string()), None);
    assert!(matches!(run("ada").await.expect("runs"), Value::Bool(true)));
    assert!(matches!(run("bo").await.expect("runs"), Value::Bool(false)), "another person's token is never read");
}
