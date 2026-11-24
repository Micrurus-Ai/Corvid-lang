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
        }
    }
}

impl std::error::Error for TimeError {}
