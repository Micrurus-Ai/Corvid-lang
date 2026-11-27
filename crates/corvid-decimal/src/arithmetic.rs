//! Exact arithmetic: `+`, `-`, `*`, `%`, negation and absolute value.
//!
//! Each operation computes the exact result and checks it back into
//! range, so a result is exact or an error. The result is computed in
//! 128 bits when every intermediate fits, which is nearly always, and in
//! arbitrary precision otherwise; both give the same value or the same
//! error, because the fast path only runs when nothing overflows and the
//! result's scale is within bounds. There
//! is no `/` here; division always names its rounding (see
//! `rounding.rs`).

use crate::error::DecimalError;
use crate::value::Decimal;

impl Decimal {
    /// `self + other`. The result's scale is the larger operand scale,
    /// as in Python: `1.50 + 1.2` is `2.70`.
    pub fn checked_add(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = self.scale().max(other.scale());
        if let Some(sum) = self.small_pair(other, scale).and_then(|(a, b)| a.checked_add(b)) {
            return Decimal::from_parts(sum, scale);
        }
        let sum = self.coefficient_at_scale(scale) + other.coefficient_at_scale(scale);
        Decimal::from_big(sum, usize::from(scale))
    }

    /// `self - other`, with the scale rule of `checked_add`.
    pub fn checked_sub(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = self.scale().max(other.scale());
        if let Some(difference) = self.small_pair(other, scale).and_then(|(a, b)| a.checked_sub(b)) {
            return Decimal::from_parts(difference, scale);
        }
        let difference = self.coefficient_at_scale(scale) - other.coefficient_at_scale(scale);
        Decimal::from_big(difference, usize::from(scale))
    }

    /// `self * other`. The result's scale is the sum of the operand
    /// scales, so `1.5 * 1.5` is `2.25`. A product that needs more
    /// than `MAX_SCALE` fractional digits is an error, never rounded.
    pub fn checked_mul(&self, other: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = usize::from(self.scale()) + usize::from(other.scale());
        if scale <= usize::from(crate::MAX_SCALE) {
            if let Some(product) = self.coefficient().checked_mul(other.coefficient()) {
                return Decimal::from_parts(product, scale as u8);
            }
        }
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
        if let Some(remainder) = self.small_pair(other, scale).and_then(|(a, b)| a.checked_rem(b)) {
            return Decimal::from_parts(remainder, scale);
        }
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

    /// Both coefficients at `scale`, when both fit in 128 bits.
    fn small_pair(&self, other: &Decimal, scale: u8) -> Option<(i128, i128)> {
        Some((self.small_coefficient_at_scale(scale)?, other.small_coefficient_at_scale(scale)?))
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

    /// The arbitrary-precision computation every fast path must agree
    /// with: the operation as it was before the 128-bit path existed.
    fn reference(op: char, a: &Decimal, b: &Decimal) -> Result<Decimal, DecimalError> {
        let scale = a.scale().max(b.scale());
        match op {
            '+' => Decimal::from_big(a.coefficient_at_scale(scale) + b.coefficient_at_scale(scale), usize::from(scale)),
            '-' => Decimal::from_big(a.coefficient_at_scale(scale) - b.coefficient_at_scale(scale), usize::from(scale)),
            '*' => Decimal::from_big(
                a.big_coefficient() * b.big_coefficient(),
                usize::from(a.scale()) + usize::from(b.scale()),
            ),
            '%' if b.is_zero() => Err(DecimalError::DivisionByZero),
            '%' => Decimal::from_big(a.coefficient_at_scale(scale) % b.coefficient_at_scale(scale), usize::from(scale)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_128_bit_path_agrees_with_arbitrary_precision_at_every_edge() {
        let limit = 10i128.pow(crate::MAX_DIGITS) - 1;
        let magnitudes = [0, 1, 7, 10, 999, 1_000_000_007, 10i128.pow(19), 10i128.pow(37), limit / 2, limit - 1, limit];
        let scales = [0u8, 1, 2, 9, 14, 15, 27, 28];
        let mut values = Vec::new();
        for &m in &magnitudes {
            for &s in &scales {
                for sign in [1, -1] {
                    values.push(Decimal::from_parts(sign * m, s).unwrap());
                }
            }
        }
        let mut checked = 0;
        for a in &values {
            for b in &values {
                assert_eq!(a.checked_add(b), reference('+', a, b), "{a} + {b}");
                assert_eq!(a.checked_sub(b), reference('-', a, b), "{a} - {b}");
                assert_eq!(a.checked_mul(b), reference('*', a, b), "{a} * {b}");
                assert_eq!(a.checked_rem(b), reference('%', a, b), "{a} % {b}");
                let scale = a.scale().max(b.scale());
                assert_eq!(a.cmp(b), a.coefficient_at_scale(scale).cmp(&b.coefficient_at_scale(scale)), "{a} <=> {b}");
                checked += 1;
            }
        }
        assert_eq!(checked, values.len() * values.len());
    }

    #[test]
    fn negation_and_absolute_value() {
        assert_eq!(d("2.50").negate().to_string(), "-2.50");
        assert_eq!(d("-2.50").abs().to_string(), "2.50");
        assert!(d("-0.01").is_negative());
        assert!(!d("0").is_negative());
    }
}
