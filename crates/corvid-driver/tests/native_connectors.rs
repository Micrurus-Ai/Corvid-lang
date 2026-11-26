//! Connector calls in native code (native dispatch N1). The same program
//! runs as a native binary and on the interpreter against the same
//! provider. Both tiers return the same value and record the same run,
//! event for event: `ToolCall`, `HttpExchange`, `ToolResult` and the
//! `connector.invocation` record, because both run `Runtime::call_tool`.
//! Each tier replays its own recording without reaching the provider;
//! replaying on the other tier is slice N5.
//!
//! The interpreter reaches the wiremock provider through a DNS override,
//! the native binary through `HTTP_PROXY`; both send the request to the
//! same public name, so the egress policy sees the same URL.

use std::path::{Path, PathBuf};
use std::process::Command;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, native_ability, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_trace_schema::read_events_from_path;
use corvid_vm::Value;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HOST: &str = "api.payments.example";

const PROGRAM: &str = r#"
effect http_write:
    cost: 1.0

type NewCharge:
    amount: Int
    note: Option<String>

type Charge:
    id: String
    amount: Int

type ChargeError:
    | Declined

connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [real, replay]
    operation charge(new: NewCharge) -> Result<Charge, ChargeError> uses http_write:
        POST "/v1/charges" body new
        on status 402 -> Declined

agent outcome(result: Result<Charge, ChargeError>) -> String:
    return match result:
        Ok(c) -> c.id
        Err(Declined) -> "declined"

agent main() -> String:
    first = outcome(charge(NewCharge(500, None)))
    second = outcome(charge(NewCharge(900, Some("gift"))))
    return first + " " + second
"#;

struct Project {
    _dir: tempfile::TempDir,
    source: PathBuf,
    program: String,
    ir: corvid_ir::IrFile,
}

fn project() -> Project {
    project_of(PROGRAM)
}

fn project_of(program: &str) -> Project {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let source = src_dir.join("main.cor");
    std::fs::write(&source, program).unwrap();
    let ir = compile_to_ir_with_config_at_path(program, &source, None).expect("program compiles");
    Project { _dir: dir, source, program: program.to_string(), ir }
}

async fn provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 500, "note": null })))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"ch_1","amount":500,"livemode":false}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 900, "note": "gift" })))
        .respond_with(ResponseTemplate::new(402).set_body_string(r#"{"error":"card_declined"}"#))
        .mount(&server)
        .await;
    server
}

fn connector_calls(ir: &corvid_ir::IrFile) -> std::collections::HashMap<String, corvid_runtime::connectors::ConnectorHttpSpec> {
    corvid_runtime::connectors::connector_calls_from_ir(ir)
}

async fn interpreter_real(project: &Project, server: &MockServer, trace_dir: &Path) -> (Value, PathBuf) {
    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&[HOST.to_string()])))
        .http_client(HttpClient::with_reqwest_builder(reqwest::Client::builder().resolve(HOST, *server.address())))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(connector_calls(&project.ir))
        .trace_to(trace_dir)
        .build();
    let value = run_ir_with_runtime(&project.ir, None, vec![], &runtime).await.expect("interpreter run");
    let trace = runtime.tracer().path().to_path_buf();
    (value, trace)
}

async fn interpreter_replay(project: &Project, trace: &Path) -> Value {
    let runtime = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(connector_calls(&project.ir))
        .replay_from(trace)
        .build();
    run_ir_with_runtime(&project.ir, None, vec![], &runtime).await.expect("interpreter replay")
}

/// Links against the release `corvid-test-tools` staticlib, which bundles
/// the current `corvid-runtime`, as the codegen native tests do, so a
/// stale debug runtime library is never picked up.
fn link_current_runtime() {
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "corvid-test-tools", "--release"])
            .current_dir(corvid_test_paths::workspace_root())
            .status()
            .expect("build corvid-test-tools");
        assert!(status.success(), "building corvid-test-tools failed");
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    });
}

fn native_binary(project: &Project) -> PathBuf {
    native_ability(&project.ir).expect("the program compiles to native code");
    link_current_runtime();
    build_or_get_cached_native(&project.source, &project.program, &project.ir, None).expect("native build").path
}

/// Runs the binary with the environment `corvid run --target native`
/// gives it. `HTTP_PROXY` stands in for the network: the binary sends
/// the request for `api.payments.example` to the wiremock server.
fn run_native(binary: &Path, mode: &str, server: Option<&MockServer>, env: &[(&str, &Path)]) -> String {
    let mut command = Command::new(binary);
    command
        .env("CORVID_CONNECTOR_MODE", mode)
        .env("CORVID_HTTP_ALLOW", HOST)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .env_remove("CORVID_TRACE_PATH")
        .env_remove("CORVID_REPLAY_TRACE_PATH");
    match server {
        Some(server) => {
            command.env("HTTP_PROXY", server.uri()).env("http_proxy", server.uri());
        }
        None => {
            // Replay must not reach any provider: point the proxy at a
            // closed port so a real request would fail loudly.
            command.env("HTTP_PROXY", "http://127.0.0.1:9").env("http_proxy", "http://127.0.0.1:9");
        }
    }
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run native binary");
    assert!(
        output.status.success(),
        "native run failed: status {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn drop_date_headers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(serde_json::Value::Array(headers)) = fields.get_mut("headers") {
                headers.retain(|pair| pair.get(0).and_then(|name| name.as_str()) != Some("date"));
            }
            fields.values_mut().for_each(drop_date_headers);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(drop_date_headers),
        _ => {}
    }
}

/// Every event of a trace after its header and seed, without the fields
/// that differ per run: times, the run id, and the response headers
/// wiremock dates.
fn run_events(trace: &Path) -> Vec<serde_json::Value> {
    read_events_from_path(trace)
        .expect("trace reads")
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .filter(|event| !matches!(event["kind"].as_str(), Some("schema_header" | "seed_read")))
        .map(|mut event| {
            let fields = event.as_object_mut().unwrap();
            for key in ["ts_ms", "run_id", "elapsed_ms", "response_headers"] {
                fields.remove(key);
            }
            // A `Response` result carries the provider's headers, and
            // wiremock dates each reply.
            drop_date_headers(&mut event);
            event
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_native_connector_call_matches_the_interpreter_in_value_and_record() {
    let project = project();
    let server = provider().await;
    let traces = tempfile::tempdir().unwrap();

    let (interpreted, interpreter_trace) = interpreter_real(&project, &server, traces.path()).await;
    assert_eq!(interpreted, Value::String("ch_1 declined".into()));

    let binary = native_binary(&project);
    let native_trace = traces.path().join("native.jsonl");
    let native = run_native(&binary, "real", Some(&server), &[("CORVID_TRACE_PATH", &native_trace)]);
    assert_eq!(native, "ch_1 declined");

    let interpreter_events = run_events(&interpreter_trace);
    let native_events = run_events(&native_trace);
    let kinds: Vec<&str> = interpreter_events.iter().filter_map(|e| e["kind"].as_str()).collect();
    assert_eq!(
        kinds,
        [
            "run_started",
            "tool_call", "http_exchange", "tool_result", "host_event",
            "tool_call", "http_exchange", "tool_result", "host_event",
            "run_completed",
        ],
        "each call records its request, exchange, result and invocation: {interpreter_events:#?}"
    );
    assert_eq!(native_events, interpreter_events, "both tiers record the same run, call for call");
}

/// A native recording replays in native code without reaching the
/// provider, as an interpreter recording replays on the interpreter.
/// Replaying a recording on the other tier is refused today by the
/// trace-writer check, for every kind of event; that is slice N5.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_tier_replays_its_recording_without_the_provider() {
    let project = project();
    let server = provider().await;
    let traces = tempfile::tempdir().unwrap();
    let binary = native_binary(&project);

    let native_trace = traces.path().join("native.jsonl");
    run_native(&binary, "real", Some(&server), &[("CORVID_TRACE_PATH", &native_trace)]);
    let (_, interpreter_trace) = interpreter_real(&project, &server, traces.path()).await;
    let sent = server.received_requests().await.unwrap().len();
    drop(server);

    let replayed = run_native(&binary, "replay", None, &[("CORVID_REPLAY_TRACE_PATH", &native_trace)]);
    assert_eq!(replayed, "ch_1 declined", "native code replays its recording");
    assert_eq!(
        interpreter_replay(&project, &interpreter_trace).await,
        Value::String("ch_1 declined".into()),
        "the interpreter replays its recording"
    );
    assert_eq!(sent, 4, "only the two recording runs reached the provider");
}


fn native_refusal(src: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("main.cor");
    std::fs::write(&source, src).unwrap();
    let ir = compile_to_ir_with_config_at_path(src, &source, None).expect("program compiles");
    native_ability(&ir).expect_err("the program stays on the interpreter").to_string()
}

/// A connector whose `circuit_breaker` counts failures per interpreter
/// run stays there until a server-wide breaker has a declared reset
/// policy, and `corvid run` says why.
#[test]
fn a_circuit_breaker_connector_stays_on_the_interpreter() {
    let src = PROGRAM.replace("    redirects: never\n", "    redirects: never\n    circuit_breaker: 3\n");
    let reason = native_refusal(&src);
    assert!(reason.contains("`charge`") && reason.contains("circuit_breaker"), "{reason}");
}

/// Native code has no streams yet, for any source.
#[test]
fn a_streamed_operation_stays_on_the_interpreter() {
    let src = r#"
effect model_read:
    cost: 1.0

type Delta:
    text: String

connector provider:
    base_url: "http://model.example"
    timeout: 1s
    redirects: never
    modes: [real]
    operation complete(question: String) -> Stream<Delta> uses model_read:
        POST "/v1/complete" body question
        streams: json_lines

agent main() -> String uses model_read:
    text = ""
    for delta in complete("hi"):
        text = text + delta.text
    return text
"#;
    let reason = native_refusal(src);
    assert!(reason.contains("Stream"), "{reason}");
}

/// `--mode mock` evaluates the connector's `mock:` expressions, which
/// only the interpreter can; asking for native code then is refused
/// rather than silently run another way.
#[test]
fn mock_mode_refuses_the_native_target() {
    let project = project();
    let code = corvid_driver::run_with_target(
        &project.source,
        corvid_driver::RunTarget::Native,
        None,
        &[],
        Some(corvid_ast::ConnectorMode::Mock),
    )
    .expect("runs");
    assert_eq!(code, 1);
}


/// `Response<Body>` in native code (native dispatch N4): the status, a
/// header read without regard to case, the typed body, a mapped status
/// through `Result<Response<Body>, E>`, and a `HEAD` request's
/// `Response<Nothing>`, the same on both tiers, with `Set-Cookie` never
/// passed on.
const RESPONSE_PROGRAM: &str = r#"
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
    modes: [real, replay]
    operation charge(id: String) -> Response<Charge> uses http_read:
        GET "/v1/charges/{id}"
    operation maybe(id: String) -> Result<Response<Charge>, ChargeError> uses http_read:
        GET "/v1/charges/{id}"
        on status 404 -> Missing
    operation exists(id: String) -> Response<Nothing> uses http_read:
        HEAD "/v1/charges/{id}"

agent text_or(value: Option<String>, fallback: String) -> String:
    return match value:
        Some(v) -> v
        None -> fallback

agent status_of(found: Result<Response<Charge>, ChargeError>) -> String:
    return match found:
        Ok(r) -> r.body().id
        Err(Missing) -> "missing"

agent main() -> String uses http_read:
    r = charge("ch_1")
    first = r.status().to_string() + " " + text_or(r.header("ETag"), "none") + " " + r.body().id + " " + text_or(r.header("set-cookie"), "no cookie") + " " + text_or(r.header("X-Absent"), "absent")
    return first + " " + status_of(maybe("ch_1")) + " " + status_of(maybe("ch_9")) + " " + exists("ch_1").status().to_string()
"#;

async fn response_provider() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges/ch_1"))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("etag", "\"v7\"")
                .insert_header("set-cookie", "session=sess-secret-123; HttpOnly")
                .set_body_string(r#"{"id":"ch_1"}"#),
        )
        .mount(&server)
        .await;
    Mock::given(path("/v1/charges/ch_9"))
        .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"error":"missing"}"#))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_native_response_matches_the_interpreter() {
    let project = project_of(RESPONSE_PROGRAM);
    let server = response_provider().await;
    let traces = tempfile::tempdir().unwrap();

    let (interpreted, interpreter_trace) = interpreter_real(&project, &server, traces.path()).await;
    let expected = "201 \"v7\" ch_1 no cookie absent ch_1 missing 201";
    assert_eq!(interpreted, Value::String(expected.into()));

    let binary = native_binary(&project);
    let native_trace = traces.path().join("native.jsonl");
    let native = run_native(&binary, "real", Some(&server), &[("CORVID_TRACE_PATH", &native_trace)]);
    assert_eq!(native, expected);
    assert_eq!(run_events(&native_trace), run_events(&interpreter_trace), "both tiers record the same run");
    assert!(!std::fs::read_to_string(&native_trace).unwrap().contains("sess-secret-123"));

    drop(server);
    let replayed = run_native(&binary, "replay", None, &[("CORVID_REPLAY_TRACE_PATH", &native_trace)]);
    assert_eq!(replayed, expected, "native replay answers the headers from the recording");

    // Every response record, header list and looked-up header is freed.
    let output = Command::new(&binary)
        .env("CORVID_CONNECTOR_MODE", "replay")
        .env("CORVID_REPLAY_TRACE_PATH", &native_trace)
        .env("CORVID_DEBUG_ALLOC", "1")
        .env_remove("CORVID_TRACE_PATH")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let count = |name: &str| -> i64 {
        stderr.lines().find_map(|line| line.strip_prefix(&format!("{name}="))).unwrap().trim().parse().unwrap()
    };
    assert_eq!(count("ALLOCS"), count("RELEASES"), "{stderr}");
}

/// A recording from either tier replays on the other (native dispatch
/// N5), with the provider gone: the interpreter replays a native
/// recording and native code replays the interpreter's, for plain
/// connector calls and for responses with their headers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_tier_replays_the_other_tiers_recording() {
    for (program, provider_answers) in [(PROGRAM, false), (RESPONSE_PROGRAM, true)] {
        let project = project_of(program);
        let server = if provider_answers { response_provider().await } else { provider().await };
        let traces = tempfile::tempdir().unwrap();
        let binary = native_binary(&project);

        let native_trace = traces.path().join("native.jsonl");
        let recorded_natively = run_native(&binary, "real", Some(&server), &[("CORVID_TRACE_PATH", &native_trace)]);
        let (interpreted, interpreter_trace) = interpreter_real(&project, &server, traces.path()).await;
        drop(server);

        assert_eq!(
            interpreter_replay(&project, &native_trace).await,
            Value::String(recorded_natively.clone().into()),
            "the interpreter replays a native recording"
        );
        let replayed_natively = run_native(&binary, "replay", None, &[("CORVID_REPLAY_TRACE_PATH", &interpreter_trace)]);
        assert_eq!(Value::String(replayed_natively.into()), interpreted, "native code replays an interpreter recording");
    }
}
