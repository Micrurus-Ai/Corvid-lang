//! Reading and writing decimal text.
//!
//! The accepted form is deliberately small: an optional sign, digits,
//! and optionally a point followed by digits (`-12.50`). There is no
//! exponent, no digit separator and no bare `.5` or `5.`, so every
//! accepted text reads one way. Output is plain notation with exactly
//! `scale` fractional digits, never an exponent.

use crate::error::DecimalError;
use crate::value::{Decimal, MAX_DIGITS, MAX_SCALE};
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
        // Every accepted character is ASCII, so a byte index is also the
        // character position an error reports: the first non-ASCII byte
        // is where reading stops.
        let bytes = text.as_bytes();
        if bytes.is_empty() {
            return Err(malformed(0, "expected a decimal number, found empty text"));
        }
        let mut index = 0;
        let negative = match bytes[0] {
            b'-' => {
                index += 1;
                true
            }
            b'+' => {
                index += 1;
                false
            }
            _ => false,
        };
        let integer_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let integer_digits = &bytes[integer_start..index];
        if integer_digits.is_empty() {
            return Err(malformed(index, "expected a digit"));
        }
        let mut fraction_digits: &[u8] = &[];
        if index < bytes.len() && bytes[index] == b'.' {
            index += 1;
            let fraction_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            fraction_digits = &bytes[fraction_start..index];
            if fraction_digits.is_empty() {
                return Err(malformed(index, "expected a digit after the decimal point"));
            }
        }
        if index < bytes.len() {
            let message = match bytes[index] {
                b'e' | b'E' => "exponents are not accepted; write the digits out",
                b'_' | b',' => "digit separators are not accepted",
                _ => "unexpected character",
            };
            return Err(malformed(index, message));
        }
        if fraction_digits.len() > usize::from(MAX_SCALE) {
            return Err(DecimalError::TooManyFractionalDigits {
                needed: fraction_digits.len(),
            });
        }
        let digits = integer_digits.iter().chain(fraction_digits);
        let significant = digits.clone().skip_while(|&&digit| digit == b'0').count();
        if significant > MAX_DIGITS as usize {
            return Err(DecimalError::Overflow);
        }
        // At most `MAX_DIGITS` significant digits fit in an `i128`.
        let magnitude = digits.fold(0i128, |value, &digit| value * 10 + i128::from(digit - b'0'));
        let coefficient = if negative { -magnitude } else { magnitude };
        Decimal::from_parts(coefficient, fraction_digits.len() as u8)
    }

    /// Corvid's `text.parse_decimal()`: trims whitespace, as
    /// `parse_int` does, and on failure names the refused text and the
    /// problem. Both tiers call this, so they return the same `Err`.
    pub fn parse_user_text(text: &str) -> Result<Decimal, String> {
        Decimal::parse(text.trim()).map_err(|error| format!("`{text}` is not a decimal number: {error}"))
    }
}

/// A decimal's canonical text, written without allocating: the sign, at
/// most `MAX_DIGITS` digits, the point and a leading `0` when the value
/// is below one fit in 41 bytes.
pub struct CanonicalText {
    bytes: [u8; 41],
    len: usize,
}

impl std::ops::Deref for CanonicalText {
    type Target = str;

    fn deref(&self) -> &str {
        // Only ASCII digits, `-` and `.` are written.
        std::str::from_utf8(&self.bytes[..self.len]).expect("canonical decimal text is ASCII")
    }
}

impl Decimal {
    /// Plain notation with exactly `scale` fractional digits, never an
    /// exponent; negative zero prints as zero. `Display` writes this.
    pub fn canonical_text(&self) -> CanonicalText {
        let mut digits = [0u8; 39];
        let mut magnitude = self.coefficient().unsigned_abs();
        let mut count = 0;
        loop {
            digits[count] = b'0' + (magnitude % 10) as u8;
            magnitude /= 10;
            count += 1;
            if magnitude == 0 {
                break;
            }
        }
        let scale = usize::from(self.scale());
        // Pad with zeros so there is at least one digit before the point.
        while count <= scale {
            digits[count] = b'0';
            count += 1;
        }
        let mut text = CanonicalText { bytes: [0; 41], len: 0 };
        let mut push = |byte: u8| {
            text.bytes[text.len] = byte;
            text.len += 1;
        };
        if self.coefficient() < 0 {
            push(b'-');
        }
        for position in (0..count).rev() {
            if position + 1 == scale {
                push(b'.');
            }
            push(digits[position]);
        }
        text
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical_text())
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

    /// The text as it was written before `canonical_text` existed.
    fn reference_text(value: &Decimal) -> String {
        let digits = value.coefficient().unsigned_abs().to_string();
        let scale = usize::from(value.scale());
        let sign = if value.coefficient() < 0 { "-" } else { "" };
        if scale == 0 {
            return format!("{sign}{digits}");
        }
        let padded = if digits.len() <= scale {
            format!("{}{digits}", "0".repeat(scale + 1 - digits.len()))
        } else {
            digits
        };
        let (whole, fraction) = padded.split_at(padded.len() - scale);
        format!("{sign}{whole}.{fraction}")
    }

    #[test]
    fn canonical_text_matches_the_reference_at_every_edge() {
        let limit = 10i128.pow(crate::MAX_DIGITS) - 1;
        for magnitude in [0, 1, 9, 10, 99, 100, 12345, 10i128.pow(27), 10i128.pow(28), 10i128.pow(29), limit] {
            for scale in 0..=crate::MAX_SCALE {
                for sign in [1, -1] {
                    let value = Decimal::from_parts(sign * magnitude, scale).unwrap();
                    assert_eq!(value.to_string(), reference_text(&value));
                    assert_eq!(Decimal::parse(&value.to_string()).unwrap().scale(), scale);
                }
            }
        }
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
        // A position counts characters; text before a non-ASCII
        // character is all ASCII, so a byte index and a character
        // index agree wherever reading stops.
        assert_eq!(error("12€"), "unexpected character at position 2");
        assert_eq!(error("€12"), "expected a digit at position 0");
        assert_eq!(error("1.5é"), "unexpected character at position 3");
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
        let widest = format!("-{}.{}", "9".repeat(10), "9".repeat(28));
        assert_eq!(Decimal::parse(&widest).unwrap().to_string(), widest);
        assert_eq!(Decimal::parse(&format!("+{}", "9".repeat(38))).unwrap().to_string(), "9".repeat(38));
        assert_eq!(Decimal::parse("-0").unwrap(), Decimal::ZERO);
    }
}
