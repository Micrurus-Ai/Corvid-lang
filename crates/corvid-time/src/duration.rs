//! `Duration`: an exact length of elapsed time, and its arithmetic.

use crate::error::TimeError;
use crate::instant::{MAX_NANOSECONDS, MIN_NANOSECONDS};
use crate::unit::TimeUnit;
use std::fmt;

/// The longest duration: the span of the instant range, so every
/// difference of two instants is a duration and no duration is longer
/// than any instant arithmetic can use.
const MAX_SPAN: i128 = MAX_NANOSECONDS - MIN_NANOSECONDS;

/// An exact signed length of time in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Duration {
    nanoseconds: i128,
}

impl Duration {
    pub const ZERO: Duration = Duration { nanoseconds: 0 };

    pub fn from_nanoseconds(nanoseconds: i128) -> Result<Duration, TimeError> {
        if (-MAX_SPAN..=MAX_SPAN).contains(&nanoseconds) {
            Ok(Duration { nanoseconds })
        } else {
            Err(TimeError::OutOfRange { what: "duration" })
        }
    }

    /// `count` of `unit`, exactly.
    pub fn of(count: i64, unit: TimeUnit) -> Result<Duration, TimeError> {
        Duration::from_nanoseconds(i128::from(count) * unit.nanoseconds())
    }

    pub fn nanoseconds(&self) -> i128 {
        self.nanoseconds
    }

    /// The number of whole `unit`s, truncated toward zero:
    /// `whole(Seconds)` of 1.9 seconds is 1, and of -1.9 seconds is -1.
    pub fn whole(&self, unit: TimeUnit) -> Result<i64, TimeError> {
        i64::try_from(self.nanoseconds / unit.nanoseconds()).map_err(|_| TimeError::IntOutOfRange)
    }

    pub fn checked_add(&self, other: &Duration) -> Result<Duration, TimeError> {
        Duration::from_nanoseconds(self.nanoseconds + other.nanoseconds)
    }

    pub fn checked_sub(&self, other: &Duration) -> Result<Duration, TimeError> {
        Duration::from_nanoseconds(self.nanoseconds - other.nanoseconds)
    }

    pub fn checked_mul(&self, factor: i64) -> Result<Duration, TimeError> {
        let product = self
            .nanoseconds
            .checked_mul(i128::from(factor))
            .ok_or(TimeError::OutOfRange { what: "duration" })?;
        Duration::from_nanoseconds(product)
    }

    /// `self / divisor`, truncated toward zero to the nanosecond, as
    /// `Int` division truncates.
    pub fn checked_div(&self, divisor: i64) -> Result<Duration, TimeError> {
        if divisor == 0 {
            return Err(TimeError::DivisionByZero);
        }
        Duration::from_nanoseconds(self.nanoseconds / i128::from(divisor))
    }

    /// `-self`. The range is symmetric, so this never fails.
    pub fn negate(&self) -> Duration {
        Duration { nanoseconds: -self.nanoseconds }
    }

    pub fn abs(&self) -> Duration {
        Duration { nanoseconds: self.nanoseconds.abs() }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", crate::duration_text::format(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TimeUnit::*;

    #[test]
    fn construction_is_exact_in_every_unit() {
        assert_eq!(Duration::of(90, Minutes).unwrap(), Duration::of(5_400, Seconds).unwrap());
        assert_eq!(Duration::of(1, Hours).unwrap().nanoseconds(), 3_600_000_000_000);
        assert_eq!(Duration::of(-3, Microseconds).unwrap().nanoseconds(), -3_000);
    }

    #[test]
    fn readings_truncate_toward_zero() {
        let d = Duration::from_nanoseconds(1_900_000_000).unwrap();
        assert_eq!(d.whole(Seconds).unwrap(), 1);
        assert_eq!(d.negate().whole(Seconds).unwrap(), -1);
        assert_eq!(d.whole(Milliseconds).unwrap(), 1_900);
        assert_eq!(Duration::of(i64::MAX, Hours).map(|d| d.whole(Nanoseconds)), Err(TimeError::OutOfRange { what: "duration" }));
    }

    #[test]
    fn arithmetic_is_checked() {
        let hour = Duration::of(1, Hours).unwrap();
        assert_eq!(hour.checked_mul(3).unwrap(), Duration::of(180, Minutes).unwrap());
        assert_eq!(hour.checked_div(7).unwrap().nanoseconds(), 514_285_714_285);
        assert_eq!(hour.checked_div(0), Err(TimeError::DivisionByZero));
        assert_eq!(hour.checked_sub(&hour.checked_mul(2).unwrap()).unwrap(), hour.negate());
        let longest = Duration::from_nanoseconds(MAX_SPAN).unwrap();
        assert_eq!(longest.checked_add(&Duration::of(1, Nanoseconds).unwrap()), Err(TimeError::OutOfRange { what: "duration" }));
        assert_eq!(hour.checked_mul(i64::MAX), Err(TimeError::OutOfRange { what: "duration" }));
    }

    #[test]
    fn a_reading_too_large_for_int_is_an_error() {
        let longest = Duration::from_nanoseconds(MAX_SPAN).unwrap();
        assert_eq!(longest.whole(Nanoseconds), Err(TimeError::IntOutOfRange));
        assert!(longest.whole(Seconds).is_ok());
    }
}
