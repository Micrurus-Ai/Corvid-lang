//! `Response<Body>` (B05 slice 3c): a connector operation that declares
//! it gets the status and the response headers next to the decoded body.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_vm::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn program(main: &str) -> String {
    format!(
        r#"
effect http_read:
    cost: 1.0

type Charge:
    id: String

type ChargeError:
    | Missing

connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [mock, real, replay]
    operation charge(id: String) -> Response<Charge> uses http_read:
        GET "/v1/charges/{{id}}"
        mock: Charge(id)
    operation maybe(id: String) -> Result<Response<Charge>, ChargeError> uses http_read:
        GET "/v1/charges/{{id}}"
        on status 404 -> Missing
        mock: Ok(Charge(id))
    operation exists(id: String) -> Response<Nothing> uses http_read:
        HEAD "/v1/charges/{{id}}"
        mock: nothing

agent text_or(value: Option<String>, fallback: String) -> String:
    return match value:
        Some(v) -> v
        None -> fallback
{main}"#
    )
}

fn compile(src: &str) -> (tempfile::TempDir, corvid_ir::IrFile) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.cor");
    std::fs::write(&path, src).unwrap();
    let ir = compile_to_ir_with_config_at_path(src, &path, None).expect("program compiles");
    (dir, ir)
}

fn real_runtime(ir: &corvid_ir::IrFile, server: &MockServer, trace_dir: Option<&std::path::Path>) -> Runtime {
    let mut builder = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.payments.example".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("api.payments.example", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(ir));
    if let Some(dir) = trace_dir {
        builder = builder.trace_to(dir);
    }
    builder.build()
}

fn text(value: &str) -> Value {
    Value::String(std::sync::Arc::from(value))
}

/// The status, a header read without regard to case, and the typed body;
/// `Set-Cookie` is never passed on, so neither the program nor the trace
/// holds the cookie, and replay answers the header from the recording.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_response_carries_status_headers_and_body_and_replays() {
    let src = program(
        r#"
agent main() -> String uses http_read:
    r = charge("ch_1")
    cookie = text_or(r.header("set-cookie"), "no cookie")
    return r.status().to_string() + " " + text_or(r.header("ETag"), "none") + " " + r.body().id + " " + cookie
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/charges/ch_1"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("etag", "\"v7\"")
                .insert_header("set-cookie", "session=sess-secret-123; HttpOnly")
                .set_body_string(r#"{"id":"ch_1"}"#),
        )
        .mount(&server)
        .await;
    let trace_dir = tempfile::tempdir().unwrap();
    let runtime = real_runtime(&ir, &server, Some(trace_dir.path()));

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("runs");
    assert_eq!(result, text("201 \"v7\" ch_1 no cookie"));

    let trace_path = runtime.tracer().path().to_path_buf();
    drop(runtime);
    let raw = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!raw.contains("sess-secret-123"), "the cookie must never reach the trace");

    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    let replayed = run_ir_with_runtime(&ir, None, vec![], &replay).await.expect("replays");
    assert_eq!(replayed, text("201 \"v7\" ch_1 no cookie"));
}

/// A `HEAD` request answers with headers and no body.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_head_request_reads_headers_without_a_body() {
    let src = program(
        r#"
agent main() -> String uses http_read:
    r = exists("ch_1")
    return r.status().to_string() + " " + text_or(r.header("x-total"), "none")
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/v1/charges/ch_1"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-total", "42"))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &real_runtime(&ir, &server, None)).await.expect("runs");

    assert_eq!(result, text("200 42"));
}

/// Inside a `Result`, a 2xx is `Ok(Response)` and a mapped status is the
/// typed error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_result_of_a_response_maps_its_errors() {
    let src = program(
        r#"
agent describe(r: Result<Response<Charge>, ChargeError>) -> String:
    return match r:
        Ok(found) -> found.status().to_string() + " " + found.body().id
        Err(e) -> "missing"

agent main() -> String uses http_read:
    return describe(maybe("ch_1")) + ", " + describe(maybe("ch_2"))
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges/ch_1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"ch_1"}"#))
        .mount(&server)
        .await;
    Mock::given(path("/v1/charges/ch_2"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &real_runtime(&ir, &server, None)).await.expect("runs");

    assert_eq!(result, text("200 ch_1, missing"));
}

/// In mock mode the declared body answers with status 200 and no headers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_mode_answers_a_response_from_its_body() {
    let src = program(
        r#"
agent main() -> String uses http_read:
    r = charge("ch_mock")
    return r.status().to_string() + " " + r.body().id + " " + text_or(r.header("etag"), "no etag")
"#,
    );
    let (_dir, ir) = compile(&src);
    let runtime = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Mock))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("runs");

    assert_eq!(result, text("200 ch_mock no etag"));
}
