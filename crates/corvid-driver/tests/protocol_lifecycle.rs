//! Durable verified-provider-protocol execution (slice 52h-2).
//!
//! Proves the lifecycle end-to-end against a loopback provider: submit
//! once, bind the provider job id from the TYPED response, poll through
//! the declared transition table, and return only on a terminal state —
//! never treating the submit response as completion. Also proves the
//! durability precondition: a protocol refuses to run outside a durable
//! job, because its intent has to survive a restart.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{DurableQueueRuntime, HttpClient, HttpEgressPolicy, ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn source(token_env: &str) -> String {
    format!(
        r#"
effect http_write:
    cost: 1.0

type Job:
    id: String
    status: String

connector shipping:
    base_url: "http://api.example.com"
    auth: bearer(secret("{token_env}"))
    modes: [real]
    operation submit_shipment(order: String) -> Job dangerous uses http_write:
        POST "/shipments" body order
        async:
            statuses: [queued, processing, completed, failed]
            initial: queued
            terminal: [completed, failed]
            deadline: 600s
            deadline_target: failed
            idempotency: intent
            poll GET "/shipments/{{id}}"
            every: 1s
            state queued:
                on queued -> queued
                on processing -> processing
                on completed -> completed
                on failed -> failed
            state processing:
                on queued -> processing
                on processing -> processing
                on completed -> completed
                on failed -> failed

agent main() -> Job uses http_write:
    approve SubmitShipment("order-1")
    return submit_shipment("order-1")
"#
    )
}

/// A protocol whose connector declares `circuit_breaker: 2` — two
/// consecutive failed observations trip it (slice 52h-3).
fn source_with_breaker(token_env: &str) -> String {
    source(token_env).replace(
        "    modes: [real]",
        "    circuit_breaker: 2\n    modes: [real]",
    )
}

fn compile(dir: &std::path::Path, src: &str) -> corvid_ir::IrFile {
    let p = dir.join("main.cor");
    std::fs::write(&p, src).unwrap();
    compile_to_ir_with_config_at_path(src, &p, None).expect("protocol program compiles")
}

fn loopback_client(host: &str, server: &MockServer) -> reqwest::Client {
    reqwest::Client::builder()
        .resolve(host, *server.address())
        .build()
        .expect("reqwest client builds")
}

/// The load-bearing test: one submit, the provider job id taken from the
/// typed submit response, polls walking `queued → processing →
/// completed`, and the TERMINAL observation returned — not the submit
/// response.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_submits_once_binds_the_job_id_and_returns_only_on_terminal() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    // Submit: answers with the provider's OWN job id. `expect(1)` is the
    // exactly-once assertion — a resumed or retried intent must not
    // create a second provider job.
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-9","status":"queued"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    // First poll: still working. The path proves `{id}` was bound from
    // the submit RESPONSE, not from the call argument ("order-1").
    Mock::given(method("GET"))
        .and(path("/shipments/prov-9"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-9","status":"processing"}"#),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second poll: terminal.
    Mock::given(method("GET"))
        .and(path("/shipments/prov-9"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-9","status":"completed"}"#),
        )
        .mount(&server)
        .await;

    // A durable job is the precondition for a protocol: its intent and
    // every transition are recorded as this job's checkpoints.
    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");

    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build()
        .with_durable_job(queue.clone(), job.id.clone());

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("the protocol runs to a terminal state");
    std::env::remove_var(TOKEN_ENV);

    // The value is the TERMINAL observation, never the submit response.
    match result {
        Value::Struct(s) => {
            assert_eq!(
                s.get_field("status").unwrap(),
                Value::String(Arc::from("completed")),
                "the call must return the terminal observation, not the submit response"
            );
        }
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }

    // Exactly one submit (wiremock's `.expect(1)` verifies on drop), and
    // the intent's transitions were checkpointed on the durable job.
    let checkpoints = queue.list_checkpoints(&job.id).expect("checkpoints");
    assert!(
        checkpoints.len() >= 3,
        "intent + submit-bound + each transition must be checkpointed; got {}",
        checkpoints.len()
    );
    let last = checkpoints.last().unwrap();
    let state = last.payload.get("state").and_then(|s| s.as_str());
    assert_eq!(
        state,
        Some("completed"),
        "the final checkpoint must record the terminal state so a resume is a no-op"
    );
    let history = last
        .payload
        .get("status_history")
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        history.len(),
        2,
        "both observations must be recorded as transition evidence"
    );
}

/// Exactly-once across a RESTART: re-running the same durable job
/// re-finds the completed intent from its checkpoints and returns the
/// recorded terminal observation — without submitting a second provider
/// job or issuing another poll. This is the property that makes a lost
/// response or a crashed process safe.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resuming_the_same_job_never_submits_a_second_provider_job() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_RESUME";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    // Exactly ONE submit and exactly ONE poll may happen across BOTH
    // runs — the second run must be served entirely from the durable
    // intent.
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-1","status":"queued"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/shipments/prov-1"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-1","status":"completed"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");

    let build_runtime = || {
        Runtime::builder()
            .approver(Arc::new(ProgrammaticApprover::always_yes()))
            .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
            .http_client(HttpClient::with_reqwest_client(loopback_client(
                "api.example.com",
                &server,
            )))
            .connector_mode(Some(corvid_ast::ConnectorMode::Real))
            .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
            .build()
            .with_durable_job(queue.clone(), job.id.clone())
    };

    // First run: submits, polls once, reaches terminal.
    let first = run_ir_with_runtime(&ir, None, vec![], &build_runtime())
        .await
        .expect("first run completes");

    // Second run — a fresh Runtime against the SAME durable job, as a
    // restart would produce. No provider traffic may occur.
    let second = run_ir_with_runtime(&ir, None, vec![], &build_runtime())
        .await
        .expect("resumed run completes from the durable intent");
    std::env::remove_var(TOKEN_ENV);

    assert_eq!(
        first, second,
        "a resumed run must be indistinguishable from the original"
    );
    match second {
        Value::Struct(s) => assert_eq!(
            s.get_field("status").unwrap(),
            Value::String(Arc::from("completed"))
        ),
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }
    // wiremock's `.expect(1)` on both mocks verifies on drop: exactly one
    // submit and one poll across the two runs.
}

/// Governed cadence (slice 52h-3): a provider's `Retry-After` is
/// honoured, and the elapsed time proves it — the run must take at least
/// the requested backoff, which is longer than the declared 1s interval.
/// A provider can slow us down; it can never speed us past what the
/// source declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_retry_after_slows_the_declared_cadence() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_RA";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-ra","status":"queued"}"#),
        )
        .mount(&server)
        .await;
    // First poll: still working, and ask for a 3s backoff — well above
    // the declared 1s cadence.
    Mock::given(method("GET"))
        .and(path("/shipments/prov-ra"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("retry-after", "3")
                .set_body_string(r#"{"id":"prov-ra","status":"processing"}"#),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/shipments/prov-ra"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-ra","status":"completed"}"#),
        )
        .mount(&server)
        .await;

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");

    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build()
        .with_durable_job(queue.clone(), job.id.clone());

    let started = std::time::Instant::now();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("the protocol completes");
    let elapsed = started.elapsed();
    std::env::remove_var(TOKEN_ENV);

    match result {
        Value::Struct(s) => assert_eq!(
            s.get_field("status").unwrap(),
            Value::String(Arc::from("completed"))
        ),
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }
    // 1s (first poll, declared cadence) + 3s (honouring Retry-After
    // before the second poll) — so comfortably over 3s. Without the
    // Retry-After it would be ~2s.
    assert!(
        elapsed >= std::time::Duration::from_millis(3_500),
        "the provider's Retry-After must slow the loop; took {elapsed:?}"
    );
}

/// Circuit-breaker admission, tolerant half (slice 52h-3): a TRANSIENT
/// failed observation must not kill a long protocol — the submitted job
/// is still out there, and one bad poll says nothing about it. The loop
/// retries on the next tick and still reaches terminal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_transient_poll_failure_is_tolerated_and_the_protocol_still_completes() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_CB1";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source_with_breaker(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-cb","status":"queued"}"#),
        )
        .mount(&server)
        .await;
    // One transient 500 — below the breaker threshold of 2.
    Mock::given(method("GET"))
        .and(path("/shipments/prov-cb"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream hiccup"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/shipments/prov-cb"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-cb","status":"completed"}"#),
        )
        .mount(&server)
        .await;

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build()
        .with_durable_job(queue.clone(), job.id.clone());

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("a transient poll failure must not kill the protocol");
    std::env::remove_var(TOKEN_ENV);

    match result {
        Value::Struct(s) => assert_eq!(
            s.get_field("status").unwrap(),
            Value::String(Arc::from("completed"))
        ),
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }
}

/// Circuit-breaker admission, tripping half (slice 52h-3): polling a
/// PERSISTENTLY broken provider forever is its own failure, so N
/// consecutive failed observations trip the breaker. The diagnostic must
/// say the provider job was NOT cancelled — the intent stays recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consecutive_failed_observations_trip_the_declared_circuit_breaker() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_CB2";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source_with_breaker(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-dead","status":"queued"}"#),
        )
        .mount(&server)
        .await;
    // Always broken.
    Mock::given(method("GET"))
        .and(path("/shipments/prov-dead"))
        .respond_with(ResponseTemplate::new(500).set_body_string("down"))
        .mount(&server)
        .await;

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build()
        .with_durable_job(queue.clone(), job.id.clone());

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("a persistently broken provider must trip the breaker");
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err:?}");
    assert!(
        text.contains("circuit breaker open"),
        "the failure must name the tripped breaker; got: {text}"
    );
    assert!(
        text.contains("NOT cancelled"),
        "the diagnostic must state the provider job was not cancelled; got: {text}"
    );
    // The intent survives for a later resume — it is not discarded.
    let checkpoints = queue.list_checkpoints(&job.id).expect("checkpoints");
    assert!(
        !checkpoints.is_empty(),
        "the intent must remain recorded after the breaker trips"
    );
}

/// Semantic cancellation with a DECLARED cancel endpoint (slice 52h-3):
/// cancelling the durable job mid-flight compensates by calling the
/// provider's cancel endpoint — the placeholder bound from the submit
/// response, exactly like the poll path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_job_compensates_through_the_declared_cancel_endpoint() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_COMP";
    let dir = tempfile::tempdir().expect("tempdir");
    // Same protocol, plus a declared cancel endpoint.
    let src = source(TOKEN_ENV).replace(
        "            poll GET \"/shipments/{id}\"",
        "            poll GET \"/shipments/{id}\"\n            cancel POST \"/shipments/{id}/cancel\"",
    );
    let ir = compile(dir.path(), &src);
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-x","status":"queued"}"#),
        )
        .mount(&server)
        .await;
    // Never terminal, so the loop keeps observing until we cancel it.
    Mock::given(method("GET"))
        .and(path("/shipments/prov-x"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-x","status":"processing"}"#),
        )
        .mount(&server)
        .await;
    // The compensation call MUST happen — `expect(1)` verifies on drop,
    // and the path proves `{id}` came from the submit response.
    Mock::given(method("POST"))
        .and(path("/shipments/prov-x/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .expect(1)
        .mount(&server)
        .await;

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build()
        .with_durable_job(queue.clone(), job.id.clone());

    // Cancel the job shortly after the run starts, while it is polling.
    let canceller = {
        let queue = queue.clone();
        let job_id = job.id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1_500)).await;
            let _ = queue.cancel(&job_id);
        })
    };

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("a cancelled protocol does not return a terminal value");
    let _ = canceller.await;
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err:?}");
    assert!(
        text.contains("compensated"),
        "a declared cancel endpoint must be used to compensate; got: {text}"
    );
}

/// Durability precondition: a protocol operation invoked OUTSIDE a
/// durable job refuses, rather than silently degrading to a non-durable
/// poll loop whose intent would be lost on restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_refuses_to_run_outside_a_durable_job() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_TOKEN_NODJ";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-proto");

    let server = MockServer::start().await;
    // No request must reach the provider at all.
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"x","status":"queued"}"#))
        .expect(0)
        .mount(&server)
        .await;

    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .http_policy(HttpEgressPolicy::new(Some(&["api.example.com".to_string()])))
        .http_client(HttpClient::with_reqwest_client(loopback_client(
            "api.example.com",
            &server,
        )))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build(); // <- deliberately NOT bound to a durable job

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("a protocol without a durable job must refuse");
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err:?}");
    assert!(
        text.contains("durable job"),
        "the refusal must name the durable-job requirement; got: {text}"
    );
}
