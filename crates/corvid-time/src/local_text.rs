//! Text for zoned and local times.
//!
//! - RFC 9557 zoned text, `2026-03-29T03:30:00+02:00[Europe/Berlin]`, is
//!   what `ZonedTime` prints and `ZonedTime::parse` reads. The offset
//!   must be the one the zone uses at that instant.
//! - Local text, `2026-03-29T09:00` or `2026-03-29T09:00:00`, names a
//!   wall-clock time and needs a zone to become an instant. A local time
//!   the zone skips or shows twice is an error that names both offsets,
//!   so a program cannot silently book a time that does not exist.

use crate::error::TimeError;
use crate::instant::Instant;
use crate::zone::TimeZone;
use crate::zoned::ZonedTime;
use chrono::{DateTime, LocalResult, NaiveDateTime, Offset, SecondsFormat, TimeZone as _, Utc};
use chrono_tz::GapInfo;
use std::fmt;

fn offset_text(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let seconds = seconds.unsigned_abs();
    format!("{sign}{:02}:{:02}", seconds / 3600, seconds % 3600 / 60)
}

impl ZonedTime {
    /// Reads RFC 9557 zoned text. The bracketed zone decides the rules;
    /// an offset that disagrees with them is an error rather than a
    /// guess about which part the writer meant.
    pub fn parse(text: &str) -> Result<ZonedTime, TimeError> {
        let malformed = |message: &str| TimeError::Malformed { position: 0, message: message.to_string() };
        let open = text
            .find('[')
            .filter(|_| text.ends_with(']'))
            .ok_or_else(|| malformed("expected zoned text such as 2026-03-29T03:30:00+02:00[Europe/Berlin]"))?;
        let zone = TimeZone::parse(&text[open + 1..text.len() - 1])?;
        let stamp = &text[..open];
        // A local time in year 10000 (the last instant, seen east of
        // UTC) is written with a sign, which RFC 3339 has no form for.
        let written = DateTime::parse_from_rfc3339(stamp)
            .or_else(|error| DateTime::parse_from_str(stamp, "%Y-%m-%dT%H:%M:%S%.f%:z").map_err(|_| error))
            .map_err(|error| malformed(&format!("expected an RFC 3339 time before the zone ({error})")))?;
        let written_offset = written.offset().local_minus_utc();
        // The zone's offset at the instant the written offset names.
        let named = Instant::from_chrono(written.with_timezone(&Utc)).map(|instant| ZonedTime::new(instant, zone));
        if let Ok(zoned) = named {
            if i64::from(written_offset) == zoned.offset_seconds() {
                return Ok(zoned);
            }
        }
        // Before standard time, zones ran on local mean time, whose
        // offsets have seconds (Asia/Kolkata was +05:53:28). RFC 3339
        // writes offsets in whole minutes, so `Display` writes the exact
        // local time with its offset rounded to the minute. Read that back
        // as the local time in the zone whose exact offset it names.
        if let Some(zoned) = sub_minute_offset_match(written.naive_local(), written_offset, zone) {
            return Ok(zoned);
        }
        let zoned = named?;
        Err(TimeError::OffsetMismatch {
            text: text.to_string(),
            zone: zone.name(),
            expected: offset_text(zoned.offset_seconds() as i32),
        })
    }

    /// Corvid's `text.parse_zoned_time()`: trims whitespace and on
    /// failure names the refused text and the problem. Both tiers call
    /// this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<ZonedTime, String> {
        ZonedTime::parse(text.trim()).map_err(|error| format!("`{text}` is not a zoned time: {error}"))
    }

    /// Corvid's `text.parse_local_time(zone)`: trims whitespace; the
    /// error names the local time and, for a gap or a fold, both
    /// offsets. Both tiers call this.
    pub fn parse_local_user_text(text: &str, zone: TimeZone) -> Result<ZonedTime, String> {
        ZonedTime::parse_local(text.trim(), zone).map_err(|error| error.to_string())
    }

    /// Reads a wall-clock time (`2026-03-29T09:00`, seconds and a
    /// fraction optional) in `zone`.
    pub fn parse_local(text: &str, zone: TimeZone) -> Result<ZonedTime, TimeError> {
        let local = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
            .ok_or_else(|| TimeError::Malformed {
                position: 0,
                message: "expected a local date and time such as 2026-03-29T09:00, with no offset".to_string(),
            })?;
        let rules = zone.chrono_zone();
        match rules.from_local_datetime(&local) {
            LocalResult::Single(at) => Ok(ZonedTime::new(Instant::from_chrono(at.with_timezone(&Utc))?, zone)),
            LocalResult::Ambiguous(first, second) => Err(TimeError::LocalTimeRepeated {
                local: text.to_string(),
                zone: zone.name(),
                first: offset_text(first.offset().fix().local_minus_utc()),
                second: offset_text(second.offset().fix().local_minus_utc()),
            }),
            LocalResult::None => {
                let gap = GapInfo::new(&local, &rules);
                let before = gap
                    .as_ref()
                    .and_then(|gap| gap.begin)
                    .map(|(_, offset)| offset_text(offset.fix().local_minus_utc()))
                    .unwrap_or_else(|| "?".to_string());
                let after = gap
                    .as_ref()
                    .and_then(|gap| gap.end)
                    .map(|end| offset_text(end.offset().fix().local_minus_utc()))
                    .unwrap_or_else(|| "?".to_string());
                Err(TimeError::LocalTimeSkipped { local: text.to_string(), zone: zone.name(), before, after })
            }
        }
    }
}

/// The zoned time `local` names in `zone` when the zone's exact offset
/// there has seconds and `written` is that offset rounded to the
/// minute, as `Display` writes it. `None` for any other offset.
fn sub_minute_offset_match(local: NaiveDateTime, written: i32, zone: TimeZone) -> Option<ZonedTime> {
    let candidates = match zone.chrono_zone().from_local_datetime(&local) {
        LocalResult::Single(at) => vec![at],
        LocalResult::Ambiguous(first, second) => vec![first, second],
        LocalResult::None => vec![],
    };
    candidates.into_iter().find_map(|at| {
        let exact = at.offset().fix().local_minus_utc();
        (exact % 60 != 0 && (exact - written).abs() < 60)
            .then(|| Instant::from_chrono(at.with_timezone(&Utc)).ok())
            .flatten()
            .map(|instant| ZonedTime::new(instant, zone))
    })
}

/// RFC 9557: the local time with its offset, then the zone in brackets.
impl fmt::Display for ZonedTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let local = self.local();
        let stamp = local.fixed_offset().to_rfc3339_opts(SecondsFormat::AutoSi, false);
        write!(f, "{stamp}[{}]", self.zone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> TimeZone {
        TimeZone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn zoned_text_round_trips() {
        let text = "2026-03-29T03:30:00+02:00[Europe/Berlin]";
        assert_eq!(ZonedTime::parse(text).unwrap().to_string(), text);
        let utc = "2026-07-11T08:30:00.5+00:00[UTC]";
        assert_eq!(ZonedTime::parse(utc).unwrap().to_string(), "2026-07-11T08:30:00.500+00:00[UTC]");
    }

    #[test]
    fn local_mean_time_and_the_ends_of_the_range_round_trip() {
        // Local mean time offsets have seconds; the text rounds them to
        // the minute (Amsterdam was +00:19:32, Tokyo +09:18:59).
        for (instant, zone) in [
            ("1850-06-01T12:00:00Z", "Asia/Kolkata"),
            ("1850-06-01T12:00:00Z", "Europe/Amsterdam"),
            ("1850-06-01T12:00:00Z", "Asia/Tokyo"),
            ("0001-01-01T00:00:00Z", "Pacific/Kiritimati"),
            ("0001-01-01T00:00:00Z", "America/New_York"),
            ("9999-12-31T23:59:59.999999999Z", "Asia/Kolkata"),
        ] {
            let zoned = Instant::parse(instant).unwrap().in_zone(TimeZone::parse(zone).unwrap());
            let text = zoned.to_string();
            assert_eq!(ZonedTime::parse(&text), Ok(zoned), "{text}");
        }
        let last = Instant::parse("9999-12-31T23:59:59.999999999Z").unwrap().in_zone(TimeZone::parse("Asia/Tokyo").unwrap());
        assert_eq!(last.to_string(), "+10000-01-01T08:59:59.999999999+09:00[Asia/Tokyo]");
        let error = ZonedTime::parse("1850-06-01T21:18:59+09:21[Asia/Tokyo]").unwrap_err();
        assert!(matches!(error, TimeError::OffsetMismatch { .. }), "{error}");
    }

    #[test]
    fn user_text_is_trimmed_and_its_error_names_the_text() {
        assert_eq!(ZonedTime::parse_user_text(" 2026-03-29T03:30:00+02:00[Europe/Berlin] ").unwrap().to_string(), "2026-03-29T03:30:00+02:00[Europe/Berlin]");
        assert_eq!(
            ZonedTime::parse_user_text("2026-03-29").unwrap_err(),
            "`2026-03-29` is not a zoned time: expected zoned text such as 2026-03-29T03:30:00+02:00[Europe/Berlin] at position 0"
        );
        assert_eq!(
            ZonedTime::parse_local_user_text(" 2026-03-29T02:30 ", berlin()).unwrap_err(),
            "2026-03-29T02:30 does not exist in Europe/Berlin: clocks move from +01:00 to +02:00 there, skipping it"
        );
    }

    #[test]
    fn an_offset_the_zone_does_not_use_is_an_error() {
        let error = ZonedTime::parse("2026-07-11T12:00:00+01:00[Europe/Berlin]").unwrap_err();
        assert_eq!(
            error.to_string(),
            "`2026-07-11T12:00:00+01:00[Europe/Berlin]` gives an offset Europe/Berlin does not use at that time; Europe/Berlin is at +02:00 then"
        );
        assert!(ZonedTime::parse("2026-07-11T12:00:00+02:00").is_err(), "zoned text needs its zone");
    }

    #[test]
    fn local_text_resolves_in_the_zone() {
        let at = ZonedTime::parse_local("2026-03-29T09:00", berlin()).unwrap();
        assert_eq!(at.to_string(), "2026-03-29T09:00:00+02:00[Europe/Berlin]");
        let precise = ZonedTime::parse_local("2026-03-29T09:00:01.25", berlin()).unwrap();
        assert_eq!(precise.nanosecond(), 250_000_000);
    }

    #[test]
    fn a_skipped_local_time_names_both_offsets() {
        let error = ZonedTime::parse_local("2026-03-29T02:30", berlin()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "2026-03-29T02:30 does not exist in Europe/Berlin: clocks move from +01:00 to +02:00 there, skipping it"
        );
    }

    #[test]
    fn a_repeated_local_time_names_both_offsets() {
        let error = ZonedTime::parse_local("2026-10-25T02:30", berlin()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "2026-10-25T02:30 happens twice in Europe/Berlin: once at +02:00 and again at +01:00"
        );
    }

    #[test]
    fn local_text_with_an_offset_is_refused() {
        assert!(ZonedTime::parse_local("2026-03-29T09:00+02:00", berlin()).is_err());
    }
}
