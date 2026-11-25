//! Native entry points for `Duration`, `Instant`, `TimeZone` and
//! `ZonedTime` (B06).
//!
//! Native code holds each of these as a Corvid String containing its
//! canonical text, the text its JSON form carries: ISO 8601 for a
//! duration (`PT1H30M`), RFC 3339 in UTC for an instant, the IANA name
//! for a zone and RFC 9557 for a zoned time. Every operator and method
//! is one call here: the entry point parses its operands with
//! `corvid_time`, computes with `corvid_time`, the code the interpreter
//! calls, and returns canonical text or an `Int`. Order and equality
//! are by value, never by comparing text.
//!
//! Operands are borrowed; a returned String is a fresh +1 the caller
//! owns. A `TimeError` stops the program the way the interpreter's
//! arithmetic error does, with the same message.
//!
//! The operations that read a zone's rules (a local field, a calendar
//! move, RFC 9557 text, reading a local or zoned time) report
//! themselves to the runtime, exactly as the interpreter's
//! `zone_rules_use` does, so a native replay warns when the trace was
//! recorded with another tz database.

use super::native_scalar::{canonical, runtime_trap, slot_string, write_parse_result};
use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;
use corvid_time::{Duration, Instant, TimeError, TimeUnit, TimeZone, ZonedField, ZonedTime};

fn checked<T>(result: Result<T, TimeError>) -> T {
    result.unwrap_or_else(|error| runtime_trap(&error))
}

unsafe fn duration_of(value: &CorvidString) -> Duration {
    unsafe { canonical(value, "Duration", Duration::parse) }
}

unsafe fn instant_of(value: &CorvidString) -> Instant {
    unsafe { canonical(value, "Instant", Instant::parse) }
}

unsafe fn zone_of(value: &CorvidString) -> TimeZone {
    unsafe { canonical(value, "TimeZone", TimeZone::parse) }
}

unsafe fn zoned_of(value: &CorvidString) -> ZonedTime {
    unsafe { canonical(value, "ZonedTime", ZonedTime::parse) }
}

fn text_of(value: impl std::fmt::Display) -> CorvidString {
    string_from_rust(value.to_string())
}

/// Reports a zone-dependent operation to the runtime, for the replay
/// warning. A binary that never started the runtime records and
/// replays no trace, so there is nothing to report to.
fn note_zone_rules_use(operation: &str, zone: TimeZone) {
    if let Some(bridge) = super::state::bridge_if_initialized() {
        bridge.corvid_runtime().note_zone_rules_use(operation, zone.name());
    }
}

macro_rules! duration_pair {
    ($symbol:ident, $method:ident) => {
        /// # Safety
        /// Both operands must be live native Duration strings.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(left: CorvidString, right: CorvidString) -> CorvidString {
            let (a, b) = unsafe { (duration_of(&left), duration_of(&right)) };
            text_of(checked(a.$method(&b)))
        }
    };
}

duration_pair!(corvid_duration_add, checked_add);
duration_pair!(corvid_duration_sub, checked_sub);

/// `d * n` and `n * d`.
///
/// # Safety
/// The operand must be a live native Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_duration_mul(duration: CorvidString, factor: i64) -> CorvidString {
    text_of(checked(unsafe { duration_of(&duration) }.checked_mul(factor)))
}

/// `d / n`, truncated toward zero to the nanosecond.
///
/// # Safety
/// The operand must be a live native Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_duration_div(duration: CorvidString, divisor: i64) -> CorvidString {
    text_of(checked(unsafe { duration_of(&duration) }.checked_div(divisor)))
}

/// `-d`.
///
/// # Safety
/// The operand must be a live native Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_duration_negate(duration: CorvidString) -> CorvidString {
    text_of(unsafe { duration_of(&duration) }.negate())
}

/// `d.abs()`.
///
/// # Safety
/// The operand must be a live native Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_duration_abs(duration: CorvidString) -> CorvidString {
    text_of(unsafe { duration_of(&duration) }.abs())
}

/// Order by length: -1, 0 or 1.
///
/// # Safety
/// Both operands must be live native Duration strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_duration_compare(left: CorvidString, right: CorvidString) -> i64 {
    let (a, b) = unsafe { (duration_of(&left), duration_of(&right)) };
    a.cmp(&b) as i64
}

macro_rules! duration_reading {
    ($symbol:ident, $unit:expr) => {
        /// `d.whole_<unit>()`, truncated toward zero; traps when the
        /// reading is outside the `Int` range.
        ///
        /// # Safety
        /// The operand must be a live native Duration string.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(duration: CorvidString) -> i64 {
            checked(unsafe { duration_of(&duration) }.whole($unit))
        }
    };
}

duration_reading!(corvid_duration_whole_hours, TimeUnit::Hours);
duration_reading!(corvid_duration_whole_minutes, TimeUnit::Minutes);
duration_reading!(corvid_duration_whole_seconds, TimeUnit::Seconds);
duration_reading!(corvid_duration_whole_milliseconds, TimeUnit::Milliseconds);
duration_reading!(corvid_duration_whole_microseconds, TimeUnit::Microseconds);
duration_reading!(corvid_duration_whole_nanoseconds, TimeUnit::Nanoseconds);

/// `i + d` and `d + i`.
///
/// # Safety
/// The operands must be a live native Instant and Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_add_duration(instant: CorvidString, duration: CorvidString) -> CorvidString {
    let (i, d) = unsafe { (instant_of(&instant), duration_of(&duration)) };
    text_of(checked(i.checked_add(&d)))
}

/// `i - d`.
///
/// # Safety
/// The operands must be a live native Instant and Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_sub_duration(instant: CorvidString, duration: CorvidString) -> CorvidString {
    let (i, d) = unsafe { (instant_of(&instant), duration_of(&duration)) };
    text_of(checked(i.checked_sub(&d)))
}

/// `a - b` between two instants: the `Duration` from `b` to `a`.
///
/// # Safety
/// Both operands must be live native Instant strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_sub_instant(left: CorvidString, right: CorvidString) -> CorvidString {
    let (a, b) = unsafe { (instant_of(&left), instant_of(&right)) };
    text_of(a.duration_since(&b))
}

/// Order on the timeline: -1, 0 or 1. RFC 3339 text with and without
/// a fraction does not sort bytewise, so this compares the instants.
///
/// # Safety
/// Both operands must be live native Instant strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_compare(left: CorvidString, right: CorvidString) -> i64 {
    let (a, b) = unsafe { (instant_of(&left), instant_of(&right)) };
    a.cmp(&b) as i64
}

/// `i.epoch_milliseconds()`, rounded toward negative infinity.
///
/// # Safety
/// The operand must be a live native Instant string.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_epoch_milliseconds(instant: CorvidString) -> i64 {
    unsafe { instant_of(&instant) }.epoch_milliseconds()
}

/// `Instant.from_epoch_milliseconds(n)`; traps outside the instant range.
#[no_mangle]
pub extern "C" fn corvid_instant_from_epoch_milliseconds(milliseconds: i64) -> CorvidString {
    text_of(checked(Instant::from_epoch_milliseconds(milliseconds)))
}

/// `i.in_zone(zone)`.
///
/// # Safety
/// The operands must be a live native Instant and TimeZone string.
#[no_mangle]
pub unsafe extern "C" fn corvid_instant_in_zone(instant: CorvidString, zone: CorvidString) -> CorvidString {
    let (i, z) = unsafe { (instant_of(&instant), zone_of(&zone)) };
    text_of(i.in_zone(z))
}

/// `z + d` and `d + z`: a move along the timeline.
///
/// # Safety
/// The operands must be a live native ZonedTime and Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_add_duration(zoned: CorvidString, duration: CorvidString) -> CorvidString {
    let (z, d) = unsafe { (zoned_of(&zoned), duration_of(&duration)) };
    text_of(checked(z.checked_add(&d)))
}

/// `z - d`.
///
/// # Safety
/// The operands must be a live native ZonedTime and Duration string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_sub_duration(zoned: CorvidString, duration: CorvidString) -> CorvidString {
    let (z, d) = unsafe { (zoned_of(&zoned), duration_of(&duration)) };
    text_of(checked(z.checked_sub(&d)))
}

/// `a - b` between two zoned times: the `Duration` between their instants.
///
/// # Safety
/// Both operands must be live native ZonedTime strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_sub_zoned_time(left: CorvidString, right: CorvidString) -> CorvidString {
    let (a, b) = unsafe { (zoned_of(&left), zoned_of(&right)) };
    text_of(a.instant().duration_since(&b.instant()))
}

/// `a == b` between two zoned times: 1 for the same instant in the same
/// zone, 0 otherwise.
///
/// # Safety
/// Both operands must be live native ZonedTime strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_equal(left: CorvidString, right: CorvidString) -> i64 {
    i64::from(unsafe { zoned_of(&left) == zoned_of(&right) })
}

/// `a == b` between two zones: 1 or 0.
///
/// # Safety
/// Both operands must be live native TimeZone strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_time_zone_equal(left: CorvidString, right: CorvidString) -> i64 {
    i64::from(unsafe { zone_of(&left) == zone_of(&right) })
}

macro_rules! zoned_field {
    ($symbol:ident, $field:expr) => {
        /// A local field of a zoned time, read from the zone's rules.
        ///
        /// # Safety
        /// The operand must be a live native ZonedTime string.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(zoned: CorvidString) -> i64 {
            let z = unsafe { zoned_of(&zoned) };
            note_zone_rules_use($field.name(), z.zone());
            z.field($field)
        }
    };
}

zoned_field!(corvid_zoned_time_year, ZonedField::Year);
zoned_field!(corvid_zoned_time_month, ZonedField::Month);
zoned_field!(corvid_zoned_time_day, ZonedField::Day);
zoned_field!(corvid_zoned_time_hour, ZonedField::Hour);
zoned_field!(corvid_zoned_time_minute, ZonedField::Minute);
zoned_field!(corvid_zoned_time_second, ZonedField::Second);
zoned_field!(corvid_zoned_time_nanosecond, ZonedField::Nanosecond);
zoned_field!(corvid_zoned_time_weekday, ZonedField::Weekday);
zoned_field!(corvid_zoned_time_offset_seconds, ZonedField::OffsetSeconds);

/// `z.time_zone()`.
///
/// # Safety
/// The operand must be a live native ZonedTime string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_zone(zoned: CorvidString) -> CorvidString {
    text_of(unsafe { zoned_of(&zoned) }.zone())
}

/// `z.instant()`.
///
/// # Safety
/// The operand must be a live native ZonedTime string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_instant(zoned: CorvidString) -> CorvidString {
    text_of(unsafe { zoned_of(&zoned) }.instant())
}

macro_rules! zoned_calendar_move {
    ($symbol:ident, $method:ident, $operation:literal) => {
        /// The same local time on another calendar date.
        ///
        /// # Safety
        /// The operand must be a live native ZonedTime string.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(zoned: CorvidString, count: i64) -> CorvidString {
            let z = unsafe { zoned_of(&zoned) };
            note_zone_rules_use($operation, z.zone());
            text_of(checked(z.$method(count)))
        }
    };
}

zoned_calendar_move!(corvid_zoned_time_add_days, add_days, "add_days");
zoned_calendar_move!(corvid_zoned_time_add_months, add_months, "add_months");
zoned_calendar_move!(corvid_zoned_time_add_years, add_years, "add_years");

/// `z.with_zone(zone)`: the same instant in another zone.
///
/// # Safety
/// The operands must be a live native ZonedTime and TimeZone string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_with_zone(zoned: CorvidString, zone: CorvidString) -> CorvidString {
    let (z, zone) = unsafe { (zoned_of(&zoned), zone_of(&zone)) };
    text_of(z.with_zone(zone))
}

/// `z.to_string()`: RFC 9557 text, which writes the local time and
/// offset the zone's rules give.
///
/// # Safety
/// The operand must be a live native ZonedTime string.
#[no_mangle]
pub unsafe extern "C" fn corvid_zoned_time_to_string(zoned: CorvidString) -> CorvidString {
    let z = unsafe { zoned_of(&zoned) };
    note_zone_rules_use("to_string", z.zone());
    text_of(z)
}

/// `text.parse_duration()` into a native `Result<Duration, String>`.
///
/// # Safety
/// The receiver must remain a live Corvid string for this call, and
/// `payload` must point to a writable native Result payload slot.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_duration(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    unsafe { write_parse_result(Duration::parse_user_text(text).map(|d| d.to_string()), payload) }
}

/// `text.parse_instant()` into a native `Result<Instant, String>`.
///
/// # Safety
/// As for [`corvid_string_parse_duration`].
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_instant(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    unsafe { write_parse_result(Instant::parse_user_text(text).map(|i| i.to_string()), payload) }
}

/// `text.parse_time_zone()` into a native `Result<TimeZone, String>`.
///
/// # Safety
/// As for [`corvid_string_parse_duration`].
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_time_zone(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    unsafe { write_parse_result(TimeZone::parse_user_text(text).map(|z| z.to_string()), payload) }
}

/// `text.parse_zoned_time()` into a native `Result<ZonedTime, String>`.
/// Reading the text checks its offset against the zone's rules, so a
/// successful read reports the zone, as the interpreter does.
///
/// # Safety
/// As for [`corvid_string_parse_duration`].
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_zoned_time(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    let parsed = ZonedTime::parse_user_text(text);
    if let Ok(zoned) = &parsed {
        note_zone_rules_use("parse_zoned_time", zoned.zone());
    }
    unsafe { write_parse_result(parsed.map(|z| z.to_string()), payload) }
}

/// `text.parse_local_time(zone)` into a native `Result<ZonedTime, String>`.
///
/// # Safety
/// As for [`corvid_string_parse_duration`]; `zone` must be a live
/// native TimeZone string.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_local_time(
    text: CorvidString,
    zone: CorvidString,
    payload: *mut CorvidString,
) -> i64 {
    let (text, zone) = unsafe { (borrow_corvid_string(&text), zone_of(&zone)) };
    note_zone_rules_use("parse_local_time", zone);
    unsafe { write_parse_result(ZonedTime::parse_local_user_text(text, zone).map(|z| z.to_string()), payload) }
}

/// Whether two native Duration slots hold the same length, for
/// structural equality of records, lists and options.
///
/// # Safety
/// Both slots must hold live native Duration strings.
pub(crate) unsafe fn duration_slots_equal(left: u64, right: u64) -> bool {
    unsafe { duration_of(&slot_string(left)) == duration_of(&slot_string(right)) }
}

/// Whether two native Instant slots hold the same instant.
///
/// # Safety
/// Both slots must hold live native Instant strings.
pub(crate) unsafe fn instant_slots_equal(left: u64, right: u64) -> bool {
    unsafe { instant_of(&slot_string(left)) == instant_of(&slot_string(right)) }
}

/// Whether two native TimeZone slots hold the same zone.
///
/// # Safety
/// Both slots must hold live native TimeZone strings.
pub(crate) unsafe fn time_zone_slots_equal(left: u64, right: u64) -> bool {
    unsafe { zone_of(&slot_string(left)) == zone_of(&slot_string(right)) }
}

/// Whether two native ZonedTime slots hold the same instant in the same zone.
///
/// # Safety
/// Both slots must hold live native ZonedTime strings.
pub(crate) unsafe fn zoned_time_slots_equal(left: u64, right: u64) -> bool {
    unsafe { zoned_of(&slot_string(left)) == zoned_of(&slot_string(right)) }
}
