//! Exact arithmetic: `+`, `-`, `*`, `%`, negation and absolute value.
//!
//! Each operation computes the exact result in arbitrary precision and
//! checks it back into range, so a result is exact or an error. There
//! is no `/` here; division always names its rounding (see
//! `rounding.rs`).

use crate::error::DecimalError;
use crate::value::Decimal;

impl Decimal {
    /// `self + other`. The result's scale is the larger operand scale,
    /// as in Python: `1.50 + 1.2` is `2.70`.
    pub fn checked_add(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = self.scale().max(other.scale());
        let sum = self.coefficient_at_scale(scale) + other.coefficient_at_scale(scale);
        Decimal::from_big(sum, usize::from(scale))
    }

    /// `self - other`, with the scale rule of `checked_add`.
    pub fn checked_sub(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = self.scale().max(other.scale());
        let difference = self.coefficient_at_scale(scale) - other.coefficient_at_scale(scale);
        Decimal::from_big(difference, usize::from(scale))
    }

    /// `self * other`. The result's scale is the sum of the operand
    /// scales, so `1.5 * 1.5` is `2.25`. A product that needs more
    /// than `MAX_SCALE` fractional digits is an error, never rounded.
    pub fn checked_mul(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let product = self.big_coefficient() * other.big_coefficient();
        Decimal::from_big(product, usize::from(self.scale()) + usize::from(other.scale()))
    }

    /// `self % other`: the remainder of truncating division, carrying
    /// the dividend's sign as `Int`'s `%` does (`-7 % 2` is `-1`). The
    /// result is exact, with the larger operand scale.
    pub fn checked_rem(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        if other.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }
        let scale = self.scale().max(other.scale());
        let remainder = self.coefficient_at_scale(scale) % other.coefficient_at_scale(scale);
        Decimal::from_big(remainder, usize::from(scale))
    }

    /// `-self`. The coefficient bound is symmetric, so this never fails.
    pub fn negate(&self) -> Decimal {
        Decimal::from_parts(-self.coefficient(), self.scale())
            .expect("the coefficient bound is symmetric")
    }

    pub fn abs(&self) -> Decimal {
        if self.coefficient() < 0 {
            self.negate()
        } else {
            *self
        }
    }

    pub fn is_negative(&self) -> bool {
        self.coefficient() < 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    #[test]
    fn addition_is_exact_where_floats_drift() {
        assert_eq!(d("0.1").checked_add(&d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("1.50").checked_add(&d("1.2")).unwrap().to_string(), "2.70");
        assert_eq!(d("1").checked_sub(&d("0.01")).unwrap().to_string(), "0.99");
    }

    #[test]
    fn a_long_sum_of_cents_does_not_drift() {
        let mut total = Decimal::ZERO;
        for _ in 0..1000 {
            total = total.checked_add(&d("0.10")).unwrap();
        }
        assert_eq!(total.to_string(), "100.00");
    }

    #[test]
    fn multiplication_adds_scales() {
        assert_eq!(d("1.5").checked_mul(&d("1.5")).unwrap().to_string(), "2.25");
        assert_eq!(d("19.99").checked_mul(&d("3")).unwrap().to_string(), "59.97");
    }

    #[test]
    fn a_product_with_too_many_fractional_digits_is_an_error() {
        // 10^-14 squared is 10^-28, exactly at the limit.
        let tiny = d("0.00000000000001");
        assert_eq!(tiny.checked_mul(&tiny).unwrap().scale(), 28);
        let tinier = d("0.000000000000003");
        assert_eq!(
            tinier.checked_mul(&tinier),
            Err(DecimalError::TooManyFractionalDigits { needed: 30 })
        );
        // Trailing zeros past the limit are not digits that are lost.
        assert_eq!(
            d("0.00000000000000100").checked_mul(&d("0.0000000000010")).unwrap(),
            d("0.0000000000000000000000000010")
        );
    }

    #[test]
    fn overflow_is_an_error() {
        let big = d(&"9".repeat(38));
        assert_eq!(big.checked_add(&d("1")), Err(DecimalError::Overflow));
        assert_eq!(big.checked_mul(&d("2")), Err(DecimalError::Overflow));
    }

    #[test]
    fn remainder_carries_the_dividend_sign() {
        assert_eq!(d("7.5").checked_rem(&d("2")).unwrap().to_string(), "1.5");
        assert_eq!(d("-7").checked_rem(&d("2")).unwrap().to_string(), "-1");
        assert_eq!(d("1").checked_rem(&d("0.0")), Err(DecimalError::DivisionByZero));
    }

    #[test]
    fn negation_and_absolute_value() {
        assert_eq!(d("2.50").negate().to_string(), "-2.50");
        assert_eq!(d("-2.50").abs().to_string(), "2.50");
        assert!(d("-0.01").is_negative());
        assert!(!d("0").is_negative());
    }
}
