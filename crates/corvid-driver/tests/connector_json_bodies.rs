//! A connector speaks public JSON (B05 slice 3): what `T.to_json()`
//! writes and `T.from_json(text)` reads, for bodies, responses and the
//! query a record parameter becomes. These run a compiled program in
//! real mode against a wiremock provider reached under a public name.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_vm::Value;
use wiremock::matchers::{body_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TYPES: &str = r#"
effect http_write:
    cost: 1.0

type NewCharge:
    amount: Int
    note: Option<String>

type Charge:
    id: String
    amount: Int
    receipt_url: Option<String>

type ChargeError:
    | Declined
"#;

fn program(operations: &str, main: &str) -> String {
    format!(
        r#"{TYPES}
connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [real]
{operations}
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

fn runtime(ir: &corvid_ir::IrFile, server: &MockServer) -> Runtime {
    Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.payments.example".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("api.payments.example", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(ir))
        .build()
}

const CHARGE: &str = r#"    operation charge(new: NewCharge) -> Charge uses http_write:
        POST "/v1/charges" body new
"#;

fn text(value: &str) -> Value {
    Value::String(std::sync::Arc::from(value))
}

/// An absent `Option` field is sent as `null`, not as Corvid's internal
/// `{"tag":"none"}`; a present one as its plain value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_option_field_is_sent_as_its_value_or_null() {
    let src = program(
        CHARGE,
        r#"agent main() -> String:
    first = charge(NewCharge(500, None))
    second = charge(NewCharge(700, Some("gift")))
    return first.id + second.id
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 500, "note": null })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"ch_1","amount":500,"receipt_url":null}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 700, "note": "gift" })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"ch_2","amount":700,"receipt_url":"https://r.example/2"}"#,
        ))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server)).await.expect("charges");

    assert_eq!(result, text("ch_1ch_2"));
}

/// A response is read the way `Charge.from_json(text)` reads it: `null`
/// is `None`, a value is `Some`, and a field the type does not declare is
/// ignored, because providers add fields without notice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_response_decodes_options_and_ignores_undeclared_fields() {
    let src = program(
        CHARGE,
        r#"agent receipt_of(url: Option<String>) -> String:
    return match url:
        Some(u) -> u
        None -> "no receipt"

agent main() -> String:
    c = charge(NewCharge(500, None))
    return receipt_of(c.receipt_url)
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"ch_1","amount":500,"receipt_url":"https://r.example/1","livemode":false}"#,
        ))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server)).await.expect("decodes");

    assert_eq!(result, text("https://r.example/1"));
}

/// A response that does not match the declared type fails with B01's
/// error: the field path, what was expected and what was found.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mismatched_response_names_the_field_path() {
    let src = program(
        CHARGE,
        r#"agent main() -> String:
    c = charge(NewCharge(500, None))
    return c.id
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"ch_1","amount":"five hundred","receipt_url":null}"#,
        ))
        .mount(&server)
        .await;

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server))
        .await
        .expect_err("a string where Int is declared must fail");

    let message = err.to_string();
    assert!(message.contains("connector `charge`"), "{message}");
    assert!(message.contains("does not match `Charge`"), "{message}");
    assert!(message.contains("at `amount`"), "{message}");
}

/// A 2xx body that is not JSON is a failure naming the status, never a
/// silent `null` the program would read as an answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_body_that_is_not_json_is_refused() {
    let src = program(
        CHARGE,
        r#"agent main() -> String:
    c = charge(NewCharge(500, None))
    return c.id
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
        .mount(&server)
        .await;

    let err = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server))
        .await
        .expect_err("HTML is not a Charge");

    assert!(err.to_string().contains("HTTP 200 with a body that is not JSON"), "{err}");
}

/// A `Result` operation decodes its success body through the public
/// mapping and still turns a mapped status into the typed error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_result_operation_decodes_its_body_and_maps_its_errors() {
    let src = program(
        r#"    operation charge(new: NewCharge) -> Result<Charge, ChargeError> uses http_write:
        POST "/v1/charges" body new
        on status 402 -> Declined
"#,
        r#"agent receipt_of(url: Option<String>) -> String:
    return match url:
        Some(u) -> u
        None -> "none"

agent outcome(r: Result<Charge, ChargeError>) -> String:
    return match r:
        Ok(c) -> receipt_of(c.receipt_url)
        Err(e) -> "declined"

agent main() -> String:
    ok = charge(NewCharge(500, None))
    declined = charge(NewCharge(9900, None))
    return outcome(ok) + " " + outcome(declined)
"#,
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 500, "note": null })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"ch_1","amount":500,"receipt_url":null}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(path("/v1/charges"))
        .and(body_json(serde_json::json!({ "amount": 9900, "note": null })))
        .respond_with(ResponseTemplate::new(402))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server)).await.expect("runs");

    assert_eq!(result, text("none declined"));
}

/// A `query` record becomes the query string through the same mapping:
/// a `None` field is left out and a list repeats its name. `header`
/// lines send a parameter and a fixed value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_query_record_and_header_lines_reach_the_provider() {
    let src = format!(
        r#"{TYPES}
type ChargeQuery:
    customer: String
    limit: Option<Int>
    status: List<String>

connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [real]
    operation charges(filter: ChargeQuery, request_id: String) -> List<Charge> uses http_write:
        GET "/v1/charges" query filter
        header "X-Request-Id": request_id
        header "Accept": "application/json"

agent main() -> Int:
    found = charges(ChargeQuery("cus 1", None, ["paid", "refunded"]), "req-42")
    return found.length()
"#
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/charges"))
        .and(query_param("customer", "cus 1"))
        .and(query_param("status", "paid"))
        .and(query_param("status", "refunded"))
        .and(query_param_is_missing("limit"))
        .and(header("x-request-id", "req-42"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[{"id":"ch_1","amount":5,"receipt_url":null},{"id":"ch_2","amount":6,"receipt_url":null}]"#,
        ))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server)).await.expect("lists");

    assert_eq!(result, Value::Int(2));
    let sent = &server.received_requests().await.unwrap()[0];
    assert_eq!(sent.url.query(), Some("customer=cus%201&status=paid&status=refunded"));
}

/// A query field of plain variants goes out by the variant's name, and
/// `Bytes` as its base64url text: exactly the text a Corvid route reads
/// the same field from.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_query_sends_plain_variants_by_name_and_bytes_as_text() {
    let src = format!(
        r#"{TYPES}
type ChargeStatus:
    | Paid
    | Refunded

type ChargeQuery:
    status: List<ChargeStatus>
    fingerprint: Bytes

connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [real]
    operation charges(filter: ChargeQuery) -> List<Charge> uses http_write:
        GET "/v1/charges" query filter

agent main() -> Int:
    found = charges(ChargeQuery([Paid, Refunded], "hi".utf8()))
    return found.length()
"#
    );
    let (_dir, ir) = compile(&src);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/charges"))
        .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
        .mount(&server)
        .await;

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime(&ir, &server)).await.expect("lists");

    assert_eq!(result, Value::Int(0));
    let sent = &server.received_requests().await.unwrap()[0];
    assert_eq!(sent.url.query(), Some("fingerprint=aGk&status=Paid&status=Refunded"));
}
