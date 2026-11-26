//! Typed failures in a streamed connector call (decision 7 of the HTTP
//! client proposal, stream failures S1). An operation that returns
//! `Stream<Result<T, E>>` and declares `on failure -> Variant` yields each
//! event as `Ok(T)`, and an outage as one final `Err(Variant(reason))`,
//! after which the stream ends: a failure to open, the per-event timeout,
//! an event that is not JSON or does not decode, or an open circuit
//! breaker. A `for` loop over it therefore never raises for an outage.
//! The final `Err` is recorded with the delivered events, so replay
//! yields it too.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_trace_schema::{read_events_from_path, TraceEvent};
use corvid_vm::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn source(connector_extra: &str) -> String {
    format!(
        r#"
effect model_read:
    cost: 1.0

type Delta:
    text: String

type ModelError:
    | Unreachable(reason: String)

connector provider:
    base_url: "http://model.example"
    timeout: 1s
    redirects: never
    published: no
{connector_extra}    modes: [mock, real, replay]
    operation complete(question: String) -> Stream<Result<Delta, ModelError>> uses model_read:
        POST "/v1/complete" body question
        streams: server_sent_events until "[DONE]"
        on failure -> Unreachable
        mock: [Ok(Delta("mocked")), Err(Unreachable("mock outage"))]

agent piece(item: Result<Delta, ModelError>) -> String:
    return match item:
        Ok(delta) -> delta.text
        Err(Unreachable(reason)) -> "[stopped: " + reason + "]"

agent answer(question: String) -> String uses model_read:
    text = ""
    for item in complete(question):
        text = text + piece(item)
    return text
"#
    )
}

fn compile(src: &str) -> (tempfile::TempDir, corvid_ir::IrFile) {
    let dir = tempfile::tempdir().unwrap();
    let path_on_disk = dir.path().join("main.cor");
    std::fs::write(&path_on_disk, src).unwrap();
    let ir = compile_to_ir_with_config_at_path(src, &path_on_disk, None).expect("compiles");
    (dir, ir)
}

fn real(ir: &corvid_ir::IrFile, addr: std::net::SocketAddr, trace: Option<&std::path::Path>) -> Runtime {
    let mut builder = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["model.example".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(reqwest::Client::builder().resolve("model.example", addr)))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(ir));
    if let Some(dir) = trace {
        builder = builder.trace_to(dir);
    }
    builder.build()
}

async fn answer(ir: &corvid_ir::IrFile, rt: &Runtime) -> String {
    let value = run_ir_with_runtime(ir, Some("answer"), vec![Value::String(std::sync::Arc::from("hi"))], rt)
        .await
        .expect("an outage is a value, so the loop never raises");
    let Value::String(text) = value else { panic!("expected a String, got {value:?}") };
    text.to_string()
}

async fn provider(body: &str, status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/v1/complete"))
        .respond_with(ResponseTemplate::new(status).insert_header("content-type", "text/event-stream").set_body_string(body))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_event_arrives_as_ok() {
    let (_dir, ir) = compile(&source(""));
    let server = provider("data: {\"text\":\"Hello\"}\n\ndata: {\"text\":\", world\"}\n\ndata: [DONE]\n\n", 200).await;
    assert_eq!(answer(&ir, &real(&ir, *server.address(), None)).await, "Hello, world");
}

/// An event that is not JSON, and one that does not decode as the event
/// type, each end the stream with the final `Err`, after the events
/// delivered before it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_event_ends_the_stream_with_a_final_err() {
    let (_dir, ir) = compile(&source(""));
    let server = provider("data: {\"text\":\"Hello\"}\n\ndata: not json\n\ndata: {\"text\":\"never\"}\n\n", 200).await;
    let text = answer(&ir, &real(&ir, *server.address(), None)).await;
    assert!(text.starts_with("Hello[stopped: an event that is not JSON"), "{text}");
    assert!(!text.contains("never"), "the stream ends at the failure: {text}");

    let server = provider("data: {\"text\":\"Hello\"}\n\ndata: {\"words\":3}\n\ndata: [DONE]\n\n", 200).await;
    let text = answer(&ir, &real(&ir, *server.address(), None)).await;
    assert!(text.starts_with("Hello[stopped: "), "{text}");
    assert!(text.contains("text"), "the reason names the field that did not decode: {text}");
}

/// A stream that fails to open is one `Err` event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_that_fails_to_open_is_one_err_event() {
    let (_dir, ir) = compile(&source(""));
    let server = provider("{\"error\":\"overloaded\"}", 503).await;
    assert_eq!(answer(&ir, &real(&ir, *server.address(), None)).await, "[stopped: provider returned HTTP 503]");
}

fn stalling_provider() -> (std::net::SocketAddr, std::sync::mpsc::Receiver<std::net::TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
            .unwrap();
        let event = b"data: {\"text\":\"first\"}\n\n";
        socket.write_all(format!("{:x}\r\n", event.len()).as_bytes()).unwrap();
        socket.write_all(event).unwrap();
        socket.write_all(b"\r\n").unwrap();
        socket.flush().unwrap();
        tx.send(socket).unwrap();
    });
    (addr, rx)
}

/// The per-event timeout ends the stream with the final `Err`, and the
/// recording holds it with the delivered event, ending `complete`, so
/// replay gives back the same text without the provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stall_ends_with_a_final_err_that_replays() {
    let (_dir, ir) = compile(&source(""));
    let (addr, connection) = stalling_provider();
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, addr, Some(trace_dir.path()));
    let text = answer(&ir, &rt).await;
    assert_eq!(text, "first[stopped: no event within its declared timeout of 1s]");
    drop(connection);

    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);
    let recorded = read_events_from_path(&trace_path)
        .unwrap()
        .into_iter()
        .find_map(|e| match e {
            TraceEvent::ToolResult { tool, result, .. } if tool == "complete" => Some(result),
            _ => None,
        })
        .expect("the stream is recorded");
    assert_eq!(recorded["end"], "complete");
    assert_eq!(recorded["events"].as_array().unwrap().len(), 2, "{recorded}");
    assert_eq!(recorded["events"][1]["tag"], "err");

    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    assert_eq!(answer(&ir, &replay).await, text);
}

/// A stream that ends with an `Err` event counts as one consecutive
/// failure; once the breaker opens, the next call is one `Err` event and
/// no request is sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_stream_opens_the_circuit_breaker() {
    let src = source("    circuit_breaker: 1\n").replace(
        "agent answer(question: String) -> String uses model_read:",
        "agent twice(question: String) -> String uses model_read:\n    return answer(question) + \" | \" + answer(question)\n\nagent answer(question: String) -> String uses model_read:",
    );
    let (_dir, ir) = compile(&src);
    let server = provider("{\"error\":\"down\"}", 503).await;
    let rt = real(&ir, *server.address(), None);
    let value = run_ir_with_runtime(&ir, Some("twice"), vec![Value::String(std::sync::Arc::from("hi"))], &rt)
        .await
        .expect("both calls are values");
    let Value::String(text) = value else { panic!("{value:?}") };
    assert!(text.starts_with("[stopped: provider returned HTTP 503] | [stopped: circuit breaker open"), "{text}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "the open breaker sends no request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_mode_yields_the_declared_events() {
    let (_dir, ir) = compile(&source(""));
    let rt = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Mock))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();
    assert_eq!(answer(&ir, &rt).await, "mocked[stopped: mock outage]");
}

/// Governance stays loud: a replay that diverges from its recording is
/// raised, never folded into an `Err` event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_diverging_replay_is_raised_not_an_err_event() {
    let (_dir, ir) = compile(&source(""));
    let server = provider("data: {\"text\":\"Hello\"}\n\ndata: [DONE]\n\n", 200).await;
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, *server.address(), Some(trace_dir.path()));
    answer(&ir, &rt).await;
    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);

    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    let error = run_ir_with_runtime(&ir, Some("answer"), vec![Value::String(std::sync::Arc::from("a different question"))], &replay)
        .await
        .expect_err("a diverging replay is raised");
    assert!(error.to_string().to_lowercase().contains("diverge"), "{error}");
}
