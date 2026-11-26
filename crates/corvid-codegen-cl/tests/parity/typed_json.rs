//! `T.from_json(text)` and `T.from_json_strict(text)` on the native
//! tier. Every program runs on the interpreter and as a compiled
//! binary; both must produce the same result, and the binary must
//! release every allocation, including the values a failed decode had
//! already built.

use super::{assert_no_leaks, assert_parity, entry_name, ir_of, run_with_leak_detector, test_tools_lib_path};
use corvid_codegen_cl::build_native_to_disk;
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::sync::Arc;

const TYPES: &str = r#"
type Line:
    sku: String
    quantity: Int where between(1, 99)

type Order:
    id: Int
    note: Option<String>
    lines: List<Line>
    rating: Option<Int>
    weight: Float
    gift: Bool
    tags: Map<String, Int>

type Shipment:
    | Pending
    | Shipped(carrier: String, days: Int)

type Lines = List<Line>

type Outcome = Result<Int, String>
"#;

/// Run `body` (after the shared types) on both tiers; the entry agent
/// returns a String. Returns the text both produced.
#[track_caller]
fn assert_parity_text(body: &str) -> String {
    let src = format!("{TYPES}\n{body}");
    let ir = ir_of(&src);
    let runtime = Runtime::builder().approver(Arc::new(ProgrammaticApprover::always_yes())).build();
    let interpreted = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(&ir, entry_name(&ir), vec![], &runtime).await })
        .expect("interpreter run");
    let Value::String(interpreted) = interpreted else {
        panic!("interpreter returned {interpreted:?}, not a String; src=\n{src}")
    };

    let tmp = tempfile::tempdir().expect("tempdir");
    let produced = build_native_to_disk(&ir, "corvid_parity_test", &tmp.path().join("prog"), &[test_tools_lib_path().as_path()])
        .expect("compile + link");
    let (stdout, stderr, status) = run_with_leak_detector(&produced);
    assert!(status.success(), "compiled binary failed: {status:?} stderr={stderr} src=\n{src}");
    let compiled = stdout.trim_end_matches(['\r', '\n']).to_string();
    assert_eq!(compiled, interpreted.as_ref(), "the tiers disagree; src=\n{src}");
    assert_no_leaks(&stderr, &src);
    compiled
}

const ORDER_JSON: &str = r#"{\"id\": 7, \"note\": null, \"lines\": [{\"sku\": \"a\", \"quantity\": 2}, {\"sku\": \"café\", \"quantity\": 3}], \"rating\": 4, \"weight\": 2.5, \"gift\": true, \"tags\": {\"x\": 1, \"y\": 2}}"#;

#[test]
fn every_field_kind_decodes_to_the_same_values_on_both_tiers() {
    let body = format!(
        r#"
agent gift_bonus(gift: Bool) -> Int:
    if gift:
        return 100000
    return 0

agent weight_bonus(weight: Float) -> Int:
    if weight > 1.5:
        return 1000000
    return 0

agent rating_of(order: Order) -> Int:
    return match order.rating:
        Some(r) -> r
        None -> 0

agent score(order: Order) -> Int:
    base = order.id * 1000 + order.lines[1].quantity * 100 + rating_of(order) * 10 + order.lines.length()
    return base + order.tags.length() * 10000 + gift_bonus(order.gift) + weight_bonus(order.weight) + order.lines[1].sku.length() * 10000000

agent main() -> Int:
    return match Order.from_json("{ORDER_JSON}"):
        Ok(order) -> score(order)
        Err(problem) -> 0 - 1
"#
    );
    assert_parity(&format!("{TYPES}\n{body}"), 41_127_342);
}

#[test]
fn decode_failures_report_the_same_path_and_message_on_both_tiers() {
    let text = assert_parity_text(
        r#"
agent message_of(text: String) -> String:
    return match Order.from_json(text):
        Ok(order) -> "decoded"
        Err(problem) -> problem.message

agent main() -> String:
    wrong_kind = message_of("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": \"two\"}], \"weight\": 1.0, \"gift\": false, \"tags\": {}}")
    refined = message_of("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": 2}, {\"sku\": \"b\", \"quantity\": 100}], \"weight\": 1.0, \"gift\": false, \"tags\": {}}")
    missing = message_of("{\"lines\": [], \"weight\": 1.0, \"gift\": false, \"tags\": {}}")
    late = message_of("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": 2}], \"weight\": 1.0, \"gift\": false, \"tags\": {\"x\": 1, \"y\": \"two\"}}")
    broken = message_of("{\"id\": ")
    return wrong_kind + " | " + refined + " | " + missing + " | " + late + " | " + broken
"#,
    );
    let messages: Vec<&str> = text.split(" | ").collect();
    assert_eq!(messages[0], "at `lines[0].quantity`: expected Int, found a string");
    assert_eq!(messages[1], "at `lines[1].quantity`: expected a value between(1, 99), found 100");
    assert_eq!(messages[2], "at `id`: expected Int, found no value (the field is missing)");
    assert_eq!(messages[3], "at `tags.y`: expected Int, found a string");
    assert!(messages[4].starts_with("expected JSON text, found text that ends early"), "{}", messages[4]);
}

#[test]
fn every_json_error_field_is_readable_natively() {
    let text = assert_parity_text(
        r#"
agent main() -> String:
    return match Line.from_json("{\"sku\": 5, \"quantity\": 2}"):
        Ok(line) -> line.sku
        Err(problem) -> problem.path + " / " + problem.expected + " / " + problem.found
"#,
    );
    assert_eq!(text, "sku / String / a number");
}

/// `JsonError` written in a signature: the error passes to a helper
/// agent and is read there, on both tiers, and released once.
#[test]
fn a_json_error_passes_to_a_helper_agent_that_names_its_type() {
    let text = assert_parity_text(
        r#"
agent explain(problem: JsonError) -> String:
    return problem.path + ": " + problem.message

agent main() -> String:
    return match Line.from_json("{\"sku\": 5, \"quantity\": 2}"):
        Ok(line) -> line.sku
        Err(problem) -> explain(problem)
"#,
    );
    assert!(text.starts_with("sku: "), "{text}");
}

#[test]
fn strict_decoding_refuses_undeclared_keys_on_both_tiers() {
    let text = assert_parity_text(
        r#"
agent main() -> String:
    lenient = match Line.from_json("{\"sku\": \"a\", \"quantity\": 2, \"extra\": true}"):
        Ok(line) -> line.sku
        Err(problem) -> problem.message
    strict = match Line.from_json_strict("{\"sku\": \"a\", \"quantity\": 2, \"extra\": true}"):
        Ok(line) -> line.sku
        Err(problem) -> problem.message
    return lenient + " | " + strict
"#,
    );
    assert_eq!(text, "a | at `extra`: expected no field (`Line` declares no `extra`), found a value");
}

#[test]
fn sum_types_decode_to_the_same_variant_on_both_tiers() {
    assert_parity(
        &format!(
            r#"{TYPES}
agent days_of(shipment: Shipment) -> Int:
    return match shipment:
        Pending -> 0
        Shipped(carrier, days) -> days + carrier.length() * 100

agent decode_days(text: String) -> Int:
    return match Shipment.from_json(text):
        Ok(shipment) -> days_of(shipment)
        Err(problem) -> 0 - 1

agent main() -> Int:
    return decode_days("{{\"variant\": \"Shipped\", \"fields\": [\"post\", 3]}}") * 10 + decode_days("{{\"variant\": \"Pending\"}}") + decode_days("{{\"variant\": \"Lost\"}}") * 1000
"#
        ),
        3_030,
    );
}

#[test]
fn a_decoded_record_encodes_to_the_same_text_on_both_tiers() {
    let text = assert_parity_text(&format!(
        r#"
agent main() -> String:
    return match Order.from_json("{ORDER_JSON}"):
        Ok(order) -> order.to_json()
        Err(problem) -> problem.message
"#
    ));
    assert_eq!(
        text,
        r#"{"gift":true,"id":7,"lines":[{"quantity":2,"sku":"a"},{"quantity":3,"sku":"café"}],"note":null,"rating":4,"tags":{"x":1,"y":2},"weight":2.5}"#
    );
}

#[test]
fn scalars_lists_sums_options_and_results_encode_alike() {
    let text = assert_parity_text(
        r#"
agent shipped() -> Shipment:
    return Shipped("post", 3)

agent maybe(flag: Bool) -> Option<Int>:
    if flag:
        return Some(7)
    return None

agent outcome(flag: Bool) -> Outcome:
    if flag:
        return Ok(40)
    return Err("late")

agent main() -> String:
    numbers = [1, 2, 3]
    count = 7
    quoted = "a\"b"
    flag = true
    weight = 2.5
    return count.to_json() + " " + quoted.to_json() + " " + flag.to_json() + " " + weight.to_json() + " " + numbers.to_json() + " " + shipped().to_json() + " " + maybe(true).to_json() + " " + maybe(false).to_json() + " " + outcome(true).to_json() + " " + outcome(false).to_json()
"#,
    );
    assert_eq!(
        text,
        r#"7 "a\"b" true 2.5 [1,2,3] {"fields":["post",3],"tag":"variant","type":"Shipment","variant":"Shipped"} 7 null {"ok":40,"tag":"ok"} {"err":"late","tag":"err"}"#
    );
}

#[test]
fn lists_and_results_decode_at_the_top_level() {
    assert_parity(
        &format!(
            r#"{TYPES}
agent total(lines: List<Line>) -> Int:
    sum = 0
    for line in lines:
        sum += line.quantity
    return sum

agent main() -> Int:
    lines = match Lines.from_json("[{{\"sku\": \"a\", \"quantity\": 2}}, {{\"sku\": \"b\", \"quantity\": 5}}]"):
        Ok(lines) -> total(lines)
        Err(problem) -> 0 - 1
    outcome = match Outcome.from_json("{{\"tag\": \"ok\", \"ok\": 40}}"):
        Ok(inner) -> match_inner(inner)
        Err(problem) -> 0 - 1
    return lines * 100 + outcome

agent match_inner(inner: Result<Int, String>) -> Int:
    return match inner:
        Ok(n) -> n
        Err(reason) -> reason.length()
"#
        ),
        740,
    );
}

/// B01: a record with `@json(case:)` and a field's own `@json(key:)`
/// reads and writes the same keys on both tiers, and a failure names the
/// key the sender wrote.
#[test]
fn json_keys_match_on_both_tiers() {
    let out = assert_parity_text(
        r#"
@json(case: camel)
type Ticket:
    ticket_id: Int
    @json(key: "ID")
    legacy_id: String
    due_date: Option<String>

agent main() -> String:
    read = match Ticket.from_json("{\"ticketId\": 7, \"ID\": \"T-7\", \"dueDate\": null}"):
        Ok(t) -> t.to_json() + " " + t.legacy_id
        Err(e) -> "failed at " + e.path
    wrong = match Ticket.from_json("{\"ticketId\": \"seven\", \"ID\": \"T\"}"):
        Ok(t) -> "decoded"
        Err(e) -> e.path
    strict = match Ticket.from_json_strict("{\"ticket_id\": 7, \"ticketId\": 7, \"ID\": \"T\"}"):
        Ok(t) -> "decoded"
        Err(e) -> e.path
    return read + " | " + wrong + " | " + strict
"#,
    );
    assert_eq!(out, r#"{"ID":"T-7","dueDate":null,"ticketId":7} T-7 | ticketId | ticket_id"#);
}
