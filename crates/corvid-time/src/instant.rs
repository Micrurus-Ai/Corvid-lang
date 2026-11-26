//! `Instant`: an exact point on the UTC timeline, and its RFC 3339 text.

use crate::duration::Duration;
use crate::error::TimeError;
use chrono::{DateTime, SecondsFormat, Utc};
use std::fmt;

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// 0001-01-01T00:00:00Z, in nanoseconds since the Unix epoch.
pub(crate) const MIN_NANOSECONDS: i128 = -62_135_596_800 * NANOS_PER_SECOND;
/// 9999-12-31T23:59:59.999999999Z, in nanoseconds since the Unix epoch.
pub(crate) const MAX_NANOSECONDS: i128 = 253_402_300_799 * NANOS_PER_SECOND + 999_999_999;

/// An exact point in time: nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Instant {
    nanoseconds: i128,
}

impl Instant {
    pub const UNIX_EPOCH: Instant = Instant { nanoseconds: 0 };

    pub fn from_nanoseconds(nanoseconds: i128) -> Result<Instant, TimeError> {
        if (MIN_NANOSECONDS..=MAX_NANOSECONDS).contains(&nanoseconds) {
            Ok(Instant { nanoseconds })
        } else {
            Err(TimeError::OutOfRange { what: "instant" })
        }
    }

    pub fn nanoseconds_since_epoch(&self) -> i128 {
        self.nanoseconds
    }

    pub fn from_epoch_milliseconds(milliseconds: i64) -> Result<Instant, TimeError> {
        Instant::from_nanoseconds(i128::from(milliseconds) * 1_000_000)
    }

    /// Whole milliseconds since the epoch, rounded toward negative
    /// infinity, so an instant before 1970 reads as the millisecond it
    /// falls in, as JavaScript's `Date` and Temporal do.
    pub fn epoch_milliseconds(&self) -> i64 {
        // The instant range keeps this far inside i64.
        self.nanoseconds.div_euclid(1_000_000) as i64
    }

    pub fn checked_add(&self, duration: &Duration) -> Result<Instant, TimeError> {
        Instant::from_nanoseconds(self.nanoseconds + duration.nanoseconds())
    }

    pub fn checked_sub(&self, duration: &Duration) -> Result<Instant, TimeError> {
        Instant::from_nanoseconds(self.nanoseconds - duration.nanoseconds())
    }

    /// The exact time from `earlier` to `self`; negative when `earlier`
    /// is later. Always in range, because every span between two
    /// instants is.
    pub fn duration_since(&self, earlier: &Instant) -> Duration {
        Duration::from_nanoseconds(self.nanoseconds - earlier.nanoseconds)
            .expect("the span between two instants is a duration")
    }

    /// Reads RFC 3339 text with any offset (`2026-03-29T01:30:00+01:00`)
    /// as the instant it names.
    pub fn parse(text: &str) -> Result<Instant, TimeError> {
        let parsed = DateTime::parse_from_rfc3339(text).map_err(|error| TimeError::Malformed {
            position: 0,
            message: format!("expected an RFC 3339 instant such as 2026-07-11T08:30:00Z ({error})"),
        })?;
        let utc = parsed.with_timezone(&Utc);
        Instant::from_nanoseconds(
            i128::from(utc.timestamp()) * NANOS_PER_SECOND + i128::from(utc.timestamp_subsec_nanos()),
        )
    }

    /// Corvid's `text.parse_instant()`: trims whitespace and on
    /// failure names the refused text and the problem. Both tiers call
    /// this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<Instant, String> {
        Instant::parse(text.trim()).map_err(|error| format!("`{text}` is not an instant: {error}"))
    }

    pub(crate) fn from_chrono(at: DateTime<Utc>) -> Result<Instant, TimeError> {
        Instant::from_nanoseconds(i128::from(at.timestamp()) * NANOS_PER_SECOND + i128::from(at.timestamp_subsec_nanos()))
    }

    /// Seen in `zone`.
    pub fn in_zone(&self, zone: crate::zone::TimeZone) -> crate::zoned::ZonedTime {
        crate::zoned::ZonedTime::new(*self, zone)
    }

    pub(crate) fn to_chrono(self) -> DateTime<Utc> {
        let seconds = self.nanoseconds.div_euclid(NANOS_PER_SECOND) as i64;
        let nanos = self.nanoseconds.rem_euclid(NANOS_PER_SECOND) as u32;
        DateTime::from_timestamp(seconds, nanos).expect("the instant range is inside chrono's range")
    }
}

/// RFC 3339 in UTC with a trailing `Z`, with no fraction for a whole
/// second and otherwise 3, 6 or 9 fractional digits, the fewest that
/// show the value exactly.
impl Instant {
    /// RFC 3339 in UTC with all nine fractional digits, such as
    /// `2026-03-29T01:30:00.000000000Z`. Every instant in range has a
    /// four-digit year, so this text sorts in time order and equal
    /// instants have equal text: the form a table column stores.
    /// [`Instant::parse`] reads it back. If the range ever reaches a
    /// negative or five-digit year, this form needs a sign-aware year
    /// encoding; a test pins it to the range's ends.
    pub fn to_sortable_text(&self) -> String {
        self.to_chrono().to_rfc3339_opts(SecondsFormat::Nanos, true)
    }
}

impl fmt::Display for Instant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_chrono().to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unit::TimeUnit;

    #[test]
    fn user_text_is_trimmed_and_its_error_names_the_text() {
        assert_eq!(Instant::parse_user_text(" 2026-07-11T08:30:00Z ").unwrap().to_string(), "2026-07-11T08:30:00Z");
        assert!(Instant::parse_user_text("July").unwrap_err().starts_with("`July` is not an instant: expected an RFC 3339 instant"));
    }

    fn at(text: &str) -> Instant {
        Instant::parse(text).unwrap()
    }

    #[test]
    fn text_round_trips_in_utc() {
        assert_eq!(at("2026-07-11T08:30:00Z").to_string(), "2026-07-11T08:30:00Z");
        assert_eq!(at("2026-03-29T03:30:00+02:00").to_string(), "2026-03-29T01:30:00Z");
        assert_eq!(at("2026-07-11T08:30:00.5Z").to_string(), "2026-07-11T08:30:00.500Z");
        assert_eq!(at("2026-07-11T08:30:00.000000001Z").to_string(), "2026-07-11T08:30:00.000000001Z");
        assert_eq!(at("1969-12-31T23:59:59.999Z").to_string(), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn sortable_text_orders_as_time_does_and_reads_back() {
        let instants = [
            "0001-01-01T00:00:00Z",
            "1969-12-31T23:59:59.999Z",
            "1970-01-01T00:00:00Z",
            "2026-07-11T08:30:00.000000001Z",
            "2026-07-11T08:30:00.5Z",
            "2026-07-11T08:30:01Z",
            "9999-12-31T23:59:59.999999999Z",
        ]
        .map(at);
        let texts = instants.map(|i| i.to_sortable_text());
        assert_eq!(texts[4], "2026-07-11T08:30:00.500000000Z");
        assert!(texts.windows(2).all(|pair| pair[0] < pair[1]), "{texts:?}");
        for (instant, text) in instants.iter().zip(&texts) {
            assert_eq!(text.len(), 30, "{text}");
            assert_eq!(&Instant::parse(text).unwrap(), instant);
        }
        assert_eq!(at("2026-03-29T03:30:00+02:00").to_sortable_text(), at("2026-03-29T01:30:00Z").to_sortable_text());

        // The range's own ends, so widening it fails here first.
        let first = Instant::from_nanoseconds(MIN_NANOSECONDS).unwrap().to_sortable_text();
        let last = Instant::from_nanoseconds(MAX_NANOSECONDS).unwrap().to_sortable_text();
        assert_eq!(first, "0001-01-01T00:00:00.000000000Z");
        assert_eq!(last, "9999-12-31T23:59:59.999999999Z");
        assert!(first < texts[1] && texts[texts.len() - 2] < last);
    }

    #[test]
    fn the_range_is_what_rfc_3339_can_write() {
        assert_eq!(at("0001-01-01T00:00:00Z").to_string(), "0001-01-01T00:00:00Z");
        assert_eq!(at("9999-12-31T23:59:59.999999999Z").to_string(), "9999-12-31T23:59:59.999999999Z");
        let last = at("9999-12-31T23:59:59.999999999Z");
        assert_eq!(
            last.checked_add(&Duration::of(1, TimeUnit::Nanoseconds).unwrap()),
            Err(TimeError::OutOfRange { what: "instant" })
        );
    }

    #[test]
    fn malformed_text_is_an_error() {
        let error = Instant::parse("2026-07-11 08:30").unwrap_err().to_string();
        assert!(error.starts_with("expected an RFC 3339 instant"), "{error}");
        assert!(Instant::parse("2026-07-11T08:30:00").is_err(), "an instant needs an offset");
    }

    #[test]
    fn epoch_milliseconds_round_toward_negative_infinity() {
        assert_eq!(Instant::from_epoch_milliseconds(1_000).unwrap().to_string(), "1970-01-01T00:00:01Z");
        assert_eq!(at("1969-12-31T23:59:59.9995Z").epoch_milliseconds(), -1);
        assert_eq!(at("2026-07-11T08:30:00.0129Z").epoch_milliseconds(), 1_783_758_600_012);
    }

    #[test]
    fn arithmetic_with_durations() {
        let start = at("2026-07-11T08:30:00Z");
        let later = start.checked_add(&Duration::of(90, TimeUnit::Minutes).unwrap()).unwrap();
        assert_eq!(later.to_string(), "2026-07-11T10:00:00Z");
        assert_eq!(later.duration_since(&start).to_string(), "PT1H30M");
        assert_eq!(start.duration_since(&later).to_string(), "-PT1H30M");
        assert!(start < later);
    }
}
