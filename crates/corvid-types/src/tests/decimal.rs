//! B06 `Decimal` in the checker: number literals typed by context, the
//! operator rules, rounded division, literal arguments, and the JSON
//! boundaries `Decimal` cannot cross before its JSON slice.

use super::check;
use crate::errors::TypeErrorKind;

fn kinds(src: &str) -> Vec<TypeErrorKind> {
    check(src).errors.into_iter().map(|e| e.kind).collect()
}

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn literals_become_decimals_where_a_decimal_is_meant() {
    assert_clean(
        "type Line:
    price: Decimal
    quantity: Int

agent price_of(line: Line) -> Decimal:
    return line.price * line.quantity

agent main() -> Decimal:
    price: Decimal = 19.99
    discount: Decimal = -0.50
    zero: Decimal = 0
    line = Line(4.50, 2)
    taxed = price * 1.0825
    sum: Decimal = 0.1 + 0.2
    total = price_of(line) + taxed + discount + zero + sum
    total += 0.05
    if total > 100:
        return total - 1
    return total
",
    );
}

#[test]
fn a_literal_without_a_decimal_nearby_stays_a_float() {
    let checked = check("agent main() -> Float:\n    return 0.1 + 0.2\n");
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn mixing_decimal_and_float_is_a_compile_error() {
    let errors = kinds("agent main(rate: Float) -> Decimal:\n    price: Decimal = 19.99\n    return price * rate\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::DecimalFloatMix { context } if context == "`*`")),
        "got {errors:?}"
    );
    let errors = kinds("agent main(rate: Float, price: Decimal) -> Bool:\n    return price < rate\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::DecimalFloatMix { .. })), "got {errors:?}");
}

#[test]
fn decimal_and_float_slots_name_the_conversion() {
    let errors = check("agent main(price: Decimal) -> Float:\n    x: Float = price\n    return x\n").errors;
    let mismatch = errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::TypeMismatch { expected, got, .. } if expected == "Float" && got == "Decimal"))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    assert!(mismatch.kind.hint().unwrap().contains("to_float()"));

    let errors = check("agent main(count: Int) -> Decimal:\n    x: Decimal = count\n    return x\n").errors;
    let mismatch = errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::TypeMismatch { expected, got, .. } if expected == "Decimal" && got == "Int"))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    assert!(mismatch.kind.hint().unwrap().contains("to_decimal()"));
}

#[test]
fn a_literal_with_no_exact_decimal_value_is_a_compile_error() {
    let errors = kinds("agent main() -> Decimal:\n    return 0.12345678901234567890123456789\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::InvalidDecimalLiteral { literal, problem }
            if literal == "0.12345678901234567890123456789" && problem.contains("29 digits after the point"))),
        "got {errors:?}"
    );
}

#[test]
fn division_must_be_rounded_where_it_happens() {
    let errors = kinds("agent main(total: Decimal) -> Decimal:\n    return total / 3\n");
    assert!(errors.contains(&TypeErrorKind::DecimalDivisionNeedsRounding), "got {errors:?}");
    let errors = kinds("agent main(total: Decimal) -> Decimal:\n    total /= 3\n    return total\n");
    assert!(errors.contains(&TypeErrorKind::DecimalDivisionNeedsRounding), "got {errors:?}");
    let errors = kinds("agent main(total: Decimal) -> Decimal:\n    half = total / 2\n    return half.round_half_even(2)\n");
    assert!(errors.contains(&TypeErrorKind::DecimalDivisionNeedsRounding), "got {errors:?}");
    assert_clean(
        "agent main(total: Decimal, people: Int) -> Decimal:
    share = (total / people).round_half_even(2)
    third = (total / 3.0).round_floor(4)
    return share + third
",
    );
}

#[test]
fn every_rounding_mode_is_a_method() {
    assert_clean(
        "agent main(x: Decimal) -> List<Decimal>:
    return [x.round_half_even(2), x.round_half_up(2), x.round_toward_zero(0), x.round_away_from_zero(1), x.round_floor(3), x.round_ceiling(28)]
",
    );
    let errors = kinds("agent main(x: Decimal) -> Decimal:\n    return x.round_nearest(2)\n");
    assert!(!errors.is_empty(), "an unknown mode must not typecheck");
}

#[test]
fn literal_places_outside_zero_to_twenty_eight_are_compile_errors() {
    let errors = kinds("agent main(x: Decimal) -> Decimal:\n    return x.round_half_even(29)\n");
    assert!(errors.contains(&TypeErrorKind::DecimalPlacesOutOfRange { places: 29 }), "got {errors:?}");
    let errors = kinds("agent main(x: Decimal) -> Decimal:\n    return (x / 2).round_floor(-1)\n");
    assert!(errors.contains(&TypeErrorKind::DecimalPlacesOutOfRange { places: -1 }), "got {errors:?}");
}

#[test]
fn dividing_by_a_literal_zero_is_a_compile_error() {
    let errors = kinds("agent main(x: Decimal) -> Decimal:\n    return (x / 0).round_half_even(2)\n");
    assert!(errors.contains(&TypeErrorKind::DecimalDivisionByZero), "got {errors:?}");
    let errors = kinds("agent main(x: Decimal) -> Decimal:\n    return x % 0.00\n");
    assert!(errors.contains(&TypeErrorKind::DecimalDivisionByZero), "got {errors:?}");
}

#[test]
fn conversions_and_methods_typecheck() {
    assert_clean(
        "agent main(n: Int, f: Float, text: String, d: Decimal) -> Int:
    a = n.to_decimal() + f.to_decimal()
    parsed = text.parse_decimal()
    back = d.to_float() + 1.5
    label = d.abs().to_string()
    return d.scale() + d.to_int_truncated() + label.length() + back.round()
",
    );
}

#[test]
fn decimal_cannot_cross_a_json_boundary_before_its_json_slice() {
    let errors = kinds(
        "type Quote:
    amount: Decimal

tool charge(amount: Decimal) -> String

tool quote(item: String) -> Quote

prompt estimate(item: String) -> Decimal:
    \"Estimate {item}\"
",
    );
    let boundaries: Vec<_> = errors
        .iter()
        .filter_map(|kind| match kind {
            TypeErrorKind::PendingJsonFormAtBoundary { boundary, .. } => Some(boundary.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(boundaries, ["a tool signature", "a tool signature", "a prompt signature"], "got {errors:?}");
}

#[test]
fn to_json_on_a_decimal_record_says_why() {
    let errors = check("type Quote:\n    amount: Decimal\n\nagent main(q: Quote) -> String:\n    return q.to_json()\n").errors;
    assert!(
        errors.iter().any(|e| e.kind.hint().is_some_and(|hint| hint.contains("parse_decimal()"))),
        "got {errors:?}"
    );
}

#[test]
fn decimal_is_a_type_not_a_value() {
    let errors = kinds("agent main() -> Decimal:\n    return Decimal\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TypeAsValue { .. })), "got {errors:?}");
}
