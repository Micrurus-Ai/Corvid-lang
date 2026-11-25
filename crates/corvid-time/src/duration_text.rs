//! ISO 8601 duration text: `PT1H30M`, `PT0.25S`, `-PT5S`.
//!
//! Only the time part (`PT…`) is accepted. Days, weeks, months and
//! years (`P1D`, `P2W`, `P1M`, `P1Y`) are rejected with a message,
//! because none of them is a fixed length; Corvid's `Duration` is
//! exact elapsed time. Output never uses days either, so `PT25H` stays
//! `PT25H`.

use crate::duration::Duration;
use crate::error::TimeError;
use crate::unit::TimeUnit;

pub(crate) fn format(duration: &Duration) -> String {
    let total = duration.nanoseconds();
    let sign = if total < 0 { "-" } else { "" };
    let mut rest = total.unsigned_abs();
    let hour = TimeUnit::Hours.nanoseconds() as u128;
    let minute = TimeUnit::Minutes.nanoseconds() as u128;
    let second = TimeUnit::Seconds.nanoseconds() as u128;
    let hours = rest / hour;
    rest %= hour;
    let minutes = rest / minute;
    rest %= minute;
    let seconds = rest / second;
    let fraction = rest % second;
    let mut text = format!("{sign}PT");
    if hours > 0 {
        text.push_str(&format!("{hours}H"));
    }
    if minutes > 0 {
        text.push_str(&format!("{minutes}M"));
    }
    if seconds > 0 || fraction > 0 || (hours == 0 && minutes == 0) {
        text.push_str(&seconds.to_string());
        if fraction > 0 {
            let digits = format!("{fraction:09}");
            text.push('.');
            text.push_str(digits.trim_end_matches('0'));
        }
        text.push('S');
    }
    text
}

impl Duration {
    /// Corvid's `text.parse_duration()`: trims whitespace, as
    /// `parse_int` does, and on failure names the refused text and the
    /// problem. Both tiers call this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<Duration, String> {
        Duration::parse(text.trim()).map_err(|error| format!("`{text}` is not a duration: {error}"))
    }

    /// Reads ISO 8601 duration text. Components are written in order
    /// (hours, minutes, seconds), each at most once; only seconds take
    /// a fraction, of at most nine digits.
    pub fn parse(text: &str) -> Result<Duration, TimeError> {
        let chars: Vec<char> = text.chars().collect();
        let malformed = |position: usize, message: &str| TimeError::Malformed {
            position,
            message: message.to_string(),
        };
        let mut index = 0;
        let negative = match chars.first() {
            Some('-') => {
                index += 1;
                true
            }
            Some('+') => {
                index += 1;
                false
            }
            _ => false,
        };
        if chars.get(index) != Some(&'P') {
            return Err(malformed(index, "expected an ISO 8601 duration such as PT1H30M"));
        }
        index += 1;
        if chars.get(index) != Some(&'T') {
            // A date part: name the calendar unit it uses.
            let mut probe = index;
            while probe < chars.len() && chars[probe].is_ascii_digit() {
                probe += 1;
            }
            let unit = match chars.get(probe) {
                Some('D') => "day",
                Some('W') => "week",
                Some('M') => "month",
                Some('Y') => "year",
                _ => return Err(malformed(index, "expected `T` before the hours, minutes and seconds")),
            };
            return Err(TimeError::CalendarUnit { unit });
        }
        index += 1;
        let mut total: i128 = 0;
        let mut any = false;
        let mut next_allowed = 0; // 0: H, M or S; 1: M or S; 2: S; 3: nothing
        while index < chars.len() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            if index == start {
                return Err(malformed(index, "expected a number"));
            }
            let whole: i128 = chars[start..index]
                .iter()
                .collect::<String>()
                .parse()
                .map_err(|_| TimeError::OutOfRange { what: "duration" })?;
            let mut fraction_nanos: i128 = 0;
            if chars.get(index) == Some(&'.') {
                index += 1;
                let fraction_start = index;
                while index < chars.len() && chars[index].is_ascii_digit() {
                    index += 1;
                }
                let digits: String = chars[fraction_start..index].iter().collect();
                if digits.is_empty() {
                    return Err(malformed(index, "expected a digit after the decimal point"));
                }
                if digits.len() > 9 {
                    return Err(malformed(fraction_start + 9, "a duration is exact to the nanosecond, so at most nine fractional digits"));
                }
                if chars.get(index) != Some(&'S') {
                    return Err(malformed(index, "only seconds take a fraction"));
                }
                fraction_nanos = format!("{digits:0<9}").parse().expect("nine digits");
            }
            let (unit, order) = match chars.get(index) {
                Some('H') => (TimeUnit::Hours, 0),
                Some('M') => (TimeUnit::Minutes, 1),
                Some('S') => (TimeUnit::Seconds, 2),
                Some(_) => return Err(malformed(index, "expected H, M or S")),
                None => return Err(malformed(index, "expected H, M or S after the number")),
            };
            if order < next_allowed {
                return Err(malformed(index, "write hours, minutes and seconds in that order, each once"));
            }
            next_allowed = order + 1;
            index += 1;
            any = true;
            total = whole
                .checked_mul(unit.nanoseconds())
                .and_then(|n| n.checked_add(fraction_nanos))
                .and_then(|n| total.checked_add(n))
                .ok_or(TimeError::OutOfRange { what: "duration" })?;
        }
        if !any {
            return Err(malformed(index, "expected at least one of hours, minutes or seconds"));
        }
        Duration::from_nanoseconds(if negative { -total } else { total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_is_trimmed_and_its_error_names_the_text() {
        assert_eq!(Duration::parse_user_text(" PT1H30M ").unwrap().to_string(), "PT1H30M");
        let error = Duration::parse_user_text("P1D").unwrap_err();
        assert!(error.starts_with("`P1D` is not a duration: a day is not a fixed length of time"), "{error}");
    }

    fn round_trip(text: &str) -> String {
        Duration::parse(text).unwrap().to_string()
    }

    #[test]
    fn text_round_trips() {
        assert_eq!(round_trip("PT1H30M"), "PT1H30M");
        assert_eq!(round_trip("PT90M"), "PT1H30M");
        assert_eq!(round_trip("PT0.25S"), "PT0.25S");
        assert_eq!(round_trip("PT1.000000001S"), "PT1.000000001S");
        assert_eq!(round_trip("-PT5S"), "-PT5S");
        assert_eq!(round_trip("PT25H"), "PT25H");
        assert_eq!(round_trip("PT0S"), "PT0S");
        assert_eq!(round_trip("PT1H0.5S"), "PT1H0.5S");
    }

    #[test]
    fn calendar_units_are_rejected_by_name() {
        assert_eq!(Duration::parse("P1D"), Err(TimeError::CalendarUnit { unit: "day" }));
        assert_eq!(Duration::parse("P2W"), Err(TimeError::CalendarUnit { unit: "week" }));
        assert_eq!(Duration::parse("P1M"), Err(TimeError::CalendarUnit { unit: "month" }));
        assert_eq!(Duration::parse("P1Y"), Err(TimeError::CalendarUnit { unit: "year" }));
        assert!(Duration::parse("P1D").unwrap_err().to_string().contains("PT24H"));
    }

    #[test]
    fn malformed_text_names_the_position() {
        let error = |text: &str| Duration::parse(text).unwrap_err().to_string();
        assert_eq!(error("1H"), "expected an ISO 8601 duration such as PT1H30M at position 0");
        assert_eq!(error("PT"), "expected at least one of hours, minutes or seconds at position 2");
        assert_eq!(error("PT1.5M"), "only seconds take a fraction at position 5");
        assert_eq!(error("PT5M1H"), "write hours, minutes and seconds in that order, each once at position 5");
        assert_eq!(error("PT0.0000000001S"), "a duration is exact to the nanosecond, so at most nine fractional digits at position 13");
        assert_eq!(error("PT5"), "expected H, M or S after the number at position 3");
        assert_eq!(error("pt5s"), "expected an ISO 8601 duration such as PT1H30M at position 0");
    }
}
