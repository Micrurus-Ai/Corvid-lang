//! B06 — the new types cross every JSON boundary as their exact text
//! (tools, prompts, `to_json` and `from_json`, routes, stores), and a
//! table stores each in a column of its own (B09 P8a and P8c).

use super::check;

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

/// B09 P8a and P8c: a table stores every B06 type and `Bytes`, in a
/// column of its own or, inside a record, as its exact JSON text.
#[test]
fn a_table_column_may_hold_money_time_and_bytes() {
    let errors = check(
        "type Price:
    amount: Decimal

type Signed:
    signature: Bytes

database shop:
    engine: sqlite
    location: \"shop.sqlite\"

table Product:
    key id: String
    price: Decimal
    nested: Price
    data: Bytes
    proof: Signed
",
    )
    .errors;
    assert!(errors.is_empty(), "got {errors:?}");
}
