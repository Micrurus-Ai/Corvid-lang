//! The one error type every fallible decimal operation returns.

use std::fmt;

/// Why a decimal operation has no exact result. The interpreter and
/// the native runtime turn it into a trap; `parse_decimal` returns its
/// message in an `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecimalError {
    /// Text that is not a decimal number. `position` counts Unicode
    /// scalar values from 0, like `String.length()`.
    Malformed { position: usize, message: String },
    /// The exact result needs more fractional digits than `MAX_SCALE`.
    TooManyFractionalDigits { needed: usize },
    /// The exact result needs more significant digits than `MAX_DIGITS`.
    Overflow,
    /// Division or remainder by zero.
    DivisionByZero,
    /// A rounding or division `places` argument outside 0 to `MAX_SCALE`.
    PlacesOutOfRange { places: i64 },
    /// A `Float` with no decimal value: NaN or an infinity.
    NotFinite,
    /// A truncated value outside the 64-bit `Int` range.
    IntOutOfRange,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use crate::value::{MAX_DIGITS, MAX_SCALE};
        match self {
            DecimalError::Malformed { position, message } => {
                write!(f, "{message} at position {position}")
            }
            DecimalError::TooManyFractionalDigits { needed } => write!(
                f,
                "it needs {needed} digits after the point, and a Decimal holds at most {MAX_SCALE}"
            ),
            DecimalError::Overflow => write!(
                f,
                "Decimal overflow: the result needs more than {MAX_DIGITS} significant digits"
            ),
            DecimalError::DivisionByZero => write!(f, "Decimal division by zero"),
            DecimalError::PlacesOutOfRange { places } => write!(
                f,
                "places must be between 0 and {MAX_SCALE}, got {places}"
            ),
            DecimalError::NotFinite => {
                write!(f, "a Float that is NaN or infinite has no Decimal value")
            }
            DecimalError::IntOutOfRange => {
                write!(f, "the Decimal is outside the Int range")
            }
        }
    }
}

impl std::error::Error for DecimalError {}
