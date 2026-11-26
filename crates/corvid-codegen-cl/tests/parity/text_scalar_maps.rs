//! Native maps keyed by, or holding, a text-held scalar (native scalar
//! boundaries B3). A key is found by value through the type's own
//! comparison, as `==` and the interpreter's map find it, so `1.0` and
//! `1.00` are one `Decimal` key; values are held as reference-counted
//! text. Every program runs on both tiers and must answer the same.

use super::scalar_harness::assert_parity_text;

#[test]
fn a_decimal_key_is_one_key_whatever_its_scale() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    prices: Map<Decimal, String> = {1.0: "one", 2.50: "two and a half"}
    prices[1.00] = "one again"
    same = prices.contains_key(2.5)
    found = match prices.get(1):
        Some(label) -> label
        None -> "missing"
    keys = ""
    for key in prices.keys():
        keys = keys + key.to_string() + ";"
    return found + " " + same.to_string() + " " + prices.length().to_string() + " " + keys
"#,
    );
    // The first key's text is kept, as on the interpreter.
    assert_eq!(out, "one again true 2 1.0;2.50;");
}

#[test]
fn time_currency_and_bytes_keys_and_text_scalar_values() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    start = Instant.from_epoch_milliseconds(1783758600012)
    by_instant: Map<Instant, Decimal> = {start: 4.50}
    by_instant[start + 1 hour] = 9.00
    by_wait: Map<Duration, String> = {90 seconds: "short"}
    by_currency: Map<Currency, Duration> = {"EUR": 30 seconds}
    by_bytes: Map<Bytes, Int> = {"ab".utf8(): 1}
    total: Decimal = 0.00
    for amount in by_instant.values():
        total = total + amount
    wait = match by_currency.get("EUR"):
        Some(d) -> d.to_string()
        None -> "none"
    label = match by_wait.get(1.5 minutes):
        Some(text) -> text
        None -> "none"
    return total.to_string() + " " + wait + " " + label + " " + by_bytes.contains_key("ab".utf8()).to_string()
"#,
    );
    assert_eq!(out, "13.50 PT30S short true");
}

#[test]
fn decimal_keyed_maps_compare_by_value() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    a: Map<Decimal, Int> = {1.0: 1, 2.50: 2}
    b: Map<Decimal, Int> = {2.5: 2, 1.00: 1}
    c: Map<Decimal, Int> = {2.5: 3, 1.00: 1}
    return (a == b).to_string() + " " + (a == c).to_string()
"#,
    );
    assert_eq!(out, "true false");
}
