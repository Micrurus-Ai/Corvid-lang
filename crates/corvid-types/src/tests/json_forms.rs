//! B06 — the new types cross every JSON boundary as their exact text
//! (tools, prompts, `to_json` and `from_json`, routes, stores), and stay
//! refused as table columns until B09 maps their storage.

use super::check;
use crate::errors::TypeErrorKind;

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

const ALL: &str = "type Invoice:
    total: Decimal
    issued: Instant
    terms: Duration
    due: ZonedTime
    office: TimeZone
    currency: Currency
    locale: Locale
";

#[test]
fn every_b06_type_has_a_json_form() {
    assert_clean(&format!(
        "{ALL}
tool charge(invoice: Invoice) -> Invoice

prompt estimate(item: String) -> Decimal:
    \"Estimate {{item}}\"

agent main(text: String, invoice: Invoice) -> String:
    back = Invoice.from_json(text)
    return invoice.to_json()
"
    ));
}

#[test]
fn routes_and_stores_carry_b06_types() {
    assert_clean(&format!(
        "{ALL}
memory Ledger:
    last: Option<Invoice>

agent latest() -> Invoice:
    return Invoice(1.50, Instant.from_epoch_milliseconds(0), Instant.from_epoch_milliseconds(0) - Instant.from_epoch_milliseconds(0), Instant.from_epoch_milliseconds(0).in_zone(\"UTC\"), \"UTC\", \"EUR\", \"de-DE\")

server Billing:
    route GET \"/invoice\" -> json Invoice:
        return latest()
"
    ));
}

/// B09 P8a stores the B06 types in exact columns, and inside a
/// JSON-stored column as their exact text, so only `Bytes` still waits
/// for its column (P8c), directly or inside a record.
#[test]
fn a_table_column_waits_only_for_bytes() {
    let errors: Vec<_> = check(
        "type Price:
    amount: Decimal

type Signed:
    signature: Bytes

table Product:
    key id: String
    price: Decimal
    nested: Price
    data: Bytes
    proof: Signed
",
    )
    .errors
    .into_iter()
    .map(|e| e.kind)
    .collect();
    let columns: Vec<_> = errors
        .iter()
        .filter_map(|kind| match kind {
            TypeErrorKind::PendingJsonFormAtBoundary { type_name, boundary } if boundary == "a table column" => {
                Some(type_name.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(columns, ["Bytes", "Bytes"], "got {errors:?}");
}
