//! The one error type every fallible currency operation returns.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrencyError {
    /// Not an ISO 4217 code in the bundled CLDR data.
    UnknownCurrency { code: String },
    /// Not a well-formed BCP 47 tag, or a language CLDR has no data for.
    UnknownLocale { tag: String, reason: String },
    /// An amount with more fractional digits than the currency's minor
    /// unit, which formatting would have to round.
    TooManyDigits { amount: String, code: String, digits: u8 },
}

impl fmt::Display for CurrencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurrencyError::UnknownCurrency { code } => {
                write!(f, "`{code}` is not an ISO 4217 currency code, such as USD or EUR")
            }
            CurrencyError::UnknownLocale { tag, reason } => write!(f, "`{tag}` is not a locale Corvid can format for: {reason}"),
            CurrencyError::TooManyDigits { amount, code, digits } => write!(
                f,
                "{amount} has more digits after the point than {code} shows ({digits}); formatting never rounds, so round it first, such as with round_half_even({digits})"
            ),
        }
    }
}

impl std::error::Error for CurrencyError {}
