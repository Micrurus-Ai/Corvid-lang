//! B06 `Decimal` on the interpreter, run from Corvid source. Digit-level
//! rules are tested in `corvid-decimal`; these tests pin the language
//! surface: literals keep their digits, operators are exact, division is
//! rounded once, and failures trap instead of rounding.

use super::*;

async fn run_main(src: &str) -> Result<Value, crate::errors::InterpError> {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await
}

async fn text_of(src: &str) -> String {
    match run_main(src).await.expect("run") {
        Value::String(s) => s.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

#[tokio::test]
async fn literals_keep_every_digit_and_their_scale() {
    let src = "\
agent main() -> String:
    price: Decimal = 2.50
    tiny: Decimal = 0.0000000000000000000000000001
    return price.to_string() + \" \" + tiny.to_string() + \" \" + price.scale().to_string()
";
    assert_eq!(text_of(src).await, "2.50 0.0000000000000000000000000001 2");
}

#[tokio::test]
async fn decimal_sums_are_exact_where_floats_drift() {
    let src = "\
agent main() -> String:
    total: Decimal = 0
    for i in range(0, 1000):
        total += 0.10
    exact: Decimal = 0.1 + 0.2
    drift = 0.1 + 0.2
    return total.to_string() + \" \" + exact.to_string() + \" \" + drift.to_string()
";
    assert_eq!(text_of(src).await, "100.00 0.3 0.30000000000000004");
}

#[tokio::test]
async fn an_invoice_with_tax_and_a_split() {
    let src = "\
type Line:
    price: Decimal
    quantity: Int

agent main() -> String:
    lines = [Line(4.50, 2), Line(2.25, 3), Line(0.10, 3)]
    subtotal: Decimal = 0
    for line in lines:
        subtotal = subtotal + line.price * line.quantity
    tax = (subtotal * 0.0825).round_half_even(2)
    share = (subtotal / 3).round_half_even(2)
    return subtotal.to_string() + \" \" + tax.to_string() + \" \" + share.to_string()
";
    assert_eq!(text_of(src).await, "16.05 1.32 5.35");
}

#[tokio::test]
async fn each_rounding_mode_on_a_tie() {
    let src = "\
agent main() -> String:
    x: Decimal = -2.5
    y: Decimal = 2.5
    return x.round_half_even(0).to_string() + \" \" + x.round_half_up(0).to_string() + \" \" + x.round_toward_zero(0).to_string() + \" \" + x.round_away_from_zero(0).to_string() + \" \" + x.round_floor(0).to_string() + \" \" + x.round_ceiling(0).to_string() + \" \" + y.round_half_even(2).to_string()
";
    assert_eq!(text_of(src).await, "-2 -3 -2 -3 -3 -2 2.50");
}

#[tokio::test]
async fn rounded_division_rounds_once_with_its_mode() {
    let src = "\
agent main(people: Int) -> String:
    total: Decimal = 100.00
    down = (total / people).round_floor(2)
    up = (total / people).round_ceiling(2)
    return down.to_string() + \" \" + up.to_string()
";
    let ir = ir_of(src);
    let value = run_agent(&ir, "main", vec![Value::Int(3)], &empty_runtime()).await.expect("run");
    assert_eq!(value, Value::String(Arc::from("33.33 33.34")));
}

#[tokio::test]
async fn ints_mix_exactly_and_compare_numerically() {
    let src = "\
agent main() -> List<Bool>:
    a: Decimal = 1.0
    b: Decimal = 1.00
    c: Decimal = 2.5
    return [a == b, a == 1, c > 2, c < 3, 2 < c, c % 2 == 0.5]
";
    let Value::List(items) = run_main(src).await.expect("run") else { panic!("expected a list") };
    assert_eq!(items.iter_cloned(), vec![Value::Bool(true); 6]);
}

#[tokio::test]
async fn conversions_name_their_direction() {
    let src = "\
agent main() -> String:
    n = 7
    f = 0.1
    d: Decimal = -2.99
    return n.to_decimal().to_string() + \" \" + f.to_decimal().to_string() + \" \" + d.to_float().to_string() + \" \" + d.to_int_truncated().to_string() + \" \" + d.abs().to_string() + \" \" + (-d).to_string()
";
    assert_eq!(text_of(src).await, "7 0.1 -2.99 -2 2.99 2.99");
}

#[tokio::test]
async fn parse_decimal_returns_a_result() {
    let src = "\
agent main() -> String:
    good = match \" 19.990 \".parse_decimal():
        Ok(d) -> d.to_string()
        Err(e) -> e
    bad = match \"1e3\".parse_decimal():
        Ok(d) -> d.to_string()
        Err(e) -> e
    return good + \"|\" + bad
";
    assert_eq!(
        text_of(src).await,
        "19.990|`1e3` is not a decimal number: exponents are not accepted; write the digits out at position 1"
    );
}

#[tokio::test]
async fn overflow_traps_instead_of_rounding() {
    let src = "\
agent main() -> Decimal:
    big: Decimal = 9999999999999999999999999999999999999.9
    return big + 1
";
    let error = run_main(src).await.expect_err("overflow must trap");
    assert!(error.to_string().contains("Decimal overflow"), "got {error}");

    let src = "\
agent main() -> Decimal:
    tiny: Decimal = 0.000000000000003
    return tiny * tiny
";
    let error = run_main(src).await.expect_err("a product past 28 places must trap");
    assert!(error.to_string().contains("30 digits after the point"), "got {error}");
}

#[tokio::test]
async fn division_by_a_zero_value_traps() {
    let src = "\
agent main(divisor: Int) -> Decimal:
    total: Decimal = 10
    return (total / divisor).round_half_even(2)
";
    let ir = ir_of(src);
    let error = run_agent(&ir, "main", vec![Value::Int(0)], &empty_runtime())
        .await
        .expect_err("division by zero must trap");
    assert!(error.to_string().contains("division by zero"), "got {error}");
}

#[tokio::test]
async fn float_to_decimal_traps_on_nan() {
    let src = "\
agent main() -> Decimal:
    nan = 0.0 / 0.0
    return nan.to_decimal()
";
    let error = run_main(src).await.expect_err("NaN has no Decimal value");
    assert!(error.to_string().contains("NaN"), "got {error}");
}
