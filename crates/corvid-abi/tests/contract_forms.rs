//! B06 — every published projection carries the exact-text types the
//! same way: OpenAPI with the shared schema, the TypeScript client and
//! the Swift, Kotlin and Python SDKs as strings holding the exact text,
//! and the generated Python round-trips that text unchanged.

use corvid_abi::app_contract::{effect_decls_of, emit_application_contract, ApplicationContract, ContractOptions};
use corvid_abi::openapi::emit_openapi;
use corvid_abi::sdk_gen::{emit_sdk, SdkLanguage};
use corvid_abi::ts_client::emit_ts_client;
use corvid_types::effects::EffectRegistry;
use std::process::Command;

const SRC: &str = r#"
public type Invoice:
    total: Decimal
    issued: Instant
    terms: Duration
    due: ZonedTime
    office: TimeZone
    currency: Currency
    locale: Locale

public agent echo(invoice: Invoice) -> Invoice:
    return invoice
"#;

fn contract() -> ApplicationContract {
    let tokens = corvid_syntax::lex(SRC).expect("lex");
    let (file, errors) = corvid_syntax::parse_file(&tokens);
    assert!(errors.is_empty(), "parse: {errors:?}");
    let resolved = corvid_resolve::resolve(&file);
    let checked = corvid_types::typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
    let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
    emit_application_contract(
        &file,
        &resolved,
        &checked,
        &registry,
        &ContractOptions { source_path: "app.cor", compiler_version: "test", generated_at: "now" },
    )
}

#[test]
fn openapi_publishes_the_shared_schema_for_each_type() {
    let document = emit_openapi(&contract());
    let text = document.to_string();
    for name in corvid_public_json::text_scalar::NAMES {
        let schema = corvid_public_json::text_scalar::json_schema_by_name(name).unwrap();
        assert!(text.contains(&schema.to_string()), "{name}'s schema is published: {text}");
    }
    assert!(text.contains(r#""format":"decimal""#), "{text}");
}

#[test]
fn the_typescript_client_and_sdks_hold_the_exact_text_as_strings() {
    let contract = contract();
    let ts: String = emit_ts_client(&contract).into_iter().map(|f| f.contents).collect();
    assert!(ts.contains("total: string"), "{ts}");
    assert!(ts.contains("due: string"), "{ts}");
    let swift = emit_sdk(&contract, SdkLanguage::Swift).remove(0).contents;
    assert!(swift.contains("total: String"), "{swift}");
    let kotlin = emit_sdk(&contract, SdkLanguage::Kotlin).remove(0).contents;
    assert!(kotlin.contains("total: String"), "{kotlin}");
    let python = emit_sdk(&contract, SdkLanguage::Python).remove(0).contents;
    assert!(python.contains("total: str"), "{python}");
}

#[test]
fn the_python_sdk_round_trips_the_exact_text() {
    let Some(python) = ["python3", "python"]
        .into_iter()
        .find(|p| Command::new(p).arg("--version").output().map(|o| o.status.success()).unwrap_or(false))
    else {
        eprintln!("skipping: no Python interpreter on PATH");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("models.py"), emit_sdk(&contract(), SdkLanguage::Python).remove(0).contents).unwrap();
    std::fs::write(
        dir.path().join("check.py"),
        r#"
import models
INVOICE = {
    "total": "19.90",
    "issued": "2026-07-11T08:30:00.000000001Z",
    "terms": "PT1H30M",
    "due": "2026-03-29T03:30:00+02:00[Europe/Berlin]",
    "office": "Asia/Tokyo",
    "currency": "EUR",
    "locale": "de-DE",
}
invoice = models.Invoice.from_json(INVOICE)
assert invoice.total == "19.90" and invoice.issued.endswith(".000000001Z")
assert invoice.to_json() == INVOICE, invoice.to_json()
try:
    models.Invoice.from_json({**INVOICE, "total": 19.9})
except models.WireError as failure:
    assert "expected Decimal" in str(failure), str(failure)
else:
    raise AssertionError("a Decimal number must be refused")
print("ok")
"#,
    )
    .unwrap();
    let output = Command::new(python).arg("check.py").current_dir(dir.path()).output().expect("run python");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && stdout.trim() == "ok", "stdout={stdout}\nstderr={}", String::from_utf8_lossy(&output.stderr));
}
