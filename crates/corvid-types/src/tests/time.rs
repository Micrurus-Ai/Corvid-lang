//! B06 `Duration` and `Instant` in the checker: the operator table,
//! the refusal of unit-less numbers, the type method on `Instant`, and
//! the boundaries time may and may not cross before its JSON slice.

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
fn the_operator_table_typechecks() {
    assert_clean(
        "agent main(start: Instant, end: Instant, step: Duration, count: Int) -> Bool:
    span: Duration = end - start
    total: Duration = step * count + count * step - step / 2
    finish: Instant = start + total
    earlier: Instant = total + start - step
    back = -step
    return finish > earlier and span >= total and back.abs() == step and start != end
",
    );
}

#[test]
fn a_bare_number_has_no_unit() {
    let errors = check("agent main(step: Duration) -> Duration:\n    return step + 30\n").errors;
    let error = errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::TimeOperatorMismatch { left, right, .. } if left == "Duration" && right == "Int"))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    assert!(error.kind.hint().unwrap().contains("no unit"));
    let errors = kinds("agent main(at: Instant) -> Instant:\n    return at + 1000\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TimeOperatorMismatch { .. })), "got {errors:?}");
}

#[test]
fn instants_subtract_but_do_not_add() {
    let errors = kinds("agent main(a: Instant, b: Instant) -> Instant:\n    return a + b\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TimeOperatorMismatch { .. })), "got {errors:?}");
    let errors = kinds("agent main(a: Duration, b: Duration) -> Duration:\n    return a / b\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TimeOperatorMismatch { .. })), "got {errors:?}");
    let errors = kinds("agent main(a: Duration, b: Instant) -> Bool:\n    return a < b\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TimeOperatorMismatch { .. })), "got {errors:?}");
}

#[test]
fn methods_and_parsing_typecheck() {
    assert_clean(
        "agent main(text: String, d: Duration, at: Instant) -> Int:
    parsed = text.parse_duration()
    moment = text.parse_instant()
    label = d.to_string() + at.to_string()
    from_epoch = Instant.from_epoch_milliseconds(1700000000000)
    return d.whole_nanoseconds() + d.whole_microseconds() + d.whole_milliseconds() + d.whole_seconds() + d.whole_minutes() + d.whole_hours() + at.epoch_milliseconds() + from_epoch.epoch_milliseconds() + label.length()
",
    );
    let errors = kinds("agent main(d: Duration) -> Int:\n    return d.whole_days()\n");
    assert!(!errors.is_empty(), "a day is not a Duration unit");
    let errors = kinds("agent main() -> Instant:\n    return Instant.from_epoch_milliseconds(\"soon\")\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TypeMismatch { .. })), "got {errors:?}");
}

#[test]
fn time_crosses_tools_and_prompts_as_text_but_not_routes_yet() {
    assert_clean(
        "tool book_slot(at: Instant, every: Duration) -> Instant

prompt suggest_time(goal: String) -> Instant:
    \"When should {goal} happen?\"
",
    );
    let errors = kinds(
        "type Booking:
    at: Instant

memory Calendar:
    next: Instant
",
    );
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::PendingJsonFormAtBoundary { type_name, boundary }
            if type_name == "Instant" && boundary == "a store field")),
        "got {errors:?}"
    );
    let errors = check("type Booking:\n    at: Instant\n\nagent main(b: Booking) -> String:\n    return b.to_json()\n").errors;
    assert!(errors.iter().any(|e| e.kind.hint().is_some_and(|hint| hint.contains("parse_instant()"))), "got {errors:?}");
}

#[test]
fn a_decimal_inside_a_record_with_time_is_still_refused_at_a_tool() {
    let errors = kinds("type Charge:\n    at: Instant\n    amount: Decimal\n\ntool record(charge: Charge) -> String\n");
    assert!(
        errors.iter().any(|kind| matches!(kind, TypeErrorKind::PendingJsonFormAtBoundary { type_name, .. } if type_name == "Decimal")),
        "got {errors:?}"
    );
}
