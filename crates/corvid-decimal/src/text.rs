//! Reading and writing decimal text.
//!
//! The accepted form is deliberately small: an optional sign, digits,
//! and optionally a point followed by digits (`-12.50`). There is no
//! exponent, no digit separator and no bare `.5` or `5.`, so every
//! accepted text reads one way. Output is plain notation with exactly
//! `scale` fractional digits, never an exponent.

use crate::error::DecimalError;
use crate::value::{Decimal, MAX_DIGITS, MAX_SCALE};
use num_bigint::BigInt;
use std::fmt;

impl Decimal {
    /// Parses decimal text exactly. Whitespace is not trimmed here;
    /// `String.parse_decimal` trims before calling, and a
    /// `Decimal("…")` literal must be written without it.
    pub fn parse(text: &str) -> Result<Decimal, DecimalError> {
        let malformed = |position: usize, message: &str| DecimalError::Malformed {
            position,
            message: message.to_string(),
        };
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return Err(malformed(0, "expected a decimal number, found empty text"));
        }
        let mut index = 0;
        let negative = match chars[0] {
            '-' => {
                index += 1;
                true
            }
            '+' => {
                index += 1;
                false
            }
            _ => false,
        };
        let mut integer_digits = String::new();
        while index < chars.len() && chars[index].is_ascii_digit() {
            integer_digits.push(chars[index]);
            index += 1;
        }
        if integer_digits.is_empty() {
            return Err(malformed(index, "expected a digit"));
        }
        let mut fraction_digits = String::new();
        if index < chars.len() && chars[index] == '.' {
            index += 1;
            while index < chars.len() && chars[index].is_ascii_digit() {
                fraction_digits.push(chars[index]);
                index += 1;
            }
            if fraction_digits.is_empty() {
                return Err(malformed(index, "expected a digit after the decimal point"));
            }
        }
        if index < chars.len() {
            let message = match chars[index] {
                'e' | 'E' => "exponents are not accepted; write the digits out",
                '_' | ',' => "digit separators are not accepted",
                _ => "unexpected character",
            };
            return Err(malformed(index, message));
        }
        if fraction_digits.len() > usize::from(MAX_SCALE) {
            return Err(DecimalError::TooManyFractionalDigits {
                needed: fraction_digits.len(),
            });
        }
        let digits = format!("{integer_digits}{fraction_digits}");
        let significant = digits.trim_start_matches('0').len();
        if significant > MAX_DIGITS as usize {
            return Err(DecimalError::Overflow);
        }
        let mut coefficient: BigInt = digits.parse().expect("only ASCII digits remain");
        if negative {
            coefficient = -coefficient;
        }
        Decimal::from_big(coefficient, fraction_digits.len())
    }

    /// Corvid's `text.parse_decimal()`: trims whitespace, as
    /// `parse_int` does, and on failure names the refused text and the
    /// problem. Both tiers call this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<Decimal, String> {
        Decimal::parse(text.trim()).map_err(|error| format!("`{text}` is not a decimal number: {error}"))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.coefficient().unsigned_abs().to_string();
        let scale = usize::from(self.scale());
        let sign = if self.coefficient() < 0 { "-" } else { "" };
        if scale == 0 {
            return write!(f, "{sign}{digits}");
        }
        let padded = if digits.len() <= scale {
            format!("{}{digits}", "0".repeat(scale + 1 - digits.len()))
        } else {
            digits
        };
        let (whole, fraction) = padded.split_at(padded.len() - scale);
        write!(f, "{sign}{whole}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) -> String {
        Decimal::parse(text).unwrap().to_string()
    }

    #[test]
    fn text_round_trips_with_its_scale() {
        assert_eq!(round_trip("19.99"), "19.99");
        assert_eq!(round_trip("2.50"), "2.50");
        assert_eq!(round_trip("-0.05"), "-0.05");
        assert_eq!(round_trip("+7"), "7");
        assert_eq!(round_trip("0007.10"), "7.10");
        assert_eq!(round_trip("0.0000000000000000000000000001"), "0.0000000000000000000000000001");
    }

    #[test]
    fn user_text_is_trimmed_and_its_error_names_the_text() {
        assert_eq!(Decimal::parse_user_text(" 19.990 ").unwrap().to_string(), "19.990");
        assert_eq!(
            Decimal::parse_user_text("1e3").unwrap_err(),
            "`1e3` is not a decimal number: exponents are not accepted; write the digits out at position 1"
        );
    }

    #[test]
    fn negative_zero_prints_as_zero() {
        assert_eq!(round_trip("-0.00"), "0.00");
    }

    #[test]
    fn malformed_text_names_the_position() {
        let error = |text: &str| Decimal::parse(text).unwrap_err().to_string();
        assert_eq!(error(""), "expected a decimal number, found empty text at position 0");
        assert_eq!(error("1.2.3"), "unexpected character at position 3");
        assert_eq!(error(".5"), "expected a digit at position 0");
        assert_eq!(error("5."), "expected a digit after the decimal point at position 2");
        assert_eq!(error("1e3"), "exponents are not accepted; write the digits out at position 1");
        assert_eq!(error("1,000"), "digit separators are not accepted at position 1");
        assert_eq!(error(" 1"), "expected a digit at position 0");
    }

    #[test]
    fn limits_are_reported() {
        assert_eq!(
            Decimal::parse("0.00000000000000000000000000001"),
            Err(DecimalError::TooManyFractionalDigits { needed: 29 })
        );
        assert!(Decimal::parse(&"9".repeat(38)).is_ok());
        assert_eq!(Decimal::parse(&"9".repeat(39)), Err(DecimalError::Overflow));
        // Leading zeros are not significant digits.
        assert!(Decimal::parse(&format!("000{}", "9".repeat(38))).is_ok());
    }
}
