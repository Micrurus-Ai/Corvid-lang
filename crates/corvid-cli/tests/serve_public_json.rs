//! `corvid serve` honours the JSON contract it publishes.
//!
//! Each test starts a real server, reads its live `/openapi.json`, and
//! checks both directions against it: every body the schema accepts is
//! decoded, every reply validates against the response schema, and a
//! body the schema rejects receives a 400 naming the offending path.
//! The validator below covers the JSON Schema keywords Corvid emits.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

struct ServedApp {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(source: &str) -> ServedApp {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("main.cor");
    std::fs::write(&src, source).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let app = ServedApp { child, port, _dir: dir };
    let deadline = Instant::now() + Duration::from_secs(60);
    while request(port, "GET", "/openapi.json", None).is_none() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

fn request(port: u16, method: &str, path: &str, body: Option<&str>) -> Option<(u16, Value)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let body = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let reply = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    Some((status, serde_json::from_str(reply).unwrap_or(Value::String(reply.to_string()))))
}

fn post(app: &ServedApp, path: &str, body: &Value) -> (u16, Value) {
    request(app.port, "POST", path, Some(&body.to_string())).expect("request completes")
}

fn openapi(app: &ServedApp) -> Value {
    request(app.port, "GET", "/openapi.json", None).expect("openapi").1
}

/// The request and response schemas of `POST path`.
fn route_schemas(spec: &Value, path: &str) -> (Value, Value) {
    let op = &spec["paths"][path]["post"];
    (
        op["requestBody"]["content"]["application/json"]["schema"].clone(),
        op["responses"]["200"]["content"]["application/json"]["schema"].clone(),
    )
}

/// Validate `value` against the subset of JSON Schema 2020-12 that
/// Corvid's OpenAPI emitter produces. Returns the first violation.
fn validate(spec: &Value, schema: &Value, value: &Value, at: &str) -> Result<(), String> {
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference.strip_prefix("#/components/schemas/").expect("local reference");
        let target = &spec["components"]["schemas"][name];
        assert!(!target.is_null(), "dangling schema reference {reference}");
        return validate(spec, target, value, at);
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(options) = schema[keyword].as_array() {
            let passing = options.iter().filter(|s| validate(spec, s, value, at).is_ok()).count();
            let ok = if keyword == "oneOf" { passing == 1 } else { passing >= 1 };
            return if ok { Ok(()) } else { Err(format!("{at}: {passing} of the {keyword} options match {value}")) };
        }
    }
    if let Some(constant) = schema.get("const") {
        if constant != value {
            return Err(format!("{at}: expected {constant}, got {value}"));
        }
    }
    if let Some(kind) = schema.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::String(k) => vec![k.as_str()],
            Value::Array(ks) => ks.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let actual = match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        let accepted = kinds.contains(&actual) || (actual == "integer" && kinds.contains(&"number"));
        if !accepted {
            return Err(format!("{at}: expected {kinds:?}, got {value}"));
        }
    }
    if let Value::Object(object) = value {
        let properties = schema["properties"].as_object();
        for required in schema["required"].as_array().into_iter().flatten() {
            let key = required.as_str().unwrap();
            if !object.contains_key(key) {
                return Err(format!("{at}: missing required `{key}`"));
            }
        }
        for (key, item) in object {
            match properties.and_then(|p| p.get(key)) {
                Some(property) => validate(spec, property, item, &format!("{at}.{key}"))?,
                None if schema["additionalProperties"] == json!(false) => {
                    return Err(format!("{at}: `{key}` is not allowed"))
                }
                None => {}
            }
        }
    }
    if let Value::Array(items) = value {
        if let Some(prefix) = schema["prefixItems"].as_array() {
            for (i, (s, item)) in prefix.iter().zip(items).enumerate() {
                validate(spec, s, item, &format!("{at}[{i}]"))?;
            }
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                validate(spec, item_schema, item, &format!("{at}[{i}]"))?;
            }
        }
        if let Some(min) = schema["minItems"].as_u64() {
            if (items.len() as u64) < min {
                return Err(format!("{at}: fewer than {min} items"));
            }
        }
        if let Some(max) = schema["maxItems"].as_u64() {
            if items.len() as u64 > max {
                return Err(format!("{at}: more than {max} items"));
            }
        }
    }
    Ok(())
}

const NOTES: &str = r#"type Note:
    title: String
    tag: Option<String>
    rating: Option<Int>

type Line:
    sku: String
    quantity: Int where between(1, 99)

type Order:
    id: Int
    lines: List<Line>
    note: Option<Note>

choice Outcome:
    Saved(shelf: Int)
    Changed(shelf: Int, title: String)

type Receipt:
    order: Order
    outcome: Result<Outcome, String>

server notes:
    route POST "/notes" body Note -> json Note:
        return body

    route POST "/orders" body Order -> json Receipt:
        return Receipt(body, Ok(Changed(body.id, "Dune")))
"#;

#[test]
fn bodies_the_contract_accepts_are_decoded_and_replies_match_the_contract() {
    let app = serve(NOTES);
    let spec = openapi(&app);
    let (note_in, note_out) = route_schemas(&spec, "/notes");
    for body in [
        json!({"title": "a", "tag": "x", "rating": 4}),
        json!({"title": "a", "tag": null, "rating": null}),
        json!({"title": "a"}),
    ] {
        validate(&spec, &note_in, &body, "request").unwrap_or_else(|e| panic!("fixture invalid: {e}"));
        let (status, reply) = post(&app, "/notes", &body);
        assert_eq!(status, 200, "{body} -> {reply}");
        validate(&spec, &note_out, &reply, "reply").unwrap_or_else(|e| panic!("{body} -> {reply}: {e}"));
        assert_eq!(reply["title"], "a");
        assert_eq!(reply["tag"], body.get("tag").cloned().unwrap_or(Value::Null));
    }

    let (order_in, order_out) = route_schemas(&spec, "/orders");
    let order = json!({"id": 7, "lines": [{"sku": "b1", "quantity": 2}], "note": {"title": "gift"}});
    validate(&spec, &order_in, &order, "request").unwrap();
    let (status, reply) = post(&app, "/orders", &order);
    assert_eq!(status, 200, "{reply}");
    validate(&spec, &order_out, &reply, "reply").unwrap_or_else(|e| panic!("{reply}: {e}"));
    assert_eq!(reply["order"]["note"], json!({"title": "gift", "tag": null, "rating": null}));
    assert_eq!(
        reply["outcome"],
        json!({"tag": "ok", "ok": {"tag": "variant", "type": "Outcome", "variant": "Changed", "fields": [7, "Dune"]}})
    );
}

#[test]
fn bodies_the_contract_rejects_get_a_located_400() {
    let app = serve(NOTES);
    let spec = openapi(&app);
    let (note_in, _) = route_schemas(&spec, "/notes");
    let (order_in, _) = route_schemas(&spec, "/orders");
    let cases = [
        ("/notes", &note_in, json!({"title": "a", "extra": 1}), "extra"),
        ("/notes", &note_in, json!({"title": "a", "tag": {"tag": "some", "value": "x"}}), "tag"),
        ("/notes", &note_in, json!({"tag": "x"}), "title"),
        ("/notes", &note_in, json!({"title": "a", "rating": 4.5}), "rating"),
        ("/orders", &order_in, json!({"id": 7, "lines": [{"sku": "b1", "quantity": "2"}]}), "lines[0].quantity"),
        ("/orders", &order_in, json!({"id": 7, "lines": [], "note": {"title": 3}}), "note.title"),
    ];
    for (path, schema, body, located) in cases {
        assert!(validate(&spec, schema, &body, "request").is_err(), "the contract should reject {body}");
        let (status, reply) = post(&app, path, &body);
        assert_eq!(status, 400, "{body} -> {reply}");
        assert_eq!(reply["error"], "invalid_body", "{reply}");
        assert_eq!(reply["path"], located, "{body} -> {reply}");
        assert!(reply["detail"].as_str().unwrap().contains(located), "{reply}");
    }
}

#[test]
fn a_refinement_violation_is_located_at_its_field() {
    let app = serve(NOTES);
    let body = json!({"id": 7, "lines": [{"sku": "b1", "quantity": 100}]});
    let (status, reply) = post(&app, "/orders", &body);
    assert_eq!(status, 400, "{reply}");
    assert_eq!(reply["path"], "lines[0].quantity");
    assert_eq!(reply["expected"], "a value between(1, 99)");
    assert_eq!(reply["found"], "100");
}

#[test]
fn a_single_file_service_publishes_schemas_for_its_private_route_types() {
    let app = serve(NOTES);
    let spec = openapi(&app);
    for name in ["Note", "Order", "Line", "Receipt", "Outcome"] {
        assert!(spec["components"]["schemas"][name].is_object(), "missing schema {name}: {spec}");
    }
    assert_eq!(spec["components"]["schemas"]["Note"]["properties"]["tag"]["type"], json!(["string", "null"]));
}
