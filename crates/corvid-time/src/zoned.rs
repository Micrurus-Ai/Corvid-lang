//! `ZonedTime`: an instant seen in a time zone, and calendar arithmetic.

use crate::duration::Duration;
use crate::error::TimeError;
use crate::instant::Instant;
use crate::zone::TimeZone;
use chrono::{DateTime, Datelike, LocalResult, Months, NaiveDateTime, Offset, TimeZone as _, Timelike, Utc};
use chrono_tz::{GapInfo, Tz};

/// A local field of a `ZonedTime`, read with a method of the same name
/// (`hour()`, `offset_seconds()`). The checker's method table and both
/// tiers read the names from this one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ZonedField {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Nanosecond,
    Weekday,
    OffsetSeconds,
}

impl ZonedField {
    pub const ALL: [ZonedField; 9] = [
        ZonedField::Year,
        ZonedField::Month,
        ZonedField::Day,
        ZonedField::Hour,
        ZonedField::Minute,
        ZonedField::Second,
        ZonedField::Nanosecond,
        ZonedField::Weekday,
        ZonedField::OffsetSeconds,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ZonedField::Year => "year",
            ZonedField::Month => "month",
            ZonedField::Day => "day",
            ZonedField::Hour => "hour",
            ZonedField::Minute => "minute",
            ZonedField::Second => "second",
            ZonedField::Nanosecond => "nanosecond",
            ZonedField::Weekday => "weekday",
            ZonedField::OffsetSeconds => "offset_seconds",
        }
    }

    pub fn from_name(name: &str) -> Option<ZonedField> {
        ZonedField::ALL.into_iter().find(|field| field.name() == name)
    }
}

/// An instant and the zone it is seen in. Every instant has exactly
/// one local time in every zone, so building one never fails; the
/// local date, time and offset are read from the zone's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZonedTime {
    instant: Instant,
    zone: TimeZone,
}

impl ZonedTime {
    pub fn new(instant: Instant, zone: TimeZone) -> ZonedTime {
        ZonedTime { instant, zone }
    }

    pub fn instant(&self) -> Instant {
        self.instant
    }

    pub fn zone(&self) -> TimeZone {
        self.zone
    }

    pub(crate) fn local(&self) -> DateTime<Tz> {
        self.instant.to_chrono().with_timezone(&self.zone.chrono_zone())
    }

    pub fn year(&self) -> i64 {
        i64::from(self.local().year())
    }
    pub fn month(&self) -> i64 {
        i64::from(self.local().month())
    }
    pub fn day(&self) -> i64 {
        i64::from(self.local().day())
    }
    pub fn hour(&self) -> i64 {
        i64::from(self.local().hour())
    }
    pub fn minute(&self) -> i64 {
        i64::from(self.local().minute())
    }
    pub fn second(&self) -> i64 {
        i64::from(self.local().second())
    }
    pub fn nanosecond(&self) -> i64 {
        i64::from(self.local().nanosecond())
    }
    /// 1 for Monday through 7 for Sunday, as ISO 8601 numbers them.
    pub fn weekday(&self) -> i64 {
        i64::from(self.local().weekday().number_from_monday())
    }
    /// The offset from UTC in force at this instant, in seconds.
    pub fn offset_seconds(&self) -> i64 {
        i64::from(self.local().offset().fix().local_minus_utc())
    }

    pub fn field(&self, field: ZonedField) -> i64 {
        match field {
            ZonedField::Year => self.year(),
            ZonedField::Month => self.month(),
            ZonedField::Day => self.day(),
            ZonedField::Hour => self.hour(),
            ZonedField::Minute => self.minute(),
            ZonedField::Second => self.second(),
            ZonedField::Nanosecond => self.nanosecond(),
            ZonedField::Weekday => self.weekday(),
            ZonedField::OffsetSeconds => self.offset_seconds(),
        }
    }

    /// The same instant seen in another zone.
    pub fn with_zone(&self, zone: TimeZone) -> ZonedTime {
        ZonedTime { instant: self.instant, zone }
    }

    /// Moves along the timeline by an exact duration.
    pub fn checked_add(&self, duration: &Duration) -> Result<ZonedTime, TimeError> {
        Ok(ZonedTime { instant: self.instant.checked_add(duration)?, zone: self.zone })
    }

    pub fn checked_sub(&self, duration: &Duration) -> Result<ZonedTime, TimeError> {
        Ok(ZonedTime { instant: self.instant.checked_sub(duration)?, zone: self.zone })
    }

    /// The same local time `days` calendar days later (or earlier). In
    /// Berlin, one day after 2026-03-28 09:00 is 2026-03-29 09:00, which
    /// is 23 hours later because the clocks moved forward overnight.
    pub fn add_days(&self, days: i64) -> Result<ZonedTime, TimeError> {
        let local = self.local().naive_local();
        let moved = chrono::Duration::try_days(days)
            .and_then(|delta| local.checked_add_signed(delta))
            .ok_or(TimeError::OutOfRange { what: "instant" })?;
        self.at_local(moved)
    }

    /// The same local time `months` calendar months later. A day past
    /// the end of the target month moves back to its last day, so one
    /// month after 31 January is 28 or 29 February.
    pub fn add_months(&self, months: i64) -> Result<ZonedTime, TimeError> {
        let local = self.local().naive_local();
        let count = u32::try_from(months.unsigned_abs()).map_err(|_| TimeError::OutOfRange { what: "instant" })?;
        let moved = if months >= 0 {
            local.checked_add_months(Months::new(count))
        } else {
            local.checked_sub_months(Months::new(count))
        }
        .ok_or(TimeError::OutOfRange { what: "instant" })?;
        self.at_local(moved)
    }

    pub fn add_years(&self, years: i64) -> Result<ZonedTime, TimeError> {
        self.add_months(years.checked_mul(12).ok_or(TimeError::OutOfRange { what: "instant" })?)
    }

    /// The zoned time a calendar move lands on. A local time the zone
    /// shows twice takes the earlier offset, and one the zone skips
    /// moves forward by the length of the gap: the rule RFC 5545
    /// (iCalendar) and Temporal's `compatible` option use, the only
    /// result a calendar can show without asking.
    fn at_local(&self, local: NaiveDateTime) -> Result<ZonedTime, TimeError> {
        let zone = self.zone.chrono_zone();
        let utc = match zone.from_local_datetime(&local) {
            LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => at.with_timezone(&Utc),
            LocalResult::None => {
                let (_, before) = GapInfo::new(&local, &zone)
                    .and_then(|gap| gap.begin)
                    .ok_or(TimeError::OutOfRange { what: "instant" })?;
                let offset = before.fix();
                Utc.from_utc_datetime(&(local - chrono::Duration::seconds(i64::from(offset.local_minus_utc()))))
            }
        };
        Ok(ZonedTime { instant: Instant::from_chrono(utc)?, zone: self.zone })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> TimeZone {
        TimeZone::parse("Europe/Berlin").unwrap()
    }

    fn zoned(text: &str) -> ZonedTime {
        ZonedTime::parse(text).unwrap()
    }

    #[test]
    fn local_fields_follow_the_zone() {
        let at = ZonedTime::new(Instant::parse("2026-03-29T01:30:00Z").unwrap(), berlin());
        assert_eq!((at.year(), at.month(), at.day(), at.hour(), at.minute()), (2026, 3, 29, 3, 30));
        assert_eq!(at.offset_seconds(), 7_200);
        assert_eq!(at.weekday(), 7);
        let winter = ZonedTime::new(Instant::parse("2026-03-29T00:30:00Z").unwrap(), berlin());
        assert_eq!((winter.hour(), winter.offset_seconds()), (1, 3_600));
    }

    #[test]
    fn a_calendar_day_across_the_spring_change_is_23_hours() {
        let saturday = zoned("2026-03-28T09:00:00+01:00[Europe/Berlin]");
        let sunday = saturday.add_days(1).unwrap();
        assert_eq!(sunday.to_string(), "2026-03-29T09:00:00+02:00[Europe/Berlin]");
        assert_eq!(sunday.instant().duration_since(&saturday.instant()).to_string(), "PT23H");
        let exact = saturday.checked_add(&Duration::of(24, crate::TimeUnit::Hours).unwrap()).unwrap();
        assert_eq!(exact.to_string(), "2026-03-29T10:00:00+02:00[Europe/Berlin]");
    }

    #[test]
    fn a_calendar_day_across_the_autumn_change_is_25_hours() {
        let saturday = zoned("2026-10-24T09:00:00+02:00[Europe/Berlin]");
        let sunday = saturday.add_days(1).unwrap();
        assert_eq!(sunday.to_string(), "2026-10-25T09:00:00+01:00[Europe/Berlin]");
        assert_eq!(sunday.instant().duration_since(&saturday.instant()).to_string(), "PT25H");
    }

    #[test]
    fn a_move_into_the_gap_goes_forward_and_into_the_fold_takes_the_earlier_offset() {
        let before_gap = zoned("2026-03-28T02:30:00+01:00[Europe/Berlin]");
        assert_eq!(before_gap.add_days(1).unwrap().to_string(), "2026-03-29T03:30:00+02:00[Europe/Berlin]");
        let before_fold = zoned("2026-10-24T02:30:00+02:00[Europe/Berlin]");
        assert_eq!(before_fold.add_days(1).unwrap().to_string(), "2026-10-25T02:30:00+02:00[Europe/Berlin]");
    }

    #[test]
    fn months_clamp_to_the_end_of_the_month() {
        let january = zoned("2026-01-31T12:00:00+01:00[Europe/Berlin]");
        assert_eq!(january.add_months(1).unwrap().to_string(), "2026-02-28T12:00:00+01:00[Europe/Berlin]");
        let leap = zoned("2028-01-31T12:00:00+01:00[Europe/Berlin]");
        assert_eq!(leap.add_months(1).unwrap().to_string(), "2028-02-29T12:00:00+01:00[Europe/Berlin]");
        assert_eq!(leap.add_years(-1).unwrap().to_string(), "2027-01-31T12:00:00+01:00[Europe/Berlin]");
        assert_eq!(january.add_months(-2).unwrap().to_string(), "2025-11-30T12:00:00+01:00[Europe/Berlin]");
    }

    #[test]
    fn with_zone_keeps_the_instant() {
        let berlin_noon = zoned("2026-07-11T12:00:00+02:00[Europe/Berlin]");
        let tokyo = berlin_noon.with_zone(TimeZone::parse("Asia/Tokyo").unwrap());
        assert_eq!(tokyo.to_string(), "2026-07-11T19:00:00+09:00[Asia/Tokyo]");
        assert_eq!(tokyo.instant(), berlin_noon.instant());
        assert_ne!(tokyo, berlin_noon);
    }
}
