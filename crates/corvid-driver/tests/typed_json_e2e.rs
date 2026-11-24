//! B01 - `T.from_json(text)`, `T.from_json_strict(text)` and
//! `value.to_json()` through the public JSON mapping, compiled and run
//! as real programs. The mapping itself is unit-tested in
//! `corvid_vm::public_json`; these pin the language surface: typing,
//! compile-time refusals, taint, and the interpreter's results.

use corvid_driver::{compile_to_ir, run_ir_with_runtime, Diagnostic};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const TYPES: &str = "
type Line:
    sku: String
    quantity: Int where between(1, 99)

type Order:
    id: Int
    note: Option<String>
    lines: List<Line>

type Orders = List<Order>

type Shipment:
    | Pending
    | Shipped(carrier: String, days: Int)
";

async fn run(body: &str) -> Value {
    let source = format!("{TYPES}\n{body}");
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("source must compile: {errors:#?}"));
    run_ir_with_runtime(&ir, None, vec![], &Runtime::builder().build())
        .await
        .expect("program must run")
}

fn run_text(value: Value) -> String {
    match value {
        Value::String(text) => text.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

fn compile_errors(body: &str) -> Vec<Diagnostic> {
    let source = format!("{TYPES}\n{body}");
    compile_to_ir(&source).expect_err("source must be refused")
}

fn assert_refused(body: &str, needle: &str) {
    let errors = compile_errors(body);
    assert!(
        errors.iter().any(|e| e.message.contains(needle)),
        "expected a diagnostic containing {needle:?}, got {errors:#?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decoding_then_encoding_follows_the_public_mapping() {
    let out = run(r#"
agent main() -> String:
    return match Order.from_json("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": 2}], \"source\": \"web\"}"):
        Ok(order) -> order.to_json()
        Err(problem) -> problem.message
"#)
    .await;
    // The undeclared `source` key is skipped, the absent `note` is
    // `null`, and keys are written in sorted order.
    assert_eq!(run_text(out), r#"{"id":7,"lines":[{"quantity":2,"sku":"a"}],"note":null}"#);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_decode_is_a_located_json_error() {
    let out = run(r#"
agent describe(text: String) -> String:
    return match Order.from_json(text):
        Ok(order) -> "decoded"
        Err(problem) -> problem.path + " | " + problem.expected + " | " + problem.found

agent main() -> String:
    kind = describe("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": \"2\"}]}")
    range = describe("{\"id\": 7, \"lines\": [{\"sku\": \"a\", \"quantity\": 100}]}")
    missing = describe("{\"lines\": []}")
    text = describe("{\"id\": 7,")
    return kind + "\n" + range + "\n" + missing + "\n" + text
"#)
    .await;
    assert_eq!(
        run_text(out),
        "lines[0].quantity | Int | a string\n\
         lines[0].quantity | a value between(1, 99) | 100\n\
         id | Int | no value (the field is missing)\n \
         | JSON text | text that ends early, at line 1, column 9"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_decoding_rejects_undeclared_keys() {
    let out = run(r#"
agent main() -> String:
    text = "{\"id\": 7, \"lines\": [], \"source\": \"web\"}"
    lenient = match Order.from_json(text):
        Ok(order) -> "ok"
        Err(problem) -> problem.message
    strict = match Order.from_json_strict(text):
        Ok(order) -> "ok"
        Err(problem) -> problem.message
    return lenient + " | " + strict
"#)
    .await;
    assert_eq!(
        run_text(out),
        "ok | at `source`: expected no field (`Order` declares no `source`), found a value"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aliases_name_lists_maps_and_other_composite_targets() {
    let out = run(r#"
type Stock = Map<String, Int>

agent main() -> String:
    orders = match Orders.from_json("[{\"id\": 1, \"lines\": []}, {\"id\": 2, \"note\": \"gift\", \"lines\": []}]"):
        Ok(found) -> found.length()
        Err(problem) -> -1
    stock = match Stock.from_json("{\"apples\": 3, \"pears\": 5}"):
        Ok(found) -> found.get("pears").unwrap_or(0)
        Err(problem) -> -1
    return orders.to_string() + " " + stock.to_string()
"#)
    .await;
    assert_eq!(run_text(out), "2 5");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sum_types_round_trip_through_the_variant_envelope() {
    let out = run(r#"
agent main() -> String:
    return match Shipment.from_json("{\"variant\": \"Shipped\", \"fields\": [\"post\", 3]}"):
        Ok(shipment) -> shipment.to_json()
        Err(problem) -> problem.message
"#)
    .await;
    assert_eq!(
        run_text(out),
        r#"{"fields":["post",3],"tag":"variant","type":"Shipment","variant":"Shipped"}"#
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scalars_and_options_encode_as_plain_json() {
    let out = run(r#"
agent no_note() -> Option<String>:
    return None

agent main() -> String:
    absent = no_note()
    count = 42
    ratio = 2.5
    return "hÃ©llo".to_json() + " " + count.to_json() + " " + ratio.to_json() + " " + absent.to_json() + " " + [1, 2].to_json()
"#)
    .await;
    assert_eq!(run_text(out), r#""hÃ©llo" 42 2.5 null [1,2]"#);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_finite_float_traps_when_encoded() {
    let source = format!(
        "{TYPES}\nagent main() -> String:\n    zero = 0.0\n    return (zero / zero).to_json()\n"
    );
    let ir = compile_to_ir(&source).expect("source must compile");
    let result = run_ir_with_runtime(&ir, None, vec![], &Runtime::builder().build()).await;
    match result {
        Err(err) => {
            let text = format!("{err:?}");
            // Checked Float division may trap first; either way the
            // program must not produce JSON text.
            assert!(text.contains("to_json") || text.contains("NaN") || text.contains("division"), "{text}");
        }
        Ok(value) => panic!("a NaN must not encode, got {value:?}"),
    }
}

#[test]
fn from_json_is_typed_as_a_result_with_json_error() {
    assert_refused(
        "agent main() -> Order:\n    return Order.from_json(\"{}\")\n",
        "Result<",
    );
    assert_refused(
        "agent main() -> Int:\n    return match Order.from_json(\"{}\"):\n        Ok(order) -> order.id\n        Err(problem) -> problem.line\n",
        "no field named `line` on type `JsonError`",
    );
    assert_refused("agent main(n: Int) -> String:\n    return match Order.from_json(n):\n        Ok(order) -> \"\"\n        Err(problem) -> \"\"\n", "String");
}

#[test]
fn types_without_a_json_form_are_refused_at_compile_time() {
    assert_refused(
        "type Cited:\n    claim: Grounded<String>\n\nagent main() -> String:\n    return match Cited.from_json(\"{}\"):\n        Ok(c) -> \"\"\n        Err(problem) -> problem.message\n",
        "field `Cited.claim`: JSON text cannot prove where a `Grounded` value came from",
    );
    assert_refused(
        "type Hook:\n    run: (Int) -> Int\n\nagent main(hook: Hook) -> String:\n    return hook.to_json()\n",
        "`to_json` is not available for `Hook`: field `Hook.run`: a function has no JSON form",
    );
}

#[test]
fn encoding_tainted_data_keeps_the_taint() {
    let errors = compile_errors(
        "effect web_content:
    data: untrusted

tool fetch_page(url: String) -> String uses web_content

tool run_shell(cmd: String) -> String dangerous

type Page:
    body: Tainted<String>

agent direct(url: String) -> String:
    return run_shell(fetch_page(url).to_json())

agent nested(page: Page) -> String:
    return run_shell(page.to_json())
",
    );
    let tainted = errors.iter().filter(|e| e.message.to_lowercase().contains("taint")).count();
    assert!(tainted >= 2, "both encodings must stay tainted at the dangerous sink: {errors:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deterministic_code_may_decode_and_encode() {
    let out = run(r#"
@deterministic
agent normalize(text: String) -> String:
    return match Order.from_json(text):
        Ok(order) -> order.to_json()
        Err(problem) -> problem.message

agent main() -> String:
    return normalize("{\"lines\": [], \"id\": 3}")
"#)
    .await;
    assert_eq!(run_text(out), r#"{"id":3,"lines":[],"note":null}"#);
}

#[test]
fn extend_methods_cannot_reuse_built_in_json_names() {
    assert_refused(
        "extend Order:\n    public agent to_json(o: Order) -> String:\n        return \"x\"\n",
        "would be hidden by the built-in `to_json`",
    );
}

#[test]
fn every_reserved_method_name_is_a_built_in() {
    let order = corvid_types::Type::Struct(corvid_resolve::DefId(0));
    for name in corvid_resolve::BUILTIN_METHODS_ON_EVERY_TYPE {
        assert!(
            corvid_types::builtin_method(&order, name).is_some() || corvid_types::type_method(&order, name).is_some(),
            "`{name}` is reserved for extend methods but the method table does not offer it"
        );
    }
}

// ----- Types declared in another module ----------------------------

const MODELS: &str = "
public type Address:
    city: String

public type Customer:
    id: Int
    address: Address
    referrer: Option<Customer>

public type Customers = List<Customer>
";

/// Compile `main` next to a `models.cor` holding [`MODELS`].
fn compile_with_models(main: &str) -> Result<corvid_ir::IrFile, Vec<Diagnostic>> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("models.cor"), MODELS).unwrap();
    let main_path = dir.path().join("main.cor");
    std::fs::write(&main_path, main).unwrap();
    corvid_driver::compile_to_ir_with_config_at_path(main, &main_path, None)
}

async fn run_with_models(main: &str) -> String {
    let ir = compile_with_models(main).unwrap_or_else(|errors| panic!("source must compile: {errors:#?}"));
    let out = run_ir_with_runtime(&ir, None, vec![], &Runtime::builder().build())
        .await
        .expect("program must run");
    run_text(out)
}

const CUSTOMER_JSON: &str = r#"{\"id\": 4, \"address\": {\"city\": \"Kampala\"}, \"referrer\": {\"id\": 1, \"address\": {\"city\": \"Gulu\"}, \"referrer\": null}}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_type_imported_by_name_decodes_and_encodes() {
    let out = run_with_models(&format!(
        r#"import "./models" use Customer

agent main() -> String:
    return match Customer.from_json("{CUSTOMER_JSON}"):
        Ok(customer) -> customer.address.city + " " + customer.to_json()
        Err(problem) -> problem.message
"#
    ))
    .await;
    assert_eq!(
        out,
        r#"Kampala {"address":{"city":"Kampala"},"id":4,"referrer":{"address":{"city":"Gulu"},"id":1,"referrer":null}}"#
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_type_reached_through_a_module_alias_decodes() {
    let out = run_with_models(&format!(
        r#"import "./models" as m

agent referrer_city(customer: m.Customer) -> String:
    return match customer.referrer:
        Some(referrer) -> referrer.address.city
        None -> "no referrer"

agent main() -> String:
    return match m.Customer.from_json("{CUSTOMER_JSON}"):
        Ok(customer) -> referrer_city(customer)
        Err(problem) -> problem.message
"#
    ))
    .await;
    assert_eq!(out, "Gulu");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_alias_over_an_imported_type_decodes_a_list_with_located_errors() {
    let out = run_with_models(
        r#"import "./models" as m

type Customers = List<m.Customer>

agent main() -> String:
    return match Customers.from_json("[{\"id\": 1, \"address\": {\"city\": 5}}]"):
        Ok(customers) -> "decoded"
        Err(problem) -> problem.path + " | " + problem.expected
"#,
    )
    .await;
    assert_eq!(out, "[0].address.city | String");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alias_declared_in_another_module_names_a_list_target() {
    let out = run_with_models(
        r#"import "./models" use Customers

agent main() -> String:
    return match Customers.from_json("[{\"id\": 1, \"address\": {\"city\": \"Mbale\"}}, {\"id\": 2, \"address\": {\"city\": \"Jinja\"}}]"):
        Ok(customers) -> customers[1].address.city + " of " + customers.length().to_string()
        Err(problem) -> problem.message
"#,
    )
    .await;
    assert_eq!(out, "Jinja of 2");
}

#[test]
fn an_imported_type_without_a_json_form_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("models.cor"),
        "public type Cited:\n    claim: Grounded<String>\n",
    )
    .unwrap();
    let main = "import \"./models\" use Cited\n\nagent main(text: String) -> Bool:\n    return Cited.from_json(text).is_ok()\n";
    let main_path = dir.path().join("main.cor");
    std::fs::write(&main_path, main).unwrap();
    let errors = corvid_driver::compile_to_ir_with_config_at_path(main, &main_path, None)
        .expect_err("decoding Grounded data from an imported type must be refused");
    assert!(
        errors.iter().any(|e| e.message.contains("field `Cited.claim`") && e.message.contains("Grounded")),
        "expected the refusal to name the imported field, got {errors:#?}"
    );
}
