//! B06 — `to_json` and `from_json` carry the new types as their exact
//! text, and a `Decimal` written as a JSON number is refused with the
//! reason, never read through a binary float.

use super::*;

async fn text_of(src: &str) -> String {
    let ir = ir_of(src);
    match run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run") {
        Value::String(s) => s.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

const INVOICE: &str = "\
type Invoice:
    total: Decimal
    issued: Instant
    terms: Duration
    due: ZonedTime
    office: TimeZone
    currency: Currency
    locale: Locale

";

#[tokio::test]
async fn every_b06_type_round_trips_as_its_exact_text() {
    let src = format!(
        "{INVOICE}agent main() -> String:
    issued = Instant.from_epoch_milliseconds(1783758600012)
    terms = issued - Instant.from_epoch_milliseconds(1783758600012 - 5400000)
    invoice = Invoice(19.90, issued, terms, issued.in_zone(\"Europe/Berlin\"), \"Asia/Tokyo\", \"EUR\", \"de-DE\")
    text = invoice.to_json()
    back = match Invoice.from_json(text):
        Ok(i) -> i == invoice
        Err(e) -> false
    return text + \" \" + back.to_string()
"
    );
    assert_eq!(
        text_of(&src).await,
        r#"{\"currency\":\"EUR\",\"due\":\"2026-07-11T10:30:00.012+02:00[Europe/Berlin]\",\"issued\":\"2026-07-11T08:30:00.012Z\",\"locale\":\"de-DE\",\"office\":\"Asia/Tokyo\",\"terms\":\"PT1H30M\",\"total\":\"19.90\"} true"#
            .replace("\\\"", "\"")
    );
}

#[tokio::test]
async fn a_decimal_must_be_a_string_in_json() {
    let src = "\
type Price:
    amount: Decimal

agent main() -> String:
    return match Price.from_json(\"{\\\"amount\\\": 19.99}\"):
        Ok(p) -> p.amount.to_string()
        Err(e) -> e.path + \": \" + e.found
";
    assert_eq!(
        text_of(src).await,
        "amount: the number 19.99; a Decimal is written as a string, such as \"19.99\", so no digit is lost"
    );
}

#[tokio::test]
async fn malformed_text_names_the_field_and_the_problem() {
    let src = "\
type Meeting:
    office: TimeZone

agent main() -> String:
    return match Meeting.from_json(\"{\\\"office\\\": \\\"Europe/Berln\\\"}\"):
        Ok(m) -> m.office.to_string()
        Err(e) -> e.path + \" \" + e.expected + \": \" + e.found
";
    assert_eq!(
        text_of(src).await,
        "office TimeZone: `Europe/Berln`: `Europe/Berln` is not a time zone in the IANA database; did you mean `Europe/Berlin`?"
    );
}
