//! A `hosts: allowed_hosts` connector (B05 slice 4): its operations take
//! the whole URL as a `String` parameter and may call any host the
//! project's `[http] allow` permits, under the same SSRF block,
//! allowlist, declared timeout and redirect policy, recording and replay
//! as every connector. It replaces `http_get` and `http_post_json`.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_vm::Value;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = r#"
effect http_read:
    cost: 1.0

effect http_write:
    cost: 1.0
    reversible: false

type Message:
    text: String

type Receipt:
    id: Int

connector web:
    hosts: allowed_hosts
    timeout: 10s
    redirects: allowed_hosts
    modes: [real, replay]
    operation fetch(url: String) -> Response<String> uses http_read:
        GET url
        response: text
    operation send(url: String, message: Message) -> Receipt dangerous uses http_write:
        POST url body message

agent main(url: String) -> String uses http_read:
    page = fetch(url)
    return page.status().to_string() + " " + page.body()
"#;

fn compile() -> (tempfile::TempDir, corvid_ir::IrFile) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.cor");
    std::fs::write(&path, SOURCE).unwrap();
    let ir = compile_to_ir_with_config_at_path(SOURCE, &path, None).expect("compiles");
    (dir, ir)
}

fn runtime(ir: &corvid_ir::IrFile, allow: &[&str], hosts: &[(&str, &MockServer)]) -> Runtime {
    let mut builder = reqwest::Client::builder();
    for (host, server) in hosts {
        builder = builder.resolve(host, *server.address());
    }
    let allow: Vec<String> = allow.iter().map(|h| h.to_string()).collect();
    let policy = if allow.is_empty() { HttpEgressPolicy::unset() } else { HttpEgressPolicy::new(Some(&allow)) };
    Runtime::builder()
        .http_policy(policy)
        .http_client(HttpClient::with_reqwest_builder(builder))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(ir))
        .build()
}

fn text(value: &str) -> Value {
    Value::String(std::sync::Arc::from(value))
}

async fn page_server(body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/status"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;
    server
}

/// The URL is the argument, the host is allowed, and `response: text`
/// reads a page that is not JSON.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connector_url_reaches_an_allowed_host() {
    let (_dir, ir) = compile();
    let server = page_server("<p>ok</p>").await;
    let rt = runtime(&ir, &["api.example.com"], &[("api.example.com", &server)]);

    let result = run_ir_with_runtime(&ir, None, vec![text("http://api.example.com/status")], &rt)
        .await
        .expect("an allowed host is reached");

    assert_eq!(result, text("200 <p>ok</p>"));
}

/// The SSRF block runs under every allowlist: a loopback, private or
/// link-local address is refused even when the allowlist names it, and
/// nothing is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_on_a_private_address_is_refused_even_when_allowed() {
    let (_dir, ir) = compile();
    let server = page_server("secret").await;
    let rt = runtime(&ir, &["127.0.0.1", "169.254.169.254", "localhost"], &[]);

    for url in [
        format!("http://127.0.0.1:{}/status", server.address().port()),
        "http://169.254.169.254/latest/meta-data".to_string(),
        "http://localhost/status".to_string(),
    ] {
        let err = run_ir_with_runtime(&ir, None, vec![text(&url)], &rt)
            .await
            .expect_err("a private address must be refused");
        assert!(err.to_string().contains("structural SSRF block"), "{url}: {err}");
    }
    assert!(server.received_requests().await.unwrap().is_empty(), "nothing may be sent");
}

/// A host the allowlist does not name is refused, naming the host.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unlisted_host_is_refused() {
    let (_dir, ir) = compile();
    let server = page_server("hidden").await;
    let rt = runtime(&ir, &["api.example.com"], &[("other.example.org", &server)]);

    let err = run_ir_with_runtime(&ir, None, vec![text("http://other.example.org/status")], &rt)
        .await
        .expect_err("an unlisted host must be refused");

    assert!(err.to_string().contains("other.example.org"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// With no `[http] allow` configured, every call fails closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connector_url_fails_closed_without_an_allow_list() {
    let (_dir, ir) = compile();
    let server = page_server("hidden").await;
    let rt = runtime(&ir, &[], &[("api.example.com", &server)]);

    let err = run_ir_with_runtime(&ir, None, vec![text("http://api.example.com/status")], &rt)
        .await
        .expect_err("no allowlist must fail closed");

    assert!(err.to_string().contains("[http] allow"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// What `http_post_json` did by hand: a typed record sent as JSON to a
/// URL the program holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_typed_body_is_posted_to_a_url() {
    let src = SOURCE.replace(
        "agent main(url: String) -> String uses http_read:\n    page = fetch(url)\n    return page.status().to_string() + \" \" + page.body()\n",
        "agent main(url: String) -> Int uses http_write:\n    message = Message(\"hello\")\n    approve Send(url, message)\n    return send(url, message).id\n",
    );
    let dir = tempfile::tempdir().unwrap();
    let path_on_disk = dir.path().join("main.cor");
    std::fs::write(&path_on_disk, &src).unwrap();
    let ir = compile_to_ir_with_config_at_path(&src, &path_on_disk, None).expect("compiles");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_json(serde_json::json!({ "text": "hello" })))
        .respond_with(ResponseTemplate::new(201).set_body_string(r#"{"id":7}"#))
        .mount(&server)
        .await;
    let rt = Runtime::builder()
        .approver(std::sync::Arc::new(corvid_runtime::ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["upstream.test".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("upstream.test", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![text("http://upstream.test/messages")], &rt)
        .await
        .expect("the typed body is posted");

    assert_eq!(result, Value::Int(7));
}

const SAFE_SOURCE: &str = r#"
effect http_read:
    cost: 1.0

type Item:
    id: Int

type FetchError:
    | Unreachable(reason: String)

connector web:
    hosts: allowed_hosts
    timeout: 1s
    redirects: never
    modes: [real, replay]
    operation page(url: String) -> Result<Response<String>, FetchError> uses http_read:
        GET url
        response: text
        on failure -> Unreachable
    operation item(url: String) -> Result<Item, FetchError> uses http_read:
        GET url
        on failure -> Unreachable

agent reason_of(e: FetchError) -> String:
    return match e:
        Unreachable(reason) -> "unreachable: " + reason

agent describe(r: Result<Response<String>, FetchError>) -> String:
    return match r:
        Ok(answer) -> "ok " + answer.body()
        Err(e) -> reason_of(e)

agent describe_item(r: Result<Item, FetchError>) -> String:
    return match r:
        Ok(found) -> "item " + found.id.to_string()
        Err(e) -> reason_of(e)

agent main(url: String, item_url: String) -> String uses http_read:
    return describe(page(url)) + " | " + describe_item(item(item_url))
"#;

/// `on failure -> Variant` turns every failure into the typed error the
/// program handles: a refused private address, an unmapped 500, a
/// timeout and a JSON answer of the wrong shape. A recorded failure
/// replays as the same value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_failure_turns_every_failure_into_the_declared_variant() {
    let dir = tempfile::tempdir().unwrap();
    let path_on_disk = dir.path().join("main.cor");
    std::fs::write(&path_on_disk, SAFE_SOURCE).unwrap();
    let ir = compile_to_ir_with_config_at_path(SAFE_SOURCE, &path_on_disk, None).expect("compiles");
    let server = MockServer::start().await;
    Mock::given(path("/broken")).respond_with(ResponseTemplate::new(500)).mount(&server).await;
    Mock::given(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(4)))
        .mount(&server)
        .await;
    Mock::given(path("/wrong")).respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"seven"}"#)).mount(&server).await;
    Mock::given(path("/fine")).respond_with(ResponseTemplate::new(200).set_body_string("hello")).mount(&server).await;

    let trace_dir = tempfile::tempdir().unwrap();
    let real = |trace: Option<&std::path::Path>| {
        let mut builder = Runtime::builder()
            .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string(), "127.0.0.1".to_string()])))
            .http_client(HttpClient::with_reqwest_builder(
                reqwest::Client::builder().resolve("api.example.com", *server.address()),
            ))
            .connector_mode(Some(corvid_ast::ConnectorMode::Real))
            .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir));
        if let Some(dir) = trace {
            builder = builder.trace_to(dir);
        }
        builder.build()
    };
    let recorded_rt = real(Some(trace_dir.path()));
    let rt = real(None);
    let run = |page: &str, item: &str| {
        run_ir_with_runtime(&ir, None, vec![text(page), text(item)], &rt)
    };

    let refused = run_ir_with_runtime(
        &ir,
        None,
        vec![text("http://127.0.0.1/secret"), text("http://api.example.com/broken")],
        &recorded_rt,
    )
    .await
    .expect("handled");
    let Value::String(refused) = refused else { panic!("expected text, got {refused:?}") };
    assert!(refused.contains("unreachable: ") && refused.contains("structural SSRF block"), "{refused}");
    assert!(refused.contains("| unreachable: provider returned HTTP 500"), "{refused}");

    let slow = run("http://api.example.com/slow", "http://api.example.com/wrong").await.expect("handled");
    let Value::String(slow) = slow else { panic!("expected text, got {slow:?}") };
    assert!(slow.contains("declared timeout of 1s"), "{slow}");
    assert!(slow.contains("does not match `Item` at `id`"), "{slow}");

    let fine = run("http://api.example.com/fine", "http://api.example.com/broken").await.expect("handled");
    assert_eq!(fine, text("ok hello | unreachable: provider returned HTTP 500"));

    // The recorded failures replay as the same values, with no request.
    let trace_path = recorded_rt.tracer().path().to_path_buf();
    drop(recorded_rt);
    let before = server.received_requests().await.unwrap().len();
    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    let replayed = run_ir_with_runtime(&ir, None, vec![text("http://127.0.0.1/secret"), text("http://api.example.com/broken")], &replay)
        .await
        .expect("replays");
    assert_eq!(replayed, Value::String(std::sync::Arc::from(refused.as_ref())));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}
