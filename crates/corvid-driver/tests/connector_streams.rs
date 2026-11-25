//! Streamed connector responses (B05 slice 5): `Stream<T>` operations
//! declare `streams: server_sent_events` or `json_lines`, yield one
//! decoded event at a time, bound each wait by the declared timeout,
//! record the delivered events for replay, and close the upstream
//! connection when the program stops reading.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_trace_schema::{read_events_from_path, TraceEvent};
use corvid_vm::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = r#"
effect model_read:
    cost: 1.0

type Delta:
    text: String

connector provider:
    base_url: "http://model.example"
    timeout: 1s
    redirects: never
    modes: [mock, real, replay]
    operation complete(question: String) -> Stream<Delta> uses model_read:
        POST "/v1/complete" body question
        streams: server_sent_events until "[DONE]"
        mock: [Delta("mocked "), Delta("reply")]
    operation lines(question: String) -> Stream<Delta> uses model_read:
        POST "/v1/lines" body question
        streams: json_lines
        mock: [Delta("line")]

agent all_of(question: String) -> String uses model_read:
    text = ""
    for delta in complete(question):
        text = text + delta.text
    return text

agent first_of(question: String) -> String uses model_read:
    for delta in complete(question):
        return delta.text
    return "none"

agent lines_of(question: String) -> String uses model_read:
    text = ""
    for delta in lines(question):
        text = text + delta.text + ";"
    return text
"#;

fn compile() -> (tempfile::TempDir, corvid_ir::IrFile) {
    let dir = tempfile::tempdir().unwrap();
    let path_on_disk = dir.path().join("main.cor");
    std::fs::write(&path_on_disk, SOURCE).unwrap();
    let ir = compile_to_ir_with_config_at_path(SOURCE, &path_on_disk, None).expect("compiles");
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

async fn run(ir: &corvid_ir::IrFile, rt: &Runtime, agent: &str) -> Result<Value, corvid_driver::RunError> {
    run_ir_with_runtime(ir, Some(agent), vec![Value::String(std::sync::Arc::from("hi"))], rt).await
}

fn text(value: &str) -> Value {
    Value::String(std::sync::Arc::from(value))
}

/// Server-sent events decode one `Delta` at a time and end at the
/// declared marker; the recording replays the same text with no request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_sent_events_stream_and_replay() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(path("/v1/complete"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(
            ": keep-alive\n\ndata: {\"text\":\"Hello\"}\n\nevent: delta\ndata: {\"text\":\", world\"}\n\ndata: [DONE]\n\n",
        ))
        .mount(&server)
        .await;
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, *server.address(), Some(trace_dir.path()));

    assert_eq!(run(&ir, &rt, "all_of").await.expect("streams"), text("Hello, world"));

    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);
    let recorded = read_events_from_path(&trace_path).unwrap().into_iter().find_map(|e| match e {
        TraceEvent::ToolResult { tool, result, .. } if tool == "complete" => Some(result),
        _ => None,
    });
    let recorded = recorded.expect("the stream is recorded");
    assert_eq!(recorded["end"], "complete");
    assert_eq!(recorded["events"].as_array().unwrap().len(), 2);

    let before = server.received_requests().await.unwrap().len();
    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    assert_eq!(run(&ir, &replay, "all_of").await.expect("replays"), text("Hello, world"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_lines_stream_one_value_per_line() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(path("/v1/lines"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"text\":\"a\"}\n\n{\"text\":\"b\"}\n{\"text\":\"c\"}"))
        .mount(&server)
        .await;
    let rt = real(&ir, *server.address(), None);

    assert_eq!(run(&ir, &rt, "lines_of").await.expect("streams"), text("a;b;c;"));
}

/// An event that is not JSON ends the stream with an error naming it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_that_is_not_json_ends_the_stream_with_an_error() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(path("/v1/complete"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: {\"text\":\"ok\"}\n\ndata: oops\n\n"))
        .mount(&server)
        .await;
    let rt = real(&ir, *server.address(), None);

    let err = run(&ir, &rt, "all_of").await.expect_err("a bad event fails the stream");

    assert!(err.to_string().contains("an event that is not JSON"), "{err}");
}

/// A provider that sends a status line, headers and one event, then holds
/// the connection: `(accepted, connection)` once a request arrives.
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

/// The declared timeout bounds the wait for each event: a provider that
/// stalls after one event fails the stream after the timeout, not never.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_stream_fails_after_the_declared_timeout() {
    let (_dir, ir) = compile();
    let (addr, connection) = stalling_provider();
    let rt = real(&ir, addr, None);

    let started = std::time::Instant::now();
    let err = run(&ir, &rt, "all_of").await.expect_err("the stall must fail");

    assert!(err.to_string().contains("no event within its declared timeout of 1s"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    drop(connection);
}

/// A program that stops reading drops the stream, which closes the
/// upstream connection, and the recording says the stream was cancelled
/// with the events delivered so far.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_stream_closes_the_upstream_connection() {
    let (_dir, ir) = compile();
    let (addr, connection) = stalling_provider();
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, addr, Some(trace_dir.path()));

    assert_eq!(run(&ir, &rt, "first_of").await.expect("returns the first event"), text("first"));

    let mut upstream = connection.recv_timeout(Duration::from_secs(5)).expect("the provider was called");
    upstream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut buf = [0u8; 1];
    match upstream.read(&mut buf) {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("the upstream connection stayed open: {other:?}"),
    }
    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);
    let recorded = read_events_from_path(&trace_path).unwrap().into_iter().find_map(|e| match e {
        TraceEvent::ToolResult { tool, result, .. } if tool == "complete" => Some(result),
        _ => None,
    });
    let recorded = recorded.expect("the cancelled stream is recorded");
    assert_eq!(recorded["end"], "cancelled");
    assert_eq!(recorded["events"], serde_json::json!([{ "text": "first" }]));
}

/// In mock mode the declared events stream in order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_mode_streams_the_declared_events() {
    let (_dir, ir) = compile();
    let rt = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Mock))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    assert_eq!(run(&ir, &rt, "all_of").await.expect("mocks"), text("mocked reply"));
}
