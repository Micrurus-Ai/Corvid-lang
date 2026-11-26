//! The one error type every fallible time operation returns.

use std::fmt;

/// Why a time operation has no exact result. The interpreter and the
/// native runtime turn it into a trap; the parse methods return its
/// message in an `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeError {
    /// Text that is not a duration or an instant. `position` counts
    /// Unicode scalar values from 0, like `String.length()`.
    Malformed { position: usize, message: String },
    /// A duration written in days, weeks, months or years, which have
    /// no fixed length.
    CalendarUnit { unit: &'static str },
    /// A result outside the range the type holds.
    OutOfRange { what: &'static str },
    /// Division of a duration by zero.
    DivisionByZero,
    /// A reading too large for a 64-bit `Int`.
    IntOutOfRange,
    /// A name that is not an IANA zone, with the closest one if any.
    UnknownZone { name: String, suggestion: Option<&'static str> },
    /// A local time the zone skips, in a daylight-saving gap.
    LocalTimeSkipped { local: String, zone: &'static str, before: String, after: String },
    /// A local time the zone shows twice, in a daylight-saving fold.
    LocalTimeRepeated { local: String, zone: &'static str, first: String, second: String },
    /// Zoned text whose offset is not the zone's offset at that time.
    OffsetMismatch { text: String, zone: &'static str, expected: String },
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeError::Malformed { position, message } => write!(f, "{message} at position {position}"),
            TimeError::CalendarUnit { unit } => write!(
                f,
                "a {unit} is not a fixed length of time; write the span in hours, such as PT24H for one day of 24 hours"
            ),
            TimeError::OutOfRange { what } => write!(f, "the {what} is outside the range Corvid time holds"),
            TimeError::DivisionByZero => write!(f, "Duration division by zero"),
            TimeError::IntOutOfRange => write!(f, "the reading is outside the Int range"),
            TimeError::UnknownZone { name, suggestion: Some(suggestion) } => {
                write!(f, "`{name}` is not a time zone in the IANA database; did you mean `{suggestion}`?")
            }
            TimeError::UnknownZone { name, suggestion: None } => {
                write!(f, "`{name}` is not a time zone in the IANA database")
            }
            TimeError::LocalTimeSkipped { local, zone, before, after } => write!(
                f,
                "{local} does not exist in {zone}: clocks move from {before} to {after} there, skipping it"
            ),
            TimeError::LocalTimeRepeated { local, zone, first, second } => write!(
                f,
                "{local} happens twice in {zone}: once at {first} and again at {second}"
            ),
            TimeError::OffsetMismatch { text, zone, expected } => write!(
                f,
                "`{text}` gives an offset {zone} does not use at that time; {zone} is at {expected} then"
            ),
        }
    }
}

impl std::error::Error for TimeError {}
