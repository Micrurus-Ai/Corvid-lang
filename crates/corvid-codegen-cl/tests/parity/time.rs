//! `Duration`, `Instant`, `TimeZone` and `ZonedTime` on the native
//! tier. Every program runs on the interpreter and as a compiled
//! binary; both must return the same text, and the binary must release
//! every allocation. Traps must stop both tiers with the same
//! `corvid-time` message.

use super::scalar_harness::{assert_parity_text, assert_parity_trap};

/// A helper agent that writes a list of checks as `t` and `f`.
const FLAGS: &str = r#"
agent flags(checks: List<Bool>) -> String:
    text = ""
    for check in checks:
        if check:
            text = text + "t"
        else:
            text = text + "f"
    return text
"#;

#[test]
fn duration_and_instant_arithmetic_and_text() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    start = Instant.from_epoch_milliseconds(1783758600012)
    hour = match "PT1H".parse_duration():
        Ok(d) -> d
        Err(e) -> start - start
    half = hour / 2
    three = 3 * half
    later = start + three
    earlier = half + start
    back = later - hour * 2
    span = later - start
    negative = -span
    return hour.to_string() + " " + half.to_string() + " " + three.to_string() + " " + later.to_string() + " " + earlier.to_string() + " " + back.to_string() + " " + span.to_string() + " " + negative.to_string() + " " + negative.abs().to_string() + " " + (hour - three).to_string()
"#,
    );
    assert_eq!(
        out,
        "PT1H PT30M PT1H30M 2026-07-11T10:00:00.012Z 2026-07-11T09:00:00.012Z 2026-07-11T08:00:00.012Z PT1H30M -PT1H30M PT1H30M -PT30M"
    );
}

#[test]
fn instants_order_by_time_whatever_their_text() {
    // "…08:30:00.500Z" sorts before "…08:30:00Z" bytewise, so ordering
    // by text would get these backwards.
    let src = format!(
        r#"{FLAGS}
agent main() -> String:
    whole = Instant.from_epoch_milliseconds(1783758600000)
    half = Instant.from_epoch_milliseconds(1783758600500)
    short: Duration = half - whole
    long: Duration = (half - whole) * 4
    return flags([half > whole, whole < half, half >= whole, whole <= whole, half == half, half != whole, short < long, long > short, short == short, long != short, half - whole == short]) + " " + half.to_string()
"#
    );
    assert_eq!(assert_parity_text(&src), "ttttttttttt 2026-07-11T08:30:00.500Z");
}

#[test]
fn epoch_milliseconds_round_trip_both_ways_including_negative() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    before = Instant.from_epoch_milliseconds(-1)
    epoch = Instant.from_epoch_milliseconds(0)
    later = Instant.from_epoch_milliseconds(1783758600012)
    nudge = "PT0.0005S".parse_duration()
    d = match nudge:
        Ok(d) -> d
        Err(e) -> epoch - epoch
    readings = [later.epoch_milliseconds(), before.epoch_milliseconds(), (epoch - d).epoch_milliseconds()]
    return before.to_string() + " " + epoch.to_string() + " " + readings.to_json() + " " + (epoch - d).to_string()
"#,
    );
    assert_eq!(
        out,
        "1969-12-31T23:59:59.999Z 1970-01-01T00:00:00Z [1783758600012,-1,-1] 1969-12-31T23:59:59.999500Z"
    );
}

#[test]
fn whole_readings_truncate_toward_zero() {
    let out = assert_parity_text(
        r#"
agent show(d: Duration) -> String:
    return [d.whole_hours(), d.whole_minutes(), d.whole_seconds(), d.whole_milliseconds(), d.whole_microseconds(), d.whole_nanoseconds()].to_json()

agent main() -> String:
    return match "PT1H59M59.999999999S".parse_duration():
        Ok(d) -> show(d) + " " + show(-d)
        Err(e) -> e
"#,
    );
    assert_eq!(
        out,
        "[1,119,7199,7199999,7199999999,7199999999999] [-1,-119,-7199,-7199999,-7199999999,-7199999999999]"
    );
}

#[test]
fn every_time_parser_returns_the_same_ok_and_err_on_both_tiers() {
    let out = assert_parity_text(
        r#"
agent duration(r: Result<Duration, String>) -> String:
    return match r:
        Ok(v) -> "ok:" + v.to_string()
        Err(e) -> "err:" + e

agent instant(r: Result<Instant, String>) -> String:
    return match r:
        Ok(v) -> "ok:" + v.to_string()
        Err(e) -> "err:" + e

agent zone_text(r: Result<TimeZone, String>) -> String:
    return match r:
        Ok(v) -> "ok:" + v.to_string()
        Err(e) -> "err:" + e

agent zoned(r: Result<ZonedTime, String>) -> String:
    return match r:
        Ok(v) -> "ok:" + v.to_string()
        Err(e) -> "err:" + e

agent main() -> String:
    berlin: TimeZone = "Europe/Berlin"
    return duration(" PT1H30M ".parse_duration()) + "|" + duration("P1D".parse_duration()) + "|" + instant("2026-03-29T01:30:00+01:00".parse_instant()) + "|" + instant("July".parse_instant()) + "|" + zone_text(" Europe/Berlin ".parse_time_zone()) + "|" + zone_text("Europe/Berln".parse_time_zone()) + "|" + zoned("2026-03-29T03:30:00+02:00[Europe/Berlin]".parse_zoned_time()) + "|" + zoned("2026-07-11T12:00:00+01:00[Europe/Berlin]".parse_zoned_time()) + "|" + zoned("2026-03-29T09:00".parse_local_time(berlin)) + "|" + zoned("2026-03-29 09:00".parse_local_time(berlin))
"#,
    );
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "ok:PT1H30M");
    assert!(parts[1].starts_with("err:`P1D` is not a duration: a day is not a fixed length of time"), "{out}");
    assert_eq!(parts[2], "ok:2026-03-29T00:30:00Z");
    assert!(parts[3].starts_with("err:`July` is not an instant: expected an RFC 3339 instant"), "{out}");
    assert_eq!(parts[4], "ok:Europe/Berlin");
    assert_eq!(parts[5], "err:`Europe/Berln` is not a time zone in the IANA database; did you mean `Europe/Berlin`?");
    assert_eq!(parts[6], "ok:2026-03-29T03:30:00+02:00[Europe/Berlin]");
    assert_eq!(
        parts[7],
        "err:`2026-07-11T12:00:00+01:00[Europe/Berlin]` is not a zoned time: `2026-07-11T12:00:00+01:00[Europe/Berlin]` gives an offset Europe/Berlin does not use at that time; Europe/Berlin is at +02:00 then"
    );
    assert_eq!(parts[8], "ok:2026-03-29T09:00:00+02:00[Europe/Berlin]");
    assert!(parts[9].starts_with("err:expected a local date and time such as 2026-03-29T09:00"), "{out}");
}

#[test]
fn a_calendar_day_across_the_berlin_spring_change_is_23_hours() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    saturday = match "2026-03-28T09:00".parse_local_time("Europe/Berlin"):
        Ok(z) -> z
        Err(e) -> Instant.from_epoch_milliseconds(0).in_zone("UTC")
    next = saturday.add_days(1)
    day = match "PT24H".parse_duration():
        Ok(d) -> saturday + d
        Err(e) -> saturday
    return saturday.to_string() + " " + next.to_string() + " " + (next - saturday).to_string() + " " + day.to_string() + " " + [day.hour(), next.offset_seconds(), saturday.offset_seconds()].to_json()
"#,
    );
    assert_eq!(
        out,
        "2026-03-28T09:00:00+01:00[Europe/Berlin] 2026-03-29T09:00:00+02:00[Europe/Berlin] PT23H 2026-03-29T10:00:00+02:00[Europe/Berlin] [10,7200,3600]"
    );
}

#[test]
fn skipped_and_repeated_local_times_are_errors_naming_both_offsets() {
    let out = assert_parity_text(
        r#"
agent read(text: String) -> String:
    return match text.parse_local_time("Europe/Berlin"):
        Ok(z) -> z.to_string()
        Err(e) -> e

agent main() -> String:
    return read("2026-03-29T02:30") + "|" + read("2026-10-25T02:30")
"#,
    );
    assert_eq!(
        out,
        "2026-03-29T02:30 does not exist in Europe/Berlin: clocks move from +01:00 to +02:00 there, skipping it|2026-10-25T02:30 happens twice in Europe/Berlin: once at +02:00 and again at +01:00"
    );
}

#[test]
fn calendar_moves_clamp_to_the_end_of_the_month_and_read_local_fields() {
    let out = assert_parity_text(
        r#"
agent fields(z: ZonedTime) -> String:
    return [z.year(), z.month(), z.day(), z.hour(), z.minute(), z.second(), z.nanosecond(), z.weekday(), z.offset_seconds()].to_json()

agent main() -> String:
    return match "2026-01-31T10:15:30.25".parse_local_time("Europe/Berlin"):
        Ok(z) -> fields(z) + " | " + z.add_months(1).to_string() + " | " + z.add_months(1).add_years(2).to_string() + " | " + z.add_years(2).add_months(1).to_string() + " | " + z.add_days(-31).to_string() + " | " + z.add_months(-13).to_string()
        Err(e) -> e
"#,
    );
    assert_eq!(
        out,
        "[2026,1,31,10,15,30,250000000,6,3600] | 2026-02-28T10:15:30.250+01:00[Europe/Berlin] | 2028-02-28T10:15:30.250+01:00[Europe/Berlin] | 2028-02-29T10:15:30.250+01:00[Europe/Berlin] | 2025-12-31T10:15:30.250+01:00[Europe/Berlin] | 2024-12-31T10:15:30.250+01:00[Europe/Berlin]"
    );
}

#[test]
fn with_zone_keeps_the_instant_and_zoned_equality_needs_the_same_zone() {
    let src = format!(
        r#"{FLAGS}
agent main() -> String:
    at = Instant.from_epoch_milliseconds(1774684800000)
    berlin = at.in_zone("Europe/Berlin")
    tokyo = berlin.with_zone("Asia/Tokyo")
    again = tokyo.with_zone("Europe/Berlin")
    home = berlin.time_zone()
    named: TimeZone = "Europe/Berlin"
    return flags([berlin == again, berlin != tokyo, berlin.instant() == tokyo.instant(), home == named, home != tokyo.time_zone(), tokyo - berlin == at - at]) + " " + tokyo.to_string() + " " + home.to_string() + " " + tokyo.instant().to_string()
"#
    );
    assert_eq!(
        assert_parity_text(&src),
        "tttttt 2026-03-28T17:00:00+09:00[Asia/Tokyo] Europe/Berlin 2026-03-28T08:00:00Z"
    );
}

#[test]
fn records_lists_and_options_of_time_values_compare_by_value() {
    let src = format!(
        r#"{FLAGS}
type Slot:
    label: String
    at: Instant
    length: Duration
    local: ZonedTime
    home: TimeZone

agent main() -> String:
    a = Instant.from_epoch_milliseconds(1783758600500)
    hour = match "PT1H".parse_duration():
        Ok(d) -> d
        Err(e) -> a - a
    minutes = match "PT60M".parse_duration():
        Ok(d) -> d
        Err(e) -> a - a
    first = Slot("standup", a, hour, a.in_zone("Europe/Berlin"), "Europe/Berlin")
    same = Slot("standup", a + (hour - minutes), minutes, a.in_zone("Europe/Berlin"), "Europe/Berlin")
    other = Slot("standup", a, hour, a.in_zone("Europe/Paris"), "Europe/Berlin")
    xs: List<Instant> = [a, a + hour]
    ys: List<Instant> = [a + hour - hour, a + minutes]
    maybe: Option<Duration> = Some(hour)
    return flags([first == same, first != other, xs == ys, maybe == Some(minutes), xs.first() == ys.first()])
"#
    );
    assert_eq!(assert_parity_text(&src), "ttttt");
}

#[test]
fn json_carries_time_values_as_their_text_both_ways() {
    let out = assert_parity_text(
        r#"
type History:
    moments: List<ZonedTime>

type Booking:
    at: Instant
    length: Duration
    local: ZonedTime
    home: TimeZone

agent main() -> String:
    at = Instant.from_epoch_milliseconds(1783758600500)
    length = at - Instant.from_epoch_milliseconds(1783753200000)
    booking = Booking(at, length, at.in_zone("Asia/Kolkata"), "Asia/Kolkata")
    text = booking.to_json()
    back = match Booking.from_json(text):
        Ok(b) -> b.local.to_string() + " " + b.length.to_string()
        Err(e) -> e.path + ": " + e.found
    refused = match Booking.from_json("{\"at\": \"tomorrow\", \"length\": \"PT1H\", \"local\": \"2026-07-11T14:00:00+05:30[Asia/Kolkata]\", \"home\": \"Asia/Kolkata\"}"):
        Ok(b) -> b.at.to_string()
        Err(e) -> e.path + " " + e.expected
    old = Instant.from_epoch_milliseconds(-3818448000000).in_zone("Asia/Kolkata")
    history = History([old])
    old_back = match History.from_json(history.to_json()):
        Ok(h) -> h.to_json()
        Err(e) -> e.path + ": " + e.found
    return text + " | " + back + " | " + refused + " | " + old_back
"#,
    );
    assert_eq!(
        out,
        r#"{"at":"2026-07-11T08:30:00.500Z","home":"Asia/Kolkata","length":"PT1H30M0.5S","local":"2026-07-11T14:00:00.500+05:30[Asia/Kolkata]"} | 2026-07-11T14:00:00.500+05:30[Asia/Kolkata] PT1H30M0.5S | at Instant | {"moments":["1848-12-31T05:53:28+05:53[Asia/Kolkata]"]}"#
    );
}

#[test]
fn time_values_print_as_entry_results() {
    assert_eq!(
        assert_parity_text(
            r#"
agent main() -> ZonedTime:
    return Instant.from_epoch_milliseconds(1774684800000).in_zone("Europe/Berlin").add_days(1)
"#
        ),
        "2026-03-29T09:00:00+02:00[Europe/Berlin]"
    );
    assert_eq!(
        assert_parity_text(
            r#"
agent main() -> Duration:
    return Instant.from_epoch_milliseconds(90000) - Instant.from_epoch_milliseconds(0)
"#
        ),
        "PT1M30S"
    );
}

#[test]
fn an_instant_outside_the_range_traps_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Instant:
    return Instant.from_epoch_milliseconds(9223372036854775807)
"#,
        "the instant is outside the range Corvid time holds",
    );
    assert_parity_trap(
        r#"
agent main() -> Instant:
    start = Instant.from_epoch_milliseconds(253402300799000)
    second = Instant.from_epoch_milliseconds(1000) - Instant.from_epoch_milliseconds(0)
    return start + second
"#,
        "the instant is outside the range Corvid time holds",
    );
}

#[test]
fn division_by_zero_and_oversized_readings_trap_with_the_interpreter_message() {
    assert_parity_trap(
        r#"
agent main() -> Duration:
    span = Instant.from_epoch_milliseconds(1000) - Instant.from_epoch_milliseconds(0)
    zero = 0
    return span / zero
"#,
        "Duration division by zero",
    );
    assert_parity_trap(
        r#"
agent main() -> Int:
    span = Instant.from_epoch_milliseconds(253402300799000) - Instant.from_epoch_milliseconds(-62135596800000)
    return span.whole_nanoseconds()
"#,
        "the reading is outside the Int range",
    );
}
