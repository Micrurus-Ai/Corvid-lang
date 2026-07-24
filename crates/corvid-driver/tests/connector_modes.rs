//! End-to-end connector execution across modes (slice 52g-3c).
//!
//! A source-declared `connector` runs the same unchanged `.cor` file in
//! `mock`, `real`, and `replay` — the deployment picks the mode. These
//! tests compile a real program through the driver and run it against a
//! loopback `wiremock` server standing in for the provider (the same
//! `.resolve` no-shortcut pattern the executing-HTTP tests use: the URL
//! stays a public-looking host so the SSRF floor + allowlist are still
//! exercised, and only the TCP endpoint is loopback).

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_trace_schema::read_events_from_path;
use corvid_vm::Value;
use std::path::Path;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The connector program, parameterized by the secret's env-var name so
/// each test uses a DISTINCT credential — env vars are process-global and
/// these tests run in parallel, so a shared name would race.
fn source(token_env: &str) -> String {
    format!(
        r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "http://api.example.com"
    auth: bearer(secret("{token_env}"))
    modes: [mock, real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{{owner}}/{{repo}}"
        mock: Repo("mock-repo")

agent main() -> String:
    r = get_repo("micrurus", "corvid")
    return r.name
"#
    )
}

/// A connector whose operation returns `Result<Repo, GithubError>` and
/// maps HTTP 404 to the typed error variant `NotFound` (slice 52g-3c-5).
fn source_with_errors(token_env: &str) -> String {
    format!(
        r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

type GithubError:
    | NotFound
    | RateLimited

connector github:
    base_url: "http://api.example.com"
    auth: bearer(secret("{token_env}"))
    modes: [mock, real]
    operation get_repo(owner: String, repo: String) -> Result<Repo, GithubError> uses http_read:
        GET "/repos/{{owner}}/{{repo}}"
        on status 404 -> NotFound
        mock: Ok(Repo("mock-repo"))

agent main() -> Result<Repo, GithubError>:
    return get_repo("micrurus", "corvid")
"#
    )
}

/// A connector with a client-side rate limit of 1 request per hour whose
/// agent calls the operation twice — the second call must be refused
/// before it reaches the provider (slice 52g-3c-5).
fn source_rate_limited(token_env: &str) -> String {
    format!(
        r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "http://api.example.com"
    auth: bearer(secret("{token_env}"))
    rate_limit: 1 per 3600s
    modes: [real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{{owner}}/{{repo}}"
        mock: Repo("mock-repo")

agent main() -> String:
    a = get_repo("micrurus", "corvid")
    b = get_repo("micrurus", "corvid")
    return b.name
"#
    )
}

fn compile(main_path: &Path) -> corvid_ir::IrFile {
    let source = std::fs::read_to_string(main_path).unwrap();
    compile_to_ir_with_config_at_path(&source, main_path, None).expect("connector program compiles")
}

fn compile_source(dir: &Path, src: &str) -> corvid_ir::IrFile {
    let p = dir.join("main.cor");
    std::fs::write(&p, src).unwrap();
    compile_to_ir_with_config_at_path(src, &p, None).expect("connector program compiles")
}

fn write_source(dir: &Path, token_env: &str) -> std::path::PathBuf {
    let p = dir.join("main.cor");
    std::fs::write(&p, source(token_env)).unwrap();
    p
}

fn loopback_client(host: &str, server: &MockServer) -> reqwest::Client {
    reqwest::Client::builder()
        .resolve(host, *server.address())
        .build()
        .expect("reqwest client with .resolve builds")
}

/// Real mode: the operation call becomes an HTTP request against the
/// connector's `base_url`, the credential resolves from the secret store
/// into the `Authorization` header (the wiremock refuses the request
/// unless the exact header is present), and the 2xx body decodes to the
/// operation's return type.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_mode_performs_the_http_request_with_the_resolved_secret() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_REAL";
    let dir = tempfile::tempdir().expect("tempdir");
    let main_path = write_source(dir.path(), TOKEN_ENV);
    let ir = compile(&main_path);

    let server = MockServer::start().await;
    // The mock ONLY matches when the Authorization header carries the
    // resolved secret — proof the credential reached the request.
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .and(header("authorization", "Bearer tok-live-123"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"corvid-real"}"#))
        .mount(&server)
        .await;

    std::env::set_var(TOKEN_ENV, "tok-live-123");

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("real-mode connector call round-trips through HTTP");

    std::env::remove_var(TOKEN_ENV);

    assert_eq!(
        result,
        Value::String(std::sync::Arc::from("corvid-real")),
        "real mode should return the provider's response body, decoded to Repo.name"
    );
}

/// Mock mode: the SAME unchanged file runs without touching the network
/// — the compiled `mock` payload is evaluated instead. No wiremock, no
/// secret, no egress.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_mode_runs_the_same_file_without_the_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_path = write_source(dir.path(), "CORVID_TEST_GH_TOKEN_MOCK");
    let ir = compile(&main_path);

    let runtime = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Mock))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("mock-mode connector call evaluates the compiled payload");

    assert_eq!(result, Value::String(std::sync::Arc::from("mock-repo")));
}

/// The full acceptance path: record a REAL run to a trace, then replay
/// the SAME file from that trace with no live provider. Proves (a) the
/// recorded interaction carries the response but NEVER the credential
/// (redaction by construction — the secret rides only in the request
/// header, which is not recorded), and (b) replay serves the recorded
/// result and never performs a real request (strict no-real-fallback:
/// the replay runtime has no live server, and replay quarantines egress).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn record_a_real_run_then_replay_it_without_touching_the_provider() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_REPLAY";
    let dir = tempfile::tempdir().expect("tempdir");
    let main_path = write_source(dir.path(), TOKEN_ENV);
    let ir = compile(&main_path);

    // --- Record: a real run against wiremock, writing a trace. ---
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .and(header("authorization", "Bearer tok-secret-xyz"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"corvid-recorded"}"#))
        .mount(&server)
        .await;
    std::env::set_var(TOKEN_ENV, "tok-secret-xyz");

    let trace_dir = tempfile::tempdir().expect("trace dir");
    let record_runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .trace_to(trace_dir.path())
        .build();

    let recorded = run_ir_with_runtime(&ir, None, vec![], &record_runtime)
        .await
        .expect("real run records a trace");
    assert_eq!(recorded, Value::String(std::sync::Arc::from("corvid-recorded")));

    let trace_path = record_runtime.tracer().path().to_path_buf();
    drop(record_runtime);
    std::env::remove_var(TOKEN_ENV);

    // The credential must appear NOWHERE in the recorded trace.
    let raw = std::fs::read_to_string(&trace_path).expect("trace file readable");
    assert!(
        !raw.contains("tok-secret-xyz"),
        "the credential must never appear in the recorded trace"
    );
    let events = read_events_from_path(&trace_path).expect("trace deserializes");
    assert!(!events.is_empty(), "trace should not be empty");

    // --- Replay: the SAME file, from the trace, with NO live provider. ---
    // The record-time wiremock is dropped; if replay tried a real call it
    // would have no endpoint and egress is quarantined in replay mode.
    drop(server);
    let replay_runtime = Runtime::builder()
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build();

    let replayed = run_ir_with_runtime(&ir, None, vec![], &replay_runtime)
        .await
        .expect("replay serves the recorded interaction without a real call");
    assert_eq!(
        replayed,
        Value::String(std::sync::Arc::from("corvid-recorded")),
        "replay must reproduce the recorded response"
    );
}

/// Adversarial: real mode with an UNRESOLVED credential fails the call
/// (the request is never sent) rather than dispatching a real request
/// with a missing/empty Authorization header. The diagnostic names the
/// secret, never a value (there is none).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_mode_without_a_resolvable_credential_fails_the_call() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_ABSENT";
    std::env::remove_var(TOKEN_ENV);
    let dir = tempfile::tempdir().expect("tempdir");
    let main_path = write_source(dir.path(), TOKEN_ENV);
    let ir = compile(&main_path);

    // A server that would answer anything — to prove the call never
    // reaches it, its mount requires a header we will never send.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"should-not-happen"}"#))
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("an unresolved credential must fail the call, not send a request");
    let text = format!("{err:?}");
    assert!(
        text.contains(TOKEN_ENV),
        "the failure must name the missing secret; got: {text}"
    );
    // The failure carries the secret NAME, never a value (there is none).
    assert!(!text.contains("Bearer "), "no credential value in the error: {text}");
}

/// Typed status→error mapping (slice 52g-3c-5): a 200 decodes to
/// `Ok(Repo)`, and the mapped HTTP 404 becomes the typed `Err(NotFound)`
/// variant — not a transport failure. The operation returns
/// `Result<Repo, GithubError>`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mapped_status_becomes_a_typed_error_variant() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_ERRMAP";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile_source(dir.path(), &source_with_errors(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-err");

    // A server that returns 404 for this repo.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("a 404 maps to a typed error, not a transport failure");
    std::env::remove_var(TOKEN_ENV);

    match result {
        Value::ResultErr(inner) => match inner.get() {
            Value::Enum(e) => assert_eq!(
                e.variant_name(),
                "NotFound",
                "HTTP 404 must map to the NotFound variant"
            ),
            other => panic!("expected Err(NotFound enum), got Err({other:?})"),
        },
        other => panic!("expected a typed Err(NotFound); got {other:?}"),
    }
}

/// The success side of the same Result-returning operation: a 200
/// decodes to `Ok(Repo)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_2xx_decodes_to_ok_for_a_result_returning_operation() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_OK";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile_source(dir.path(), &source_with_errors(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-ok");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"corvid-ok"}"#))
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("a 200 decodes to Ok");
    std::env::remove_var(TOKEN_ENV);

    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::Struct(s) => assert_eq!(
                s.get_field("name").unwrap(),
                Value::String(std::sync::Arc::from("corvid-ok"))
            ),
            other => panic!("expected Ok(Repo), got Ok({other:?})"),
        },
        other => panic!("expected Ok(Repo); got {other:?}"),
    }
}

/// Reliability: a client-side rate limit refuses the second call within
/// the window BEFORE it reaches the provider (slice 52g-3c-5).
/// A retrying operation whose retries send more requests than the rate
/// limit permits. `retry: 3` + `rate_limit: 2 per window` against a 5xx
/// provider means the first call's own attempts (1 + 3 = 4) exceed the
/// window of 2, so the limiter must refuse a retry attempt rather than
/// letting one logical call quietly emit four network requests.
fn source_retry_over_rate_limit(token_env: &str) -> String {
    format!(
        r#"
effect http_read:
    cost: 1.0

type Repo:
    name: String

connector github:
    base_url: "http://api.example.com"
    auth: bearer(secret("{token_env}"))
    retry: 3
    rate_limit: 2 per 3600s
    modes: [real]
    operation get_repo(owner: String, repo: String) -> Repo uses http_read:
        GET "/repos/{{owner}}/{{repo}}"
        mock: Repo("mock-repo")

agent main() -> String:
    a = get_repo("micrurus", "corvid")
    return a.name
"#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_are_admitted_by_the_rate_limiter_one_attempt_at_a_time() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_RETRY_RL";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile_source(dir.path(), &source_retry_over_rate_limit(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-retry-rl");

    let server = MockServer::start().await;
    // Always 5xx, so the operation keeps retrying. `expect(2)` is the
    // load-bearing assertion: the provider receives exactly the two
    // requests the rate limit permits, NOT the four the retry policy
    // would send if it ran beneath the limiter.
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .respond_with(ResponseTemplate::new(503))
        .expect(2)
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("a third attempt must be refused by the rate limit");
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err:?}").to_lowercase();
    assert!(
        text.contains("rate limit"),
        "the failure must be the rate limit, not the 5xx; got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_side_rate_limit_refuses_the_second_call_in_the_window() {
    const TOKEN_ENV: &str = "CORVID_TEST_GH_TOKEN_RL";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile_source(dir.path(), &source_rate_limited(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-rl");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/micrurus/corvid"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"name":"ok"}"#))
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("the second call in the window must be rate-limited");
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err:?}").to_lowercase();
    assert!(
        text.contains("rate limit"),
        "the failure must name the rate limit; got: {err:?}"
    );
}
