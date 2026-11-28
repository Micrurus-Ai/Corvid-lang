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

traces: none

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
                        && content.contains("event: completed\ndata: null"),
                    "{mode}: {content}"
                );
            }
            "cancel" => tokio::time::timeout(Duration::from_secs(12), probe.cancelled.notified())
                .await
                .expect("upstream stream stayed open after client disconnected"),
            // A crossed bound ends the stream with `failed`, naming the
            // bound the program declared.
            "budget" => assert!(
                content.contains("event: failed")
                    && content.contains("\"error\":\"token_limit_exceeded\"")
                    && content.contains("limit of 20 tokens"),
                "{content}"
            ),
            // A provider that breaks off ends the stream with `failed`,
            // and the provider's own message stays in the server log.
            "broken" => assert!(
                content.contains("event: failed")
                    && content.contains("\"error\":\"handler_failed\"")
                    && !content.contains("[DONE]"),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_function_handles_model_actions_in_a_bounded_loop() {
    use std::sync::atomic::AtomicUsize;

    async fn next_action(
        State(calls): State<Arc<AtomicUsize>>,
        Json(request): Json<Value>,
    ) -> Json<Value> {
        calls.fetch_add(1, Ordering::SeqCst);
        let input = request["messages"][0]["content"].as_str().unwrap();
        let schema = &request["response_format"]["json_schema"]["schema"]["properties"];
        let variants = schema["action"]["oneOf"].as_array().unwrap();
        assert_eq!(variants[0]["properties"]["variant"]["const"], "Add");
        assert_eq!(
            variants[0]["properties"]["fields"]["prefixItems"][0]["type"],
            "integer"
        );
        assert_eq!(variants[1]["properties"]["fields"]["maxItems"], 0);
        let action = if input.contains("malformed") {
            json!({"action":{"variant":"Add","fields":["two"]}})
        } else if input.contains("invalid") {
            json!({"action":{"variant":"Add","fields":[-2]}})
        } else if input.contains("unknown") {
            json!({"action":{"variant":"Erase","fields":[]}})
        } else if input.contains("Task \"finish\"") && input.contains("total=2") {
            json!({"action":{"variant":"Finish","fields":[]}})
        } else {
            json!({"action":{"variant":"Add","fields":[2]}})
        };
        Json(json!({"choices":[{"message":{"role":"assistant","content":action.to_string()}}]}))
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = format!(
        "openai-compat:http://{}/v1:local-model",
        listener.local_addr().unwrap()
    );
    let app = Router::new()
        .route("/v1/chat/completions", post(next_action))
        .with_state(calls.clone());
    let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let project = tempfile::tempdir().unwrap();
    let source = r#"type Action:
    | Add(amount: Int)
    | Finish

type Decision:
    action: Action

fn add(total: Int, amount: Int) -> Result<Int, String>:
    if amount < 0:
        return Err("negative amount")
    return Ok(total + amount)

prompt choose(task: String, total: Int) -> Decision:
    "Task {task}, total={total}. Choose add or finish."

agent run_actions(task: String) -> Result<Int, String>:
    total = 0
    for turn in [1, 2, 3]:
        step = choose(task, total)
        finished = match step.action:
            Finish -> true
            Add(_) -> false
        if finished:
            return Ok(total)
        result = match step.action:
            Add(amount) -> add(total, amount)
            Finish -> Ok(total)
        total = result?
    return Err("step limit")

agent main() -> String:
    return match run_actions("SCENARIO"):
        Ok(value) -> value.to_string()
        Err(reason) -> reason
"#;
    for (task, expected, count) in [
        ("finish", "2", 2),
        ("forever", "step limit", 3),
        ("invalid", "negative amount", 1),
        ("unknown", "Erase", 1),
        ("malformed", "Int", 1),
    ] {
        calls.store(0, Ordering::SeqCst);
        std::fs::write(
            project.path().join("main.cor"),
            source.replace("SCENARIO", task),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(project.path())
            .args(["run", "--target", "interp", "main.cor"])
            .env("CORVID_MODEL", &model)
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            !matches!(task, "malformed" | "unknown"),
            "{task}: {output:?}"
        );
        let message = if output.status.success() {
            &output.stdout
        } else {
            &output.stderr
        };
        assert!(
            String::from_utf8_lossy(message).contains(expected),
            "{task}: {output:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), count, "{task}");
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_tool_calls_use_corvid_signatures_and_return_matching_ids() {
    use std::sync::atomic::AtomicUsize;
    async fn chat(State(count): State<Arc<AtomicUsize>>, Json(req): Json<Value>) -> Json<Value> {
        let n = count.fetch_add(1, Ordering::SeqCst);
        let tool = &req["tools"][0]["function"];
        assert_eq!(tool["name"], "calculate");
        assert_eq!(tool["parameters"]["properties"]["left"]["type"], "integer");
        assert_eq!(tool["parameters"]["required"], json!(["left", "right"]));
        let history = req["messages"].as_array().unwrap();
        let mode = history[0]["content"].as_str().unwrap();
        let message = if n == 0 || mode.contains("forever") {
            let name = if mode.contains("unknown") {
                "other"
            } else {
                "calculate"
            };
            let left = if mode.contains("invalid") {
                json!("two")
            } else {
                json!(2)
            };
            let call = json!({"id":format!("call_{n}"),"type":"function","function":{
                "name":name, "arguments":json!({"right":3,"left":left}).to_string()
            }});
            let mut batch = vec![call.clone()];
            if mode.contains("duplicate") {
                batch.push(call);
            }
            if mode.contains("batch") {
                let mut second = batch[0].clone();
                second["function"]["arguments"] = json!(r#"{"left":6,"right":7}"#);
                second["id"] = json!("call_second");
                batch.push(second);
            }
            json!({"role":"assistant","content":null,"tool_calls":batch})
        } else {
            assert_eq!(history[1]["tool_calls"][0]["id"], "call_0");
            assert_eq!(
                history[2],
                json!({"role":"tool","tool_call_id":"call_0","content":"5"})
            );
            if mode.contains("batch") {
                assert_eq!(
                    history[3],
                    json!({"role":"tool","tool_call_id":"call_second","content":"13"})
                );
            }
            json!({"role":"assistant","content":"the answer is five"})
        };
        Json(json!({"choices":[{"message":message}]}))
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = format!(
        "openai-compat:http://{}/v1:local-model",
        listener.local_addr().unwrap()
    );
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(calls.clone());
    let server = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    for (mode, expected, requests) in [
        ("normal", "the answer is five", 2),
        ("batch", "the answer is five", 2),
        ("duplicate", "repeated function call", 1),
        ("invalid", "calculate.left", 1),
        ("unknown", "unknown prompt function", 1),
        ("forever", "limit of 8", 9),
    ] {
        calls.store(0, Ordering::SeqCst);
        let source = r#"fn add(left: Int, right: Int) -> Int:
    return left + right

prompt answer(question: String, calculate: (Int, Int) -> Int) -> String:
    "{question}"

agent main() -> String:
    return answer("SCENARIO", add)
"#
        .replace("SCENARIO", mode);
        std::fs::write(dir.path().join("main.cor"), source).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(dir.path())
            .args(["run", "main.cor"])
            .env("CORVID_MODEL", &model)
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            matches!(mode, "normal" | "batch"),
            "{mode}: {result:?}"
        );
        let text = if result.status.success() {
            &result.stdout
        } else {
            &result.stderr
        };
        assert!(
            String::from_utf8_lossy(text).contains(expected),
            "{mode}: {result:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), requests);
    }
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_worker_resumes_tool_result_after_process_exit() {
    use std::{process::Stdio, sync::atomic::AtomicUsize, time::Duration};
    #[derive(Default)]
    struct Probe {
        first_turns: AtomicUsize,
        followups: AtomicUsize,
        waiting: tokio::sync::Notify,
    }
    async fn chat(State(probe): State<Arc<Probe>>, Json(req): Json<Value>) -> Json<Value> {
        let messages = req["messages"].as_array().unwrap();
        let response = if messages.len() == 1 {
            probe.first_turns.fetch_add(1, Ordering::SeqCst);
            json!({"role":"assistant","tool_calls":[{"id":"add-123","type":"function",
                "function":{"name":"calculate","arguments":"{\"n\":21}"}}]})
        } else {
            assert_eq!(req["tool_choice"], "none");
            assert_eq!(
                messages[2],
                json!({"role":"tool","content":"42","tool_call_id":"add-123"})
            );
            if probe.followups.fetch_add(1, Ordering::SeqCst) == 0 {
                probe.waiting.notify_one();
                std::future::pending::<()>().await;
            }
            json!({"role":"assistant","content":"forty-two"})
        };
        Json(json!({"choices":[{"message":response}]}))
    }
    let probe = Arc::new(Probe::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = format!(
        "openai-compat:http://{}/v1:local-model",
        listener.local_addr().unwrap()
    );
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(probe.clone());
    let provider = tokio::spawn(async { axum::serve(listener, app).await.unwrap() });
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.cor"),
        r#"fn twice(n: Int) -> Int:
    return n + n

prompt solve(calculate: (Int) -> Int) -> String:
    "Double 21."

agent main() -> String:
    return solve(twice)
"#,
    )
    .unwrap();
    let jobs = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_corvid"));
        command
            .current_dir(dir.path())
            .arg("jobs")
            .args(args)
            .args(["--state", "queue.sqlite"])
            .env("CORVID_MODEL", &model)
            .env("CORVID_TEST_MOCK_LLM", "0")
            .env("OPENAI_COMPAT_API_KEY", "")
            .env("NO_PROXY", "127.0.0.1");
        command
    };
    for args in [
        vec!["enqueue", "--task", "main", "--payload", "[]"],
        vec![
            "loop",
            "limits",
            "--job",
            "job_1",
            "--max-steps",
            "2",
            "--max-tool-calls",
            "1",
        ],
    ] {
        let output = jobs(&args).output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    struct Worker(std::process::Child);
    impl Drop for Worker {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let worker = || {
        Worker(
            jobs(&[
                "run",
                "--source",
                "main.cor",
                "--workers",
                "1",
                "--lease-ttl-ms",
                "200",
                "--idle-poll-ms",
                "20",
            ])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
        )
    };
    let snapshot = || {
        let output = jobs(&["checkpoint", "resume", "--job", "job_1", "--json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let first = worker();
    tokio::time::timeout(Duration::from_secs(20), probe.waiting.notified())
        .await
        .expect("worker never sent its tool result");
    drop(first);
    let saved = snapshot();
    assert_eq!(saved["remaining_tool_calls"], 0);
    assert_eq!(saved["remaining_steps"], 1);
    assert_eq!(
        saved["checkpoints"][0]["payload"]["result"]["content"],
        "42"
    );
    let _second = worker();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let saved = snapshot();
        if saved["status"] == "succeeded" {
            assert_eq!(saved["remaining_steps"], 0);
            assert_eq!(saved["checkpoints"][1]["payload"]["response"], "forty-two");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{saved}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(probe.first_turns.load(Ordering::SeqCst), 1);
    assert_eq!(probe.followups.load(Ordering::SeqCst), 2);
    provider.abort();
}
