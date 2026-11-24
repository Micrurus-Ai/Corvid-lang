//! The generated Python SDK reads and writes the public JSON mapping.
//!
//! Python decodes JSON in the wire shapes Corvid's encoder produces into
//! the generated models and encodes it back; the result must be the
//! same JSON. A malformed body must fail with a located error.

use corvid_abi::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
use corvid_abi::sdk_gen::{emit_sdk, SdkLanguage};
use corvid_types::effects::EffectRegistry;
use std::process::Command;

const SRC: &str = r#"
public type Line:
    sku: String
    quantity: Int
    note: Option<String>

public type Shipment:
    | Pending
    | Shipped(carrier: String, days: Int)

public type Order:
    id: Int
    lines: List<Line>
    tags: Map<String, Int>
    codes: Map<Int, String>
    shipment: Shipment
    outcome: Result<Int, String>
    weight: Float
    gift: Bool

public agent echo(order: Order) -> Order:
    return order
"#;

fn python() -> Option<Vec<String>> {
    let candidates: Vec<Vec<&str>> =
        if cfg!(windows) { vec![vec!["py", "-3"], vec!["python"]] } else { vec![vec!["python3"], vec!["python"]] };
    candidates.into_iter().map(|c| c.into_iter().map(String::from).collect::<Vec<_>>()).find(|c| {
        Command::new(&c[0]).args(&c[1..]).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
    })
}

fn models() -> String {
    let tokens = corvid_syntax::lex(SRC).expect("lex");
    let (file, errors) = corvid_syntax::parse_file(&tokens);
    assert!(errors.is_empty(), "parse: {errors:?}");
    let resolved = corvid_resolve::resolve(&file);
    let checked = corvid_types::typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
    let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
    let contract = emit_application_contract(
        &file,
        &resolved,
        &checked,
        &registry,
        &ContractOptions { source_path: "app.cor", compiler_version: "test", generated_at: "now" },
    );
    emit_sdk(&contract, SdkLanguage::Python).remove(0).contents
}

const SCRIPT: &str = r#"
import json
import models

ORDER = {
    "id": 7,
    "lines": [{"sku": "a", "quantity": 2, "note": None}, {"sku": "b", "quantity": 3, "note": "gift"}],
    "tags": {"x": 1},
    "codes": [[1, "one"], [2, "two"]],
    "shipment": {"tag": "variant", "type": "Shipment", "variant": "Shipped", "fields": ["post", 3]},
    "outcome": {"tag": "ok", "ok": 40},
    "weight": 2.5,
    "gift": True,
}
order = models.Order.from_json(ORDER)
assert isinstance(order.shipment, models.Shipment.Shipped), order.shipment
assert order.shipment.days == 3 and order.codes[2] == "two" and isinstance(order.outcome, models.Ok)
assert order.to_json() == ORDER, json.dumps(order.to_json())

pending = models.Shipment.from_json({"variant": "Pending"})
assert pending.to_json() == {"tag": "variant", "type": "Shipment", "variant": "Pending", "fields": []}
assert models.to_json({}, "Map<Int, String>") == []
assert models.from_json({"tag": "err", "err": "late"}, "Result<Int, String>") == models.Err("late")

error = models.JsonError.from_json({"path": "lines[0].quantity", "expected": "Int", "found": "a string", "message": "m"})
assert error.path == "lines[0].quantity"

try:
    models.Order.from_json({**ORDER, "lines": [{"sku": "a", "quantity": "two"}]})
except models.WireError as failure:
    assert str(failure) == "at `lines[0].quantity`: expected Int, found a string", str(failure)
else:
    raise AssertionError("a wrong kind must fail")
print("ok")
"#;

#[test]
fn python_models_round_trip_the_public_json_mapping() {
    let Some(python) = python() else {
        eprintln!("skipping: no Python interpreter on PATH");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("models.py"), models()).expect("write models");
    std::fs::write(dir.path().join("check.py"), SCRIPT).expect("write script");
    let output = Command::new(&python[0])
        .args(&python[1..])
        .arg("check.py")
        .current_dir(dir.path())
        .output()
        .expect("run python");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && stdout.trim() == "ok", "stdout={stdout}\nstderr={stderr}");
}
