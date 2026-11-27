//! B06 `Decimal` on the native tier. Every program runs on the
//! interpreter and as a compiled binary; both must return the same
//! text, and the binary must release every allocation. Traps must stop
//! both tiers with the same `corvid-decimal` message.

use super::scalar_harness::{assert_parity_text, assert_parity_trap};

#[test]
fn literals_keep_their_scale_and_sums_are_exact() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    price: Decimal = 2.50
    tiny: Decimal = 0.0000000000000000000000000001
    exact: Decimal = 0.1 + 0.2
    a: Decimal = 0.1
    b: Decimal = 0.2
    return price.to_string() + " " + tiny.to_string() + " " + exact.to_string() + " " + (a + b).to_string()
"#,
    );
    assert_eq!(out, "2.50 0.0000000000000000000000000001 0.3 0.3");
}

#[test]
fn an_invoice_mixes_decimals_and_ints() {
    let out = assert_parity_text(
        r#"
type Line:
    name: String
    price: Decimal
    quantity: Int

agent main() -> String:
    lines = [Line("Coffee", 4.50, 2), Line("Bagel", 2.25, 3), Line("Napkin", 0.10, 3)]
    subtotal: Decimal = 0
    for line in lines:
        subtotal = subtotal + line.price * line.quantity
    tax = (subtotal * 0.0825).round_half_even(2)
    share = (subtotal / 3).round_half_even(2)
    return subtotal.to_string() + " + " + tax.to_string() + " tax, " + share.to_string() + " each"
"#,
    );
    assert_eq!(out, "16.05 + 1.32 tax, 5.35 each");
}

#[test]
fn every_rounding_mode_agrees_on_ties() {
    let out = assert_parity_text(
        r#"
agent modes(x: Decimal) -> String:
    return x.round_half_even(0).to_string() + "," + x.round_half_up(0).to_string() + "," + x.round_toward_zero(0).to_string() + "," + x.round_away_from_zero(0).to_string() + "," + x.round_floor(0).to_string() + "," + x.round_ceiling(0).to_string()

agent main() -> String:
    up: Decimal = 2.5
    down: Decimal = -2.5
    odd: Decimal = 3.5
    near: Decimal = 2.4
    return modes(up) + " " + modes(down) + " " + modes(odd) + " " + modes(near) + " " + up.round_half_even(2).to_string()
"#,
    );
    assert_eq!(out, "2,3,2,3,2,3 -2,-3,-2,-3,-3,-2 4,4,3,4,3,4 2,2,2,3,2,3 2.50");
}

#[test]
fn rounded_division_rounds_once_with_its_mode() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    total: Decimal = 100.00
    people = 3
    third: Decimal = 3
    return (total / 3).round_half_even(2).to_string() + " " + (total / people).round_floor(2).to_string() + " " + (total / people).round_ceiling(2).to_string() + " " + (total / third).round_toward_zero(4).to_string() + " " + (7 / third).round_half_up(3).to_string() + " " + (total / third).round_away_from_zero(2).to_string()
"#,
    );
    assert_eq!(out, "33.33 33.33 33.34 33.3333 2.333 33.34");
}

#[test]
fn equality_and_order_are_numeric_and_mix_with_ints() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    a: Decimal = 1.0
    b: Decimal = 1.00
    c: Decimal = 2.5
    n = 2
    checks = [a == b, a != b, a == 1, c > 2, c < 3, 2 < c, c >= 2.50, c <= 2.49, n < c, c % 2 == 0.5]
    text = ""
    for check in checks:
        if check:
            text = text + "t"
        else:
            text = text + "f"
    return text
"#,
    );
    assert_eq!(out, "tftttttftt");
}

#[test]
fn remainder_negation_and_conversions() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    d: Decimal = -2.99
    big: Decimal = 7.5
    n = 7
    f = 0.1
    return (big % 2).to_string() + " " + (d % 2).to_string() + " " + (-d).to_string() + " " + (-big).to_string() + " " + d.abs().to_string() + " " + n.to_decimal().to_string() + " " + f.to_decimal().to_string()
"#,
    );
    assert_eq!(out, "1.5 -0.99 2.99 -7.5 2.99 7 0.1");
}

#[test]
fn scale_float_and_int_conversions() {
    let out = assert_parity_text(
        r#"
agent main() -> Int:
    d: Decimal = -2.99
    price: Decimal = 2.50
    whole: Decimal = 12345678901234
    near = 0
    if d.to_float() == -2.99:
        near = 1
    return d.scale() * 1000000000000000 + price.scale() * 100000000000000 + whole.to_int_truncated() * 10 + d.to_int_truncated() + 2 + near
"#,
    );
    assert_eq!(out, "2323456789012341");
}

#[test]
fn a_decimal_entry_result_prints_its_exact_text() {
    let out = assert_parity_text(
        r#"
agent main() -> Decimal:
    price: Decimal = 19.90
    return price * 2
"#,
    );
    assert_eq!(out, "39.80");
}

#[test]
fn parse_decimal_returns_the_same_ok_and_err_on_both_tiers() {
    let out = assert_parity_text(
        r#"
agent show(result: Result<Decimal, String>) -> String:
    return match result:
        Ok(d) -> "ok:" + d.to_string()
        Err(e) -> "err:" + e

agent total(a: String, b: String) -> Result<Decimal, String>:
    x = a.parse_decimal()?
    y = b.parse_decimal()?
    return Ok(x + y)

agent main() -> String:
    return show(" 19.990 ".parse_decimal()) + "|" + show("1e3".parse_decimal()) + "|" + show("1,000".parse_decimal()) + "|" + show(total("1.5", "2.25")) + "|" + show(total("1.5", ".5"))
"#,
    );
    assert!(out.starts_with("ok:19.990|err:`1e3` is not a decimal number: exponents are not accepted; write the digits out at position 1|"), "{out}");
    assert!(out.contains("|ok:3.75|err:`.5` is not a decimal number: "), "{out}");
}

#[test]
fn records_lists_and_options_of_decimals_compare_numerically() {
    let out = assert_parity_text(
        r#"
type Price:
    label: String
    amount: Decimal

agent main() -> String:
    one = Price("tea", 1.0)
    same = Price("tea", 1.00)
    other = Price("tea", 1.01)
    xs: List<Decimal> = [1.0, 2.50]
    ys: List<Decimal> = [1.00, 2.5]
    zs: List<Decimal> = [1.0, 2.51]
    first = xs.first()
    checks = [one == same, one != other, xs == ys, xs == zs, first == ys.first()]
    text = ""
    for check in checks:
        if check:
            text = text + "t"
        else:
            text = text + "f"
    return text + " " + one.amount.to_string() + " " + same.amount.to_string()
"#,
    );
    assert_eq!(out, "tttft 1.0 1.00");
}

#[test]
fn json_carries_decimals_as_exact_strings_and_refuses_numbers() {
    let out = assert_parity_text(
        r#"
type Price:
    label: String
    amount: Decimal

agent same(equal: Bool) -> String:
    if equal:
        return "equal"
    return "different"

agent main() -> String:
    price = Price("tea", 19.90)
    text = price.to_json()
    decoded = match Price.from_json(text):
        Ok(p) -> p
        Err(e) -> Price(e.message, 0)
    back = decoded.amount.to_string() + " " + same(decoded == price)
    refused = match Price.from_json("{\"label\": \"tea\", \"amount\": 19.99}"):
        Ok(p) -> p.amount.to_string()
        Err(e) -> e.path + ": " + e.found
    malformed = match Price.from_json("{\"label\": \"tea\", \"amount\": \"1e3\"}"):
        Ok(p) -> p.amount.to_string()
        Err(e) -> e.path + " " + e.expected + ": " + e.found
    amounts: List<Decimal> = [1.0, 2.50]
    maybe: Option<Decimal> = Some(0.10)
    return text + " | " + back + " | " + refused + " | " + malformed + " | " + amounts.to_json() + " " + maybe.to_json() + " " + price.amount.to_json()
"#,
    );
    assert!(out.starts_with(r#"{"amount":"19.90","label":"tea"} | 19.90 equal | amount: the number 19.99; a Decimal is written as a string, such as "19.99", so no digit is lost | amount Decimal: "#), "{out}");
    assert!(out.ends_with(r#" | ["1.0","2.50"] "0.10" "19.90""#), "{out}");
}

#[test]
fn overflow_traps_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Decimal:
    big: Decimal = 9999999999999999999999999999999999999.9
    return big + 1
"#,
        "Decimal overflow: the result needs more than 38 significant digits",
    );
}

#[test]
fn a_product_past_28_places_traps_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Decimal:
    tiny: Decimal = 0.000000000000003
    return tiny * tiny
"#,
        "it needs 30 digits after the point, and a Decimal holds at most 28",
    );
}

#[test]
fn division_by_a_zero_value_traps_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Decimal:
    total: Decimal = 10
    divisor = 0
    return (total / divisor).round_half_even(2)
"#,
        "Decimal division by zero",
    );
}

#[test]
fn computed_places_and_non_finite_floats_trap_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Decimal:
    price: Decimal = 2.5
    places = 29
    return price.round_half_even(places)
"#,
        "places must be between 0 and 28, got 29",
    );
    assert_parity_trap(
        r#"
agent main() -> Decimal:
    nan = 0.0 / 0.0
    return nan.to_decimal()
"#,
        "a Float that is NaN or infinite has no Decimal value",
    );
}

/// Compound assignment on a `Decimal` local compiles natively as the
/// rebinding it means, releasing each old total.
#[test]
fn compound_assignment_on_a_decimal_total_matches_the_interpreter() {
    let out = assert_parity_text(
        "agent main() -> Decimal:\n    total: Decimal = 0\n    prices: List<Decimal> = [1.50, 2.25, 0.10]\n    for price in prices:\n        total += price\n    total -= 0.05\n    total *= 2\n    return total\n",
    );
    assert_eq!(out, "7.60");
}

/// A `Decimal` literal beyond `Int`'s range keeps every digit natively too.
#[test]
fn decimal_literals_beyond_int_keep_every_digit() {
    assert_eq!(
        assert_parity_text("agent f() -> Decimal:\n    return -123456789012345678901234567890 + 1\n"),
        "-123456789012345678901234567889"
    );
}
