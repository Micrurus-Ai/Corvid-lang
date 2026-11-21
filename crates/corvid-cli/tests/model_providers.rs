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

#[derive(Default)]
struct StreamProbe {
    release: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    retries: std::sync::atomic::AtomicUsize,
}

async fn stream_provider(
    State(probe): State<Arc<StreamProbe>>,
    Json(request): Json<Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    assert_eq!(request["stream"], true);
    assert_eq!(request["stream_options"]["include_usage"], true);
    let mode: String = serde_json::from_str(request["messages"][0]["content"].as_str().unwrap())
        .expect("the prompt interpolates a Corvid string as a quoted value");
    assert!(matches!(
        mode.as_str(),
        "normal" | "cancel" | "retry" | "budget" | "broken" | "slow"
    ));
    if mode == "retry" && probe.retries.fetch_add(1, Ordering::SeqCst) == 0 {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    struct OnDrop(Arc<StreamProbe>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.cancelled.notify_one();
        }
    }
    let stream = async_stream::stream! {
        let _on_drop = (mode == "cancel").then(|| OnDrop(probe.clone()));
        if mode == "slow" { tokio::time::sleep(std::time::Duration::from_secs(4)).await; }
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"caf\u{e9}\"}}]}\n\n";
        let cut = event.find('\u{e9}').unwrap() + 1;
        yield Ok::<_, std::convert::Infallible>(event.as_bytes()[..cut].to_vec());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        yield Ok(event.as_bytes()[cut..].to_vec());
        if mode == "normal" { probe.release.notified().await; }
        if mode == "cancel" {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                yield Ok(b": keep-alive\n\n".to_vec());
            }
        }
        if mode == "broken" { return; }
        yield Ok(b"data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n\n".to_vec());
        let usage = if mode == "budget" { 21 } else { 2 };
        yield Ok(format!("data: {{\"choices\":[],\"usage\":{{\"completion_tokens\":{usage}}}}}\n\ndata: [DONE]\n\n").into_bytes());
    };
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamed_prompt_reaches_http_consumer_before_provider_finishes() {
    use futures::StreamExt;
    use std::time::Duration;
    let probe = Arc::new(StreamProbe::default());
    let app = Router::new()
        .route("/v1/chat/completions", post(stream_provider))
        .with_state(probe.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = format!(
        "openai-compat:http://{}/v1:local-model",
        listener.local_addr().unwrap()
    );
    let provider = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let project = tempfile::tempdir().unwrap();
    let source = project.path().join("main.cor");
    std::fs::write(
        &source,
        r#"prompt generate(mode: String) -> Stream<String>:
    with max_tokens 20
    "{mode}"

server stream_api:
    route GET "/answer/{mode}" -> json Stream<String>:
        return try generate(path.mode) timeout 700 on error retry 2 times backoff linear 1
"#,
    )
    .unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr = format!("127.0.0.1:{port}");
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Server(
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(project.path())
            .arg("serve")
            .arg(&source)
            .arg("--listen")
            .arg(&addr)
            .env("CORVID_MODEL", model)
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while client
        .get(format!("http://{addr}/healthz"))
        .send()
        .await
        .is_err()
    {
        assert!(child.0.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for mode in ["normal", "cancel", "retry", "budget", "broken", "slow"] {
        let response = client
            .get(format!("http://{addr}/answer/{mode}"))
            .send()
            .await
            .unwrap();
        let mut chunks = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = chunks.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
            if bytes
                .windows("caf\u{e9}".len())
                .any(|s| s == "caf\u{e9}".as_bytes())
            {
                if mode == "normal" {
                    probe.release.notify_one();
                }
                if mode == "cancel" {
                    break;
                }
            }
        }
        drop(chunks);
        let content = String::from_utf8(bytes).unwrap();
        match mode {
            "normal" | "retry" => {
                assert!(
                    content.contains("caf\u{e9}")
                        && content.contains("!")
                        && content.contains("event: done"),
                    "{mode}: {content}"
                );
            }
            "cancel" => tokio::time::timeout(Duration::from_secs(12), probe.cancelled.notified())
                .await
                .expect("upstream stream stayed open after client disconnected"),
            "budget" => assert!(
                content.contains("21") && content.contains("error"),
                "{content}"
            ),
            "broken" => assert!(
                content.contains("before [DONE]") && content.contains("error"),
                "{content}"
            ),
            "slow" => assert!(
                content.contains("700") && !content.contains("caf\u{e9}"),
                "{content}"
            ),
            _ => unreachable!(),
        }
    }
    assert_eq!(probe.retries.load(Ordering::SeqCst), 2);
    provider.abort();
}
