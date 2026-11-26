//! B06 `TimeZone` and `ZonedTime` in the checker: zone literals checked
//! against the IANA database, the zoned operator rows, and the methods.

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
fn a_string_literal_where_a_zone_is_meant_is_a_checked_zone() {
    assert_clean(
        "agent main(at: Instant) -> String:
    home: TimeZone = \"Europe/Berlin\"
    there = at.in_zone(\"Asia/Tokyo\")
    back = there.with_zone(home)
    return back.to_string() + there.time_zone().to_string()
",
    );
}

#[test]
fn an_unknown_zone_is_a_compile_error_with_a_suggestion() {
    let errors = check("agent main(at: Instant) -> ZonedTime:\n    return at.in_zone(\"Europe/Berln\")\n").errors;
    let error = errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::UnknownTimeZone { name, .. } if name == "Europe/Berln"))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    assert_eq!(error.kind.hint().as_deref(), Some("did you mean `Europe/Berlin`?"));
    let errors = kinds("agent main() -> TimeZone:\n    z: TimeZone = \"+02:00\"\n    return z\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::UnknownTimeZone { .. })), "an offset is not a zone: {errors:?}");
}

#[test]
fn a_string_without_a_zone_slot_stays_a_string() {
    let errors = kinds("agent main(name: String) -> TimeZone:\n    return name\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::ReturnTypeMismatch { .. })), "got {errors:?}");
    assert_clean("agent main(name: String) -> Bool:\n    return name.parse_time_zone().is_ok()\n");
}

#[test]
fn zoned_operators_and_methods_typecheck() {
    assert_clean(
        "agent main(start: ZonedTime, step: Duration) -> Int:
    later = start + step
    earlier = step + start - step
    span: Duration = later - earlier
    tomorrow = start.add_days(1).add_months(-1).add_years(2)
    same = tomorrow == start or start.time_zone() == later.time_zone()
    at: Instant = start.instant()
    parsed = \"2026-03-29T09:00\".parse_local_time(\"Europe/Berlin\")
    round = start.to_string().parse_zoned_time()
    return start.year() + start.month() + start.day() + start.hour() + start.minute() + start.second() + start.nanosecond() + start.weekday() + start.offset_seconds() + span.whole_hours()
",
    );
}

#[test]
fn zoned_times_compare_by_instant_not_by_order() {
    let errors = check("agent main(a: ZonedTime, b: ZonedTime) -> Bool:\n    return a < b\n").errors;
    let error = errors
        .iter()
        .find(|e| matches!(e.kind, TypeErrorKind::TimeOperatorMismatch { .. }))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    assert!(error.kind.hint().unwrap().contains("a.instant() < b.instant()"));
    let errors = kinds("agent main(a: ZonedTime) -> ZonedTime:\n    return a + 1\n");
    assert!(errors.iter().any(|kind| matches!(kind, TypeErrorKind::TimeOperatorMismatch { .. })), "got {errors:?}");
}
