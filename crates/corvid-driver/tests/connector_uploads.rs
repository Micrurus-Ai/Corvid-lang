//! File bodies (B05 slice 6): an `Upload` or `Bytes` parameter sent as
//! the body itself, and `multipart` records with file parts. A file is
//! recorded by its name, type, size and SHA-256, never its content, and
//! replay matches it by that digest.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime, StructValue};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_trace_schema::{read_events_from_path, TraceEvent};
use corvid_vm::Value;
use wiremock::matchers::{body_bytes, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = r#"
effect files_write:
    cost: 1.0
    reversible: false

type Stored:
    id: String

type Receipt:
    title: String
    file: Upload<Pdf>
    tags: List<String>
    note: Option<String>

connector storage:
    base_url: "http://storage.example"
    timeout: 10s
    redirects: never
    published: no
    modes: [real, replay]
    operation attach(file: Upload<Pdf>) -> Stored uses files_write:
        POST "/files" body file
    operation put(data: Bytes) -> Stored uses files_write:
        PUT "/blobs" body data
    operation file_receipt(receipt: Receipt) -> Stored uses files_write:
        POST "/receipts" multipart receipt

agent send_file(file: Upload<Pdf>) -> String uses files_write:
    return attach(file).id

agent send_bytes(text: String) -> String uses files_write:
    return put(text.utf8()).id

agent send_receipt(file: Upload<Pdf>) -> String uses files_write:
    return file_receipt(Receipt("March", file, ["food", "travel"], None)).id
"#;

const PDF: &[u8] = b"%PDF-1.7 a person's private document";

fn upload() -> Value {
    Value::Struct(StructValue::new(
        corvid_resolve::DefId(0),
        "Upload",
        vec![
            ("filename".to_string(), Value::String("statement.pdf".into())),
            ("content_type".to_string(), Value::String("application/pdf".into())),
            ("size".to_string(), Value::Int(PDF.len() as i64)),
            ("bytes".to_string(), Value::Bytes(corvid_crypto::Bytes::new(PDF))),
        ],
    ))
}

fn compile() -> (tempfile::TempDir, corvid_ir::IrFile) {
    let dir = tempfile::tempdir().unwrap();
    let path_on_disk = dir.path().join("main.cor");
    std::fs::write(&path_on_disk, SOURCE).unwrap();
    let ir = compile_to_ir_with_config_at_path(SOURCE, &path_on_disk, None).expect("compiles");
    (dir, ir)
}

fn real(ir: &corvid_ir::IrFile, server: &MockServer, trace: Option<&std::path::Path>) -> Runtime {
    let mut builder = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["storage.example".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("storage.example", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(ir));
    if let Some(dir) = trace {
        builder = builder.trace_to(dir);
    }
    builder.build()
}

fn stored(id: &str) -> ResponseTemplate {
    ResponseTemplate::new(201).set_body_string(format!(r#"{{"id":"{id}"}}"#))
}

/// The upload's bytes are the body, with its own content type; the trace
/// holds its name, type, size and digest but not one byte of content, and
/// replay matches the same file by digest with no request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_is_sent_as_its_bytes_and_recorded_by_digest() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/files"))
        .and(header("content-type", "application/pdf"))
        .and(body_bytes(PDF))
        .respond_with(stored("f_1"))
        .mount(&server)
        .await;
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, &server, Some(trace_dir.path()));

    let result = run_ir_with_runtime(&ir, Some("send_file"), vec![upload()], &rt).await.expect("sent");
    assert_eq!(result, Value::String("f_1".into()));

    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);
    let raw = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!raw.contains("private document"), "the content must not reach the trace");
    let encoded = corvid_crypto::Bytes::new(PDF).to_base64_url();
    assert!(
        !raw.contains(&encoded),
        "not even encoded: {}",
        raw.lines().filter(|line| line.contains(&encoded)).map(|line| &line[..line.len().min(160)]).collect::<Vec<_>>().join("\n")
    );
    let args = read_events_from_path(&trace_path).unwrap().into_iter().find_map(|e| match e {
        TraceEvent::ToolCall { tool, args, .. } if tool == "attach" => Some(args),
        _ => None,
    });
    let recorded = &args.expect("the call is recorded")[0];
    assert_eq!(recorded["filename"], "statement.pdf");
    assert_eq!(recorded["size"], PDF.len());
    assert_eq!(recorded["sha256"].as_str().unwrap().len(), 64);

    let before = server.received_requests().await.unwrap().len();
    let replay = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();
    let replayed = run_ir_with_runtime(&ir, Some("send_file"), vec![upload()], &replay).await.expect("replays");
    assert_eq!(replayed, Value::String("f_1".into()));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bytes_are_sent_as_octet_stream() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/blobs"))
        .and(header("content-type", "application/octet-stream"))
        .and(body_bytes(b"hello".to_vec()))
        .respond_with(stored("b_1"))
        .mount(&server)
        .await;
    let rt = real(&ir, &server, None);

    let result = run_ir_with_runtime(&ir, Some("send_bytes"), vec![Value::String("hello".into())], &rt)
        .await
        .expect("sent");

    assert_eq!(result, Value::String("b_1".into()));
}

/// A multipart record sends a text part per scalar, a part per list item,
/// nothing for an absent `Option`, and a file part with the upload's name
/// and type; the same request has the same bytes, so the same digest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_multipart_record_sends_text_and_file_parts() {
    let (_dir, ir) = compile();
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/receipts")).respond_with(stored("r_1")).mount(&server).await;
    let trace_dir = tempfile::tempdir().unwrap();
    let rt = real(&ir, &server, Some(trace_dir.path()));

    for _ in 0..2 {
        let result = run_ir_with_runtime(&ir, Some("send_receipt"), vec![upload()], &rt).await.expect("sent");
        assert_eq!(result, Value::String("r_1".into()));
    }

    let requests = server.received_requests().await.unwrap();
    let sent = &requests[0];
    let content_type = sent.headers.get("content-type").unwrap().to_str().unwrap().to_string();
    assert!(content_type.starts_with("multipart/form-data; boundary=corvid-"), "{content_type}");
    let body = String::from_utf8_lossy(&sent.body);
    assert!(body.contains("name=\"title\"\r\n\r\nMarch\r\n"), "{body}");
    assert_eq!(body.matches("name=\"tags\"").count(), 2, "{body}");
    assert!(!body.contains("name=\"note\""), "an absent Option is left out: {body}");
    assert!(
        body.contains("name=\"file\"; filename=\"statement.pdf\"\r\nContent-Type: application/pdf\r\n\r\n%PDF-1.7"),
        "{body}"
    );
    assert_eq!(requests[0].body, requests[1].body, "the same request has the same bytes");

    let trace_path = rt.tracer().path().to_path_buf();
    drop(rt);
    let digests: Vec<String> = read_events_from_path(&trace_path)
        .unwrap()
        .into_iter()
        .filter_map(|e| match e {
            TraceEvent::HttpExchange { operation, request_body_sha256, .. } if operation == "file_receipt" => {
                request_body_sha256
            }
            _ => None,
        })
        .collect();
    assert_eq!(digests.len(), 2);
    assert_eq!(digests[0], digests[1], "the recorded digest is reproducible");
    assert!(!std::fs::read_to_string(&trace_path).unwrap().contains("private document"));
}
