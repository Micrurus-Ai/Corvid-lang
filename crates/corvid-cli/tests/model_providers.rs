//! Run the same typed Corvid program against two local provider protocols.
use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

async fn provider(
    State(malformed): State<Arc<AtomicU8>>,
    uri: axum::http::Uri,
    Json(request): Json<Value>,
) -> Json<Value> {
    let compat = uri.path() == "/v1/chat/completions";
    assert_eq!(request["model"], "local-model");
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "user");
    assert!(messages[1]["content"].as_str().unwrap().contains("parcel"));
    let schema = if compat {
        &request["response_format"]["json_schema"]["schema"]
    } else {
        &request["format"]
    };
    let output = if schema.is_null() {
        assert_eq!(messages[0]["content"], "Summarize briefly.");
        "delayed".to_string()
    } else {
        assert_eq!(schema["properties"]["urgent"]["type"], "boolean");
        assert_eq!(schema["properties"]["reason"]["type"], "string");
        assert_eq!(messages[0]["content"], "Classify the message.");
        match malformed.load(Ordering::SeqCst) {
            1 => json!({"urgent": "maybe", "reason":"late"}).to_string(),
            2 => "{".into(),
            _ => json!({"urgent":true, "reason":"late"}).to_string(),
        }
    };
    let reply = json!({"role":"assistant", "content":output});
    Json(if compat {
        json!({"choices":[{"message":reply}]})
    } else {
        json!({"message":reply,"done":true})
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_providers_without_changing_typed_prompts() {
    let malformed = Arc::new(AtomicU8::new(0));
    let app = Router::new()
        .route("/v1/chat/completions", post(provider))
        .route("/api/chat", post(provider))
        .with_state(malformed.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("main.cor"),
        r#"effect model_call:
    reversible: true

type Decision:
    urgent: Bool
    reason: String

prompt summarize(text: String) -> String uses model_call:
    system: "Summarize briefly."
    user: "{text}"

prompt classify(text: String) -> Decision uses model_call:
    system: "Classify the message."
    user: "{text}"

agent main() -> String:
    summary = summarize("parcel")
    decision = classify("parcel")
    if decision.urgent:
        return summary + ": " + decision.reason
    return "normal"
"#,
    )
    .unwrap();

    for model in [
        format!("openai-compat:{base}/v1:local-model"),
        "ollama:local-model".into(),
    ] {
        let mut run = Command::new(env!("CARGO_BIN_EXE_corvid"));
        run.current_dir(project.path())
            .args(["run", "--target", "interp", "main.cor"])
            .env("CORVID_MODEL", &model)
            .env("OLLAMA_BASE_URL", &base)
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .env("CORVID_TEST_MOCK_LLM", "0");
        malformed.store(0, Ordering::SeqCst);
        let output = run.output().unwrap();
        assert!(output.status.success(), "{model}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("delayed: late"),
            "{output:?}"
        );
        for (mode, diagnostic) in [(1, "bool"), (2, "json")] {
            malformed.store(mode, Ordering::SeqCst);
            let bad = run.output().unwrap();
            assert!(!bad.status.success(), "{model}: {bad:?}");
            let message = String::from_utf8_lossy(&bad.stderr);
            assert!(message.to_lowercase().contains(diagnostic), "{model}: {message}");
        }
    }
    server.abort();
}
