//! Durable verified-provider-protocol execution.
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
            on_protocol_change: refuse
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
/// consecutive failed observations trip it.
fn source_with_breaker(token_env: &str) -> String {
    source(token_env).replace(
        "    modes: [real]",
        "    circuit_breaker: 2\n    modes: [real]",
    )
}

/// A protocol whose graph can be edited (`deadline_secs`) and whose
/// resume posture can be chosen, for the migration tests.
/// `circuit_breaker: 1` lets a run abort mid-flight so the next run finds
/// a genuinely IN-FLIGHT intent rather than a completed one.
fn source_migration(token_env: &str, deadline_secs: u64, policy: &str) -> String {
    source(token_env)
        .replace("    modes: [real]", "    circuit_breaker: 1\n    modes: [real]")
        .replace("deadline: 600s", &format!("deadline: {deadline_secs}s"))
        .replace(
            "on_protocol_change: refuse",
            &format!("on_protocol_change: {policy}"),
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

/// Governed cadence: a provider's `Retry-After` is
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

/// Circuit-breaker admission, tolerant half: a TRANSIENT
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

/// Circuit-breaker admission, tripping half: polling a
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

/// Semantic cancellation with a DECLARED cancel endpoint:
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

/// A protocol that changed under an in-flight intent.
///
/// The intent was created against one graph; the deployed declaration is
/// now a different one. A live provider job exists that Corvid cannot
/// un-create, so `on_protocol_change: refuse` means exactly that: the run
/// refuses, the intent stays checkpointed, and the provider sees nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protocol_change_refuses_to_resume_an_in_flight_intent_and_never_resubmits() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_MIGRATE_REFUSE";
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var(TOKEN_ENV, "tok-migrate");

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");

    // --- Run 1: submit lands, the first observation fails, and
    // `circuit_breaker: 1` aborts the run with the intent IN FLIGHT.
    {
        let ir = compile(dir.path(), &source_migration(TOKEN_ENV, 600, "refuse"));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/shipments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-m","status":"queued"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shipments/prov-m"))
            .respond_with(ResponseTemplate::new(500))
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
            .build()
            .with_durable_job(queue.clone(), job.id.clone());
        run_ir_with_runtime(&ir, None, vec![], &runtime)
            .await
            .expect_err("the breaker aborts this run with the intent still in flight");
    }

    // Precondition for the test to mean anything: a submitted, non-terminal
    // intent is on the job.
    let last = queue
        .list_checkpoints(&job.id)
        .expect("checkpoints")
        .pop()
        .expect("an in-flight intent was recorded");
    assert_eq!(
        last.payload.get("submitted").and_then(|v| v.as_bool()),
        Some(true),
        "the provider job exists, so the intent must be marked submitted"
    );
    assert_eq!(
        last.payload.get("state").and_then(|v| v.as_str()),
        Some("queued"),
        "the intent must still be mid-flight, not terminal"
    );

    // --- Run 2: the protocol GRAPH changed (deadline 600s -> 900s).
    let ir = compile(dir.path(), &source_migration(TOKEN_ENV, 900, "refuse"));
    let server = MockServer::start().await;
    // The provider must see NOTHING — no re-submit, no poll.
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/shipments/prov-m"))
        .respond_with(ResponseTemplate::new(200))
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
        .build()
        .with_durable_job(queue.clone(), job.id.clone());
    let err = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect_err("a changed protocol must not silently resume a live provider job");
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err}");
    assert!(
        text.contains("changed while an intent was in flight"),
        "the refusal must name what happened; got: {text}"
    );
    assert!(
        text.contains("on_protocol_change: refuse"),
        "the refusal must name the declaration that caused it; got: {text}"
    );
    // The honesty requirement: never imply the provider job went away.
    assert!(
        text.contains("still running"),
        "the refusal must say the provider job is still running; got: {text}"
    );
}

/// The other declared posture: `resume` continues an in-flight intent
/// under the new declaration — without re-submitting — when the recorded
/// state still exists in it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declared_resume_continues_an_in_flight_intent_across_a_protocol_change() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_MIGRATE_RESUME";
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var(TOKEN_ENV, "tok-migrate-2");

    let queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("queue"));
    let job = queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");

    {
        let ir = compile(dir.path(), &source_migration(TOKEN_ENV, 600, "resume"));
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/shipments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-n","status":"queued"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shipments/prov-n"))
            .respond_with(ResponseTemplate::new(500))
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
            .build()
            .with_durable_job(queue.clone(), job.id.clone());
        run_ir_with_runtime(&ir, None, vec![], &runtime)
            .await
            .expect_err("the breaker aborts this run with the intent still in flight");
    }

    // The protocol changed, and the declaration permits resuming.
    let ir = compile(dir.path(), &source_migration(TOKEN_ENV, 900, "resume"));
    let server = MockServer::start().await;
    // Still exactly zero re-submits: resuming is not re-doing.
    Mock::given(method("POST"))
        .and(path("/shipments"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/shipments/prov-n"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-n","status":"completed"}"#),
        )
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
        .build()
        .with_durable_job(queue.clone(), job.id.clone());
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("a declared resume continues the in-flight intent");
    std::env::remove_var(TOKEN_ENV);

    match result {
        Value::Struct(s) => assert_eq!(
            s.get_field("status").unwrap(),
            Value::String(Arc::from("completed")),
            "the resumed intent must reach the terminal observation"
        ),
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }
}

/// The acceptance path for lifecycle replay.
///
/// A protocol is not one call, it is a lifecycle: a submit plus a
/// sequence of observations spread over real time. Replaying it must
/// reproduce that whole sequence from the recording and NEVER reach the
/// provider — otherwise "replay" would re-submit work that already
/// happened, the exact failure durable intent exists to prevent.
///
/// Records a real lifecycle, then replays the SAME file with no provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_lifecycle_replays_without_touching_the_provider() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_REPLAY_TOKEN";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-replay-secret");

    // --- Record: a real lifecycle (submit + two observations). ---
    let trace_dir = tempfile::tempdir().expect("trace dir");
    let trace_path = {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/shipments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-r","status":"queued"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shipments/prov-r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"id":"prov-r","status":"processing"}"#),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shipments/prov-r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"id":"prov-r","status":"completed"}"#),
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
            .trace_to(trace_dir.path())
            .build()
            .with_durable_job(queue.clone(), job.id.clone());
        run_ir_with_runtime(&ir, None, vec![], &runtime)
            .await
            .expect("the recorded lifecycle reaches a terminal state");
        let p = runtime.tracer().path().to_path_buf();
        drop(runtime);
        p
        // `server` drops here: from this point there is no provider.
    };

    // Each lifecycle boundary is recorded UNDER ITS OWN LABEL, so an
    // observation can never be substituted for a submit.
    let raw = std::fs::read_to_string(&trace_path).expect("trace readable");
    assert!(
        raw.contains(r#""tool":"submit_shipment""#),
        "the submit boundary must be recorded"
    );
    assert!(
        raw.contains(r#""tool":"submit_shipment.poll""#),
        "each observation boundary must be recorded under its own label"
    );
    // The credential is attached to a header inside the dispatch, so it
    // cannot reach the recording.
    assert!(
        !raw.contains("tok-replay-secret"),
        "the credential must never appear in a recorded lifecycle"
    );

    // --- Replay: same file, fresh durable job, NO provider. ---
    // A fresh job means the intent starts empty, so the lifecycle really
    // re-executes against the recording instead of short-circuiting on an
    // already-terminal checkpoint.
    let replay_queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("replay queue"));
    let replay_job = replay_queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");
    let replay_runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&trace_path)
        .build()
        .with_durable_job(replay_queue.clone(), replay_job.id.clone());
    let replayed = run_ir_with_runtime(&ir, None, vec![], &replay_runtime)
        .await
        .expect("replay reproduces the lifecycle from the recording");
    std::env::remove_var(TOKEN_ENV);

    match replayed {
        Value::Struct(s) => assert_eq!(
            s.get_field("status").unwrap(),
            Value::String(Arc::from("completed")),
            "replay must reach the same terminal observation"
        ),
        other => panic!("expected the terminal Job struct; got {other:?}"),
    }

    // Replay walked the same transitions and checkpointed them, so a
    // replayed lifecycle is auditable exactly like the original.
    let checkpoints = replay_queue
        .list_checkpoints(&replay_job.id)
        .expect("checkpoints");
    let last = checkpoints.last().expect("replay checkpoints the lifecycle");
    assert_eq!(
        last.payload.get("state").and_then(|s| s.as_str()),
        Some("completed"),
        "the replayed intent must end in the same terminal state"
    );
}

/// Strict no-real-fallback, adversarially. When the recording does not
/// cover an exchange, replay must REFUSE — never quietly finish the
/// lifecycle by asking the live provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lifecycle_replay_missing_an_observation_refuses_instead_of_calling_the_provider() {
    const TOKEN_ENV: &str = "CORVID_TEST_PROTO_REPLAY_GAP_TOKEN";
    let dir = tempfile::tempdir().expect("tempdir");
    let ir = compile(dir.path(), &source(TOKEN_ENV));
    std::env::set_var(TOKEN_ENV, "tok-gap");

    let trace_dir = tempfile::tempdir().expect("trace dir");
    let trace_path = {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/shipments"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":"prov-g","status":"queued"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/shipments/prov-g"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"id":"prov-g","status":"completed"}"#),
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
            .trace_to(trace_dir.path())
            .build()
            .with_durable_job(queue.clone(), job.id.clone());
        run_ir_with_runtime(&ir, None, vec![], &runtime)
            .await
            .expect("recorded run completes");
        let p = runtime.tracer().path().to_path_buf();
        drop(runtime);
        p
    };

    // Excise the observations, keeping the submit. The recording now
    // describes a lifecycle that was never finished.
    let gapped = trace_path.with_extension("gapped.jsonl");
    let kept: String = std::fs::read_to_string(&trace_path)
        .expect("trace readable")
        .lines()
        .filter(|line| !line.contains("submit_shipment.poll"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&gapped, kept).expect("write gapped trace");

    let replay_queue = Arc::new(DurableQueueRuntime::open_in_memory().expect("replay queue"));
    let replay_job = replay_queue
        .enqueue("main", serde_json::json!([]), 0, 0.0, None, None)
        .expect("enqueue");
    let replay_runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .connector_mode(Some(corvid_ast::ConnectorMode::Replay))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .replay_from(&gapped)
        .build()
        .with_durable_job(replay_queue.clone(), replay_job.id.clone());
    let started = std::time::Instant::now();
    let err = run_ir_with_runtime(&ir, None, vec![], &replay_runtime)
        .await
        .expect_err("an uncovered observation must refuse, not fall through to the provider");
    let elapsed = started.elapsed();
    std::env::remove_var(TOKEN_ENV);

    let text = format!("{err}").to_lowercase();
    assert!(
        text.contains("diverge"),
        "the refusal must be a replay divergence, not a network failure; got: {text}"
    );
    // It must refuse AT THE GAP. The circuit breaker exists for a
    // provider's transient unwellness, and in replay there is no provider
    // — absorbing the divergence would spin against a cursor that never
    // advances and finally report the missing recording as a provider
    // timeout, which is a different (and false) claim.
    assert!(
        !text.contains("deadline"),
        "a gap in the recording must not be reported as a provider deadline; got: {text}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "the divergence must be fatal immediately, not tolerated until the declared deadline \
         (took {elapsed:?})"
    );
}
