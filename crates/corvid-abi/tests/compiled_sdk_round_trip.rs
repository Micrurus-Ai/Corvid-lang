//! The generated Kotlin and Swift SDKs compile and read and write the
//! public JSON mapping.
//!
//! Each test compiles the generated models with a small program that
//! decodes JSON in the wire shapes Corvid's encoder produces, encodes it
//! back, and requires the same JSON; a malformed body must fail. The
//! toolchains are found through `CORVID_KOTLINC` with
//! `CORVID_KOTLIN_CLASSPATH` (the kotlinx-serialization core and json
//! jars) and `CORVID_SWIFTC`, or on `PATH`. A test without its toolchain
//! reports that it skipped.

use corvid_abi::app_contract::{effect_decls_of, emit_application_contract, ContractOptions};
use corvid_abi::sdk_gen::{emit_sdk, SdkLanguage};
use corvid_types::effects::EffectRegistry;
use std::path::{Path, PathBuf};
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
    conflict: Conflict<Line>
    gone: Conflict<Line>

public agent echo(order: Order) -> Order:
    return order
"#;

/// The wire JSON the programs decode and must encode back.
const ORDER: &str = r#"{"id": 7, "lines": [{"sku": "a", "quantity": 2, "note": null}, {"sku": "b", "quantity": 3, "note": "gift"}], "tags": {"x": 1}, "codes": [[2, "two"]], "shipment": {"tag": "variant", "type": "Shipment", "variant": "Shipped", "fields": ["post", 3]}, "outcome": {"tag": "ok", "ok": 40}, "weight": 2.5, "gift": true, "conflict": {"tag": "variant", "type": "Conflict", "variant": "Taken", "fields": [{"sku": "c", "quantity": 1, "note": null}]}, "gone": {"tag": "variant", "type": "Conflict", "variant": "Deleted", "fields": []}}"#;
const PENDING: &str = r#"{"tag": "variant", "type": "Shipment", "variant": "Pending", "fields": []}"#;
const BROKEN: &str = r#"{"id": 7, "lines": [{"sku": "a", "quantity": "two"}], "tags": {}, "codes": [], "shipment": {"variant": "Pending"}, "outcome": {"ok": 1}, "weight": 1.0, "gift": false, "conflict": {"tag": "variant", "type": "Conflict", "variant": "Taken", "fields": [{"sku": "c", "quantity": 1, "note": null}]}, "gone": {"tag": "variant", "type": "Conflict", "variant": "Deleted", "fields": []}}"#;

fn models(language: SdkLanguage) -> String {
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
    emit_sdk(&contract, language).remove(0).contents
}

/// A tool from its environment variable, else `name` if it runs.
fn tool(variable: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(variable) {
        return Some(PathBuf::from(path));
    }
    let probe = Command::new(name).arg("-version").output().ok()?;
    probe.status.success().then(|| PathBuf::from(name))
}

#[track_caller]
fn run(command: &mut Command, what: &str) -> String {
    let output = command.output().unwrap_or_else(|e| panic!("{what}: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{what} failed:\nstdout={stdout}\nstderr={}", String::from_utf8_lossy(&output.stderr));
    stdout
}

fn write(dir: &Path, name: &str, contents: &str) {
    std::fs::write(dir.join(name), contents).expect("write source");
}

#[test]
fn kotlin_models_compile_and_round_trip_the_public_json_mapping() {
    let (Some(kotlinc), Some(classpath)) = (tool("CORVID_KOTLINC", "kotlinc"), std::env::var_os("CORVID_KOTLIN_CLASSPATH")) else {
        eprintln!("skipping: set CORVID_KOTLINC and CORVID_KOTLIN_CLASSPATH to compile the Kotlin SDK");
        return;
    };
    let classpath = classpath.to_string_lossy().into_owned();
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "Models.kt", &models(SdkLanguage::Kotlin));
    write(
        dir.path(),
        "Check.kt",
        &format!(
            r####"import kotlinx.serialization.json.Json

fun main() {{
    val order = Order.fromJson("""{ORDER}""")
    val shipment = order.shipment
    check(shipment is Shipment.Shipped && shipment.days == 3L) {{ "shipment: $shipment" }}
    val conflict = order.conflict
    check(conflict is CorvidConflict.Taken && conflict.holder.sku == "c") {{ "conflict: $conflict" }}
    check(order.gone == CorvidConflict.Deleted) {{ "gone: ${{order.gone}}" }}
    check(order.codes[2L] == "two" && order.outcome is CorvidResult.Ok) {{ "order: $order" }}
    check(Json.parseToJsonElement(order.toJson()) == Json.parseToJsonElement("""{ORDER}""")) {{ "round trip: ${{order.toJson()}}" }}
    check(Json.parseToJsonElement(Shipment.fromJson("""{{"variant": "Pending"}}""").toJson()) == Json.parseToJsonElement("""{PENDING}"""))
    val failed = runCatching {{ Order.fromJson("""{BROKEN}""") }}
    check(failed.isFailure) {{ "a wrong kind must fail" }}
    println("ok")
}}
"####
        ),
    );
    let jar = dir.path().join("check.jar");
    run(
        Command::new(&kotlinc)
            .args(["-cp", &classpath, "-include-runtime", "-d"])
            .arg(&jar)
            .args(["Models.kt", "Check.kt"])
            .current_dir(dir.path()),
        "kotlinc",
    );
    let separator = if cfg!(windows) { ";" } else { ":" };
    let stdout = run(
        Command::new(std::env::var_os("CORVID_JAVA").unwrap_or_else(|| "java".into()))
            .arg("-cp")
            .arg(format!("{}{separator}{classpath}", jar.display()))
            .arg("CheckKt")
            .current_dir(dir.path()),
        "java",
    );
    assert_eq!(stdout.trim(), "ok");
}

#[test]
fn swift_models_compile_and_round_trip_the_public_json_mapping() {
    let Some(swiftc) = tool("CORVID_SWIFTC", "swiftc") else {
        eprintln!("skipping: set CORVID_SWIFTC to compile the Swift SDK");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "Models.swift", &models(SdkLanguage::Swift));
    write(
        dir.path(),
        "main.swift",
        &format!(
            r####"import Foundation

func json(_ text: String) -> Data {{ text.data(using: .utf8)! }}
func same(_ a: Data, _ b: Data) throws -> Bool {{
    let left = try JSONSerialization.jsonObject(with: a) as! NSDictionary
    let right = try JSONSerialization.jsonObject(with: b) as! NSDictionary
    return left.isEqual(to: right as! [AnyHashable: Any])
}}

let order = try JSONDecoder().decode(Order.self, from: json(#"{ORDER}"#))
guard case let .shipped(_, days) = order.shipment, days == 3 else {{ fatalError("shipment: \(order.shipment)") }}
guard case let .taken(holder) = order.conflict, holder.sku == "c" else {{ fatalError("conflict: \(order.conflict)") }}
guard case .deleted = order.gone else {{ fatalError("gone: \(order.gone)") }}
guard order.codes.value[2] == "two", case .ok(40) = order.outcome else {{ fatalError("order") }}
let encoded = try JSONEncoder().encode(order)
guard try same(encoded, json(#"{ORDER}"#)) else {{ fatalError("round trip: \(String(data: encoded, encoding: .utf8)!)") }}
let pending = try JSONEncoder().encode(try JSONDecoder().decode(Shipment.self, from: json(#"{{"variant": "Pending"}}"#)))
guard try same(pending, json(#"{PENDING}"#)) else {{ fatalError("pending: \(String(data: pending, encoding: .utf8)!)") }}
if (try? JSONDecoder().decode(Order.self, from: json(#"{BROKEN}"#))) != nil {{ fatalError("a wrong kind must fail") }}
print("ok")
"####
        ),
    );
    let binary = dir.path().join("check");
    run(
        Command::new(&swiftc).args(["Models.swift", "main.swift", "-o"]).arg(&binary).current_dir(dir.path()),
        "swiftc",
    );
    let stdout = run(&mut Command::new(&binary), "swift program");
    assert_eq!(stdout.trim(), "ok");
}
