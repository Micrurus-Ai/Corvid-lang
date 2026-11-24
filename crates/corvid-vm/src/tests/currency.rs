//! B06 currency formatting on the interpreter, run from Corvid source.
//! Per-locale output is tested in `corvid-currency`; these tests pin the
//! language surface and the no-rounding rule.

use super::*;

async fn run_main(src: &str) -> Result<Value, crate::errors::InterpError> {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await
}

#[tokio::test]
async fn an_invoice_in_three_locales() {
    let src = "\
agent main() -> String:
    subtotal: Decimal = 1234.5
    euro: Currency = \"EUR\"
    us = subtotal.format_currency(\"USD\", \"en-US\")
    de = subtotal.format_currency(euro, \"de-DE\")
    yen = subtotal.round_half_even(0).format_currency(\"JPY\", \"ja-JP\")
    return us + \"|\" + de + \"|\" + yen
";
    assert_eq!(
        run_main(src).await.expect("run"),
        Value::String(Arc::from("$1,234.50|1.234,50\u{a0}€|￥1,234"))
    );
}

#[tokio::test]
async fn formatting_never_rounds() {
    let src = "\
agent main() -> String:
    price: Decimal = 1.005
    return price.format_currency(\"USD\", \"en-US\")
";
    let error = run_main(src).await.expect_err("an amount past the minor unit must trap");
    assert!(error.to_string().contains("formatting never rounds"), "got {error}");
    assert!(error.to_string().contains("round_half_even(2)"), "got {error}");
}

#[tokio::test]
async fn minor_digits_compose_with_declared_rounding() {
    let src = "\
agent main() -> String:
    total: Decimal = 100
    dinar: Currency = \"KWD\"
    share = (total / 3).round_half_even(dinar.minor_digits())
    return share.to_string() + \" \" + share.format_currency(dinar, \"en-US\")
";
    assert_eq!(run_main(src).await.expect("run"), Value::String(Arc::from("33.333 KWD\u{a0}33.333")));
}

#[tokio::test]
async fn codes_and_tags_from_run_time_parse_into_results() {
    let src = "\
agent main() -> String:
    good = match \"sw-KE\".parse_locale():
        Ok(l) -> l.to_string()
        Err(e) -> e
    bad = match \"XYZ\".parse_currency():
        Ok(c) -> c.to_string()
        Err(e) -> e
    return good + \"|\" + bad
";
    assert_eq!(
        run_main(src).await.expect("run"),
        Value::String(Arc::from("sw-KE|`XYZ` is not an ISO 4217 currency code, such as USD or EUR"))
    );
}
