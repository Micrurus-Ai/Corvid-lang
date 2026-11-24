//! Conversions between `Decimal` and Corvid's `Int` and `Float`.
//!
//! `Int` to `Decimal` is exact. `Float` to `Decimal` takes the shortest
//! decimal that reads back as the same `Float` (the digits
//! `Float.to_string()` shows), so `0.1` becomes `0.1`, not the 55-digit
//! binary expansion. `Decimal` to `Float` is the only lossy direction,
//! and Corvid spells it `to_float()`.

use crate::error::DecimalError;
use crate::rounding::RoundingMode;
use crate::value::Decimal;
use num_traits::ToPrimitive;

impl Decimal {
    pub fn from_int(value: i64) -> Decimal {
        Decimal::from_parts(i128::from(value), 0).expect("every i64 has at most 19 digits")
    }

    /// The shortest decimal that reads back as `value`. Rust's `f64`
    /// `Display` prints that shortest form without an exponent.
    pub fn from_float(value: f64) -> Result<Decimal, DecimalError> {
        if !value.is_finite() {
            return Err(DecimalError::NotFinite);
        }
        Decimal::parse(&value.to_string())
    }

    /// The nearest `Float`, through the correctly rounded `f64` parser.
    pub fn to_float(&self) -> f64 {
        self.to_string()
            .parse()
            .expect("decimal text is a valid float literal")
    }

    /// Truncates toward zero to an `Int`.
    pub fn to_int_truncated(&self) -> Result<i64, DecimalError> {
        let whole = self.round(0, RoundingMode::TowardZero)?;
        whole
            .coefficient()
            .to_i64()
            .ok_or(DecimalError::IntOutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_convert_to_their_shortest_decimal() {
        assert_eq!(Decimal::from_float(0.1).unwrap().to_string(), "0.1");
        assert_eq!(Decimal::from_float(19.99).unwrap().to_string(), "19.99");
        assert_eq!(Decimal::from_float(3.0).unwrap().to_string(), "3");
        assert_eq!(Decimal::from_float(-1e-7).unwrap().to_string(), "-0.0000001");
        assert_eq!(Decimal::from_float(f64::NAN), Err(DecimalError::NotFinite));
        assert_eq!(Decimal::from_float(f64::INFINITY), Err(DecimalError::NotFinite));
        assert_eq!(
            Decimal::from_float(1e-30),
            Err(DecimalError::TooManyFractionalDigits { needed: 30 })
        );
    }

    #[test]
    fn ints_convert_exactly() {
        assert_eq!(Decimal::from_int(i64::MIN).to_string(), i64::MIN.to_string());
        assert_eq!(Decimal::from_int(42).to_int_truncated().unwrap(), 42);
    }

    #[test]
    fn to_float_is_nearest() {
        assert_eq!(Decimal::parse("0.1").unwrap().to_float(), 0.1);
        assert_eq!(Decimal::parse("-2.50").unwrap().to_float(), -2.5);
    }

    #[test]
    fn truncation_to_int() {
        assert_eq!(Decimal::parse("-2.99").unwrap().to_int_truncated().unwrap(), -2);
        assert_eq!(
            Decimal::parse(&"9".repeat(20)).unwrap().to_int_truncated(),
            Err(DecimalError::IntOutOfRange)
        );
    }
}
