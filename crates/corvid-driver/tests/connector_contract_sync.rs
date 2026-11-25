//! The Application Contract and the runtime describe a connector's
//! request the same way (B05 slice 3d). The contract names each
//! operation's body and query types and describes them in `types`; the
//! OpenAPI document built from it gives their JSON Schema. This test runs
//! a real connector call, captures what the provider received, and checks
//! it against that schema, so the contract, the generated clients that
//! read it, and the runtime cannot drift apart unnoticed.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use serde_json::{json, Value as J};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = r#"
effect http_write:
    cost: 1.0

type Address:
    city: String
    postcode: Option<String>

type Priority:
    | Low
    | High

type NewOrder:
    reference: String
    note: Option<String>
    gift_note: Option<String>
    tags: List<String>
    price: Decimal
    quantity: Int
    express: Bool
    ship_to: Address
    priority: Priority

type OrderFilter:
    customer: String
    limit: Option<Int>
    status: List<String>

type Order:
    id: String

connector shop:
    base_url: "http://api.shop.example"
    timeout: 10s
    redirects: never
    modes: [real]
    operation place(filter: OrderFilter, order: NewOrder, request_id: String) -> Order uses http_write:
        POST "/v1/orders" query filter body order
        header "X-Request-Id": request_id

agent main() -> String uses http_write:
    price: Decimal = 19.99
    order = NewOrder("ref-1", None, Some("happy birthday"), ["a", "b"], price, 2, true, Address("Berlin", None), High)
    placed = place(OrderFilter("cus_1", None, ["open"]), order, "req-1")
    return placed.id
"#;

/// Check `value` against `schema`, resolving `$ref` in `components`.
/// Covers the keywords the OpenAPI emitter uses for structure: `type`,
/// `properties`, `required`, `additionalProperties: false`, `items`,
/// `prefixItems`, `minItems`/`maxItems`, `anyOf`, `oneOf`, `const` and
/// `$ref`. Annotations (`format`, `pattern`, `description`,
/// `contentEncoding`) are not checked here; the test asserts the
/// decimal's exact text separately.
fn conforms(value: &J, schema: &J, components: &J, at: &str) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(J::as_str) {
        let name = reference.trim_start_matches("#/components/schemas/");
        let target = components.get(name).ok_or_else(|| format!("{at}: `{reference}` is not in the contract"))?;
        return conforms(value, target, components, at);
    }
    if let Some(options) = schema.get("anyOf").or_else(|| schema.get("oneOf")).and_then(J::as_array) {
        let matching = options.iter().filter(|option| conforms(value, option, components, at).is_ok()).count();
        let one_of = schema.get("oneOf").is_some();
        return match (matching, one_of) {
            (0, _) => Err(format!("{at}: {value} matches none of the alternatives")),
            (1, _) | (_, false) => Ok(()),
            (_, true) => Err(format!("{at}: {value} matches more than one `oneOf` alternative")),
        };
    }
    if let Some(expected) = schema.get("const") {
        return if value == expected { Ok(()) } else { Err(format!("{at}: expected {expected}, got {value}")) };
    }
    if let Some(kind) = schema.get("type") {
        let kinds: Vec<&str> = match kind {
            J::String(k) => vec![k.as_str()],
            J::Array(ks) => ks.iter().filter_map(J::as_str).collect(),
            _ => vec![],
        };
        let actual = match value {
            J::Null => "null",
            J::Bool(_) => "boolean",
            J::Number(n) if n.is_i64() || n.is_u64() => "integer",
            J::Number(_) => "number",
            J::String(_) => "string",
            J::Array(_) => "array",
            J::Object(_) => "object",
        };
        let accepted = kinds.contains(&actual) || (actual == "integer" && kinds.contains(&"number"));
        if !accepted {
            return Err(format!("{at}: expected {kinds:?}, got {value}"));
        }
    }
    if let J::Object(fields) = value {
        let properties = schema.get("properties").and_then(J::as_object);
        for required in schema.get("required").and_then(J::as_array).into_iter().flatten() {
            let name = required.as_str().unwrap_or_default();
            if !fields.contains_key(name) {
                return Err(format!("{at}: required field `{name}` is missing"));
            }
        }
        for (name, field) in fields {
            match properties.and_then(|p| p.get(name)) {
                Some(field_schema) => conforms(field, field_schema, components, &format!("{at}.{name}"))?,
                None if schema.get("additionalProperties") == Some(&J::Bool(false)) => {
                    return Err(format!("{at}: `{name}` is not a declared field"));
                }
                None => {}
            }
        }
    }
    if let J::Array(items) = value {
        if let Some(min) = schema.get("minItems").and_then(J::as_u64) {
            if (items.len() as u64) < min {
                return Err(format!("{at}: fewer than {min} items"));
            }
        }
        if let Some(max) = schema.get("maxItems").and_then(J::as_u64) {
            if (items.len() as u64) > max {
                return Err(format!("{at}: more than {max} items"));
            }
        }
        let prefix = schema.get("prefixItems").and_then(J::as_array);
        for (i, item) in items.iter().enumerate() {
            let item_schema = prefix.and_then(|p| p.get(i)).or_else(|| schema.get("items"));
            if let Some(item_schema) = item_schema {
                conforms(item, item_schema, components, &format!("{at}[{i}]"))?;
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_connector_sends_matches_the_contract_schema() {
    let dir = tempfile::tempdir().unwrap();
    let main_path = dir.path().join("main.cor");
    std::fs::write(&main_path, SOURCE).unwrap();

    // The contract names the operation's body and query types and
    // describes them, though none is public.
    let contract = compile_to_application_contract_with_config(SOURCE, "main.cor", "test", None)
        .expect("contract builds");
    let operation = &contract.connectors[0].operations[0];
    assert_eq!(operation.body_type.as_deref(), Some("NewOrder"));
    assert_eq!(operation.body_encoding.as_deref(), Some("json"));
    assert_eq!(operation.query_type.as_deref(), Some("OrderFilter"));
    assert_eq!(operation.response_type, "Order");
    assert_eq!(operation.headers, vec!["X-Request-Id".to_string()]);
    for name in ["NewOrder", "Address", "Priority", "OrderFilter", "Order"] {
        assert!(contract.types.iter().any(|t| t.name == name), "`{name}` is described in the contract");
    }
    let openapi = corvid_abi::openapi::emit_openapi(&contract);
    let components = &openapi["components"]["schemas"];

    // The runtime sends a real request.
    let ir = compile_to_ir_with_config_at_path(SOURCE, &main_path, None).expect("compiles");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"ord_1"}"#))
        .mount(&server)
        .await;
    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::new(Some(&["api.shop.example".to_string()])))
        .http_client(HttpClient::with_reqwest_builder(
            reqwest::Client::builder().resolve("api.shop.example", *server.address()),
        ))
        .connector_mode(Some(corvid_ast::ConnectorMode::Real))
        .connector_calls(corvid_runtime::connectors::connector_calls_from_ir(&ir))
        .build();
    run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("the call succeeds");
    let sent = &server.received_requests().await.unwrap()[0];

    // The body conforms to the contract's schema for `NewOrder`.
    let body: J = serde_json::from_slice(&sent.body).expect("the body is JSON");
    conforms(&body, &json!({ "$ref": "#/components/schemas/NewOrder" }), components, "$")
        .unwrap_or_else(|problem| panic!("the sent body drifted from the contract: {problem}\nbody: {body}"));
    assert_eq!(body["price"], json!("19.99"), "a decimal is sent as its exact text");
    assert_eq!(body["note"], J::Null, "an absent Option is null");
    assert_eq!(body["gift_note"], json!("happy birthday"));

    // Every query parameter is a field of the contract's `OrderFilter`.
    let filter = &components["OrderFilter"]["properties"];
    for (name, _) in sent.url.query_pairs() {
        assert!(filter.get(name.as_ref()).is_some(), "query `{name}` is not a field of OrderFilter");
    }
    assert_eq!(sent.url.query(), Some("customer=cus_1&status=open"));

    // The validator itself refuses a body the schema does not describe.
    let mut drifted = body.clone();
    drifted["note"] = json!({ "tag": "none" });
    assert!(conforms(&drifted, &json!({ "$ref": "#/components/schemas/NewOrder" }), components, "$").is_err());
}
