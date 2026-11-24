//! The `Decimal` value: its representation, bounds, equality and order.

use crate::error::DecimalError;
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// The most fractional digits a `Decimal` holds, as in Python's
/// `decimal` default context and .NET's `decimal`.
pub const MAX_SCALE: u8 = 28;

/// The most significant digits a `Decimal` holds. 38 digits is the
/// widest coefficient a signed 128-bit integer represents for every
/// digit pattern.
pub const MAX_DIGITS: u32 = 38;

/// `10^MAX_DIGITS`, the exclusive bound on a coefficient's magnitude.
const COEFFICIENT_LIMIT: i128 = 10i128.pow(MAX_DIGITS);

/// An exact decimal number: `coefficient × 10^-scale`.
///
/// The scale is kept, so `2.50` prints as `2.50`, but equality, order
/// and hashing are numeric: `1.0 == 1.00`, and both are the same map
/// key.
#[derive(Debug, Clone, Copy)]
pub struct Decimal {
    coefficient: i128,
    scale: u8,
}

impl Decimal {
    pub const ZERO: Decimal = Decimal {
        coefficient: 0,
        scale: 0,
    };

    /// Builds a value from its parts, checking both bounds.
    pub fn from_parts(coefficient: i128, scale: u8) -> Result<Decimal, DecimalError> {
        if scale > MAX_SCALE {
            return Decimal::from_big(BigInt::from(coefficient), usize::from(scale));
        }
        if coefficient <= -COEFFICIENT_LIMIT || coefficient >= COEFFICIENT_LIMIT {
            return Err(DecimalError::Overflow);
        }
        Ok(Decimal { coefficient, scale })
    }

    /// Builds a value from an exact arbitrary-precision coefficient.
    /// A scale above `MAX_SCALE` is accepted only when the extra
    /// fractional digits are zeros, so no digit is ever dropped.
    pub(crate) fn from_big(mut coefficient: BigInt, mut scale: usize) -> Result<Decimal, DecimalError> {
        let ten = BigInt::from(10);
        while scale > usize::from(MAX_SCALE) {
            if coefficient.is_zero() {
                scale = usize::from(MAX_SCALE);
                break;
            }
            if !(&coefficient % &ten).is_zero() {
                let mut needed = scale;
                let mut probe = coefficient.clone();
                while (&probe % &ten).is_zero() {
                    probe /= &ten;
                    needed -= 1;
                }
                return Err(DecimalError::TooManyFractionalDigits { needed });
            }
            coefficient /= &ten;
            scale -= 1;
        }
        let coefficient = coefficient.to_i128().ok_or(DecimalError::Overflow)?;
        Decimal::from_parts(coefficient, scale as u8)
    }

    pub fn coefficient(&self) -> i128 {
        self.coefficient
    }

    /// The number of fractional digits, as written or as produced.
    pub fn scale(&self) -> u8 {
        self.scale
    }

    pub fn is_zero(&self) -> bool {
        self.coefficient == 0
    }

    pub(crate) fn big_coefficient(&self) -> BigInt {
        BigInt::from(self.coefficient)
    }

    /// The coefficient rescaled to `scale` (which must be at least
    /// this value's scale), exactly.
    pub(crate) fn coefficient_at_scale(&self, scale: u8) -> BigInt {
        debug_assert!(scale >= self.scale);
        self.big_coefficient() * BigInt::from(10).pow(u32::from(scale - self.scale))
    }

    /// The same number with trailing fractional zeros removed: the
    /// canonical form equality and hashing use.
    fn normalized(&self) -> (i128, u8) {
        let mut coefficient = self.coefficient;
        let mut scale = self.scale;
        while scale > 0 && coefficient % 10 == 0 {
            coefficient /= 10;
            scale -= 1;
        }
        (coefficient, scale)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.normalized() == other.normalized()
    }
}

impl Eq for Decimal {}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.normalized().hash(state);
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let scale = self.scale.max(other.scale);
        self.coefficient_at_scale(scale)
            .cmp(&other.coefficient_at_scale(scale))
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    #[test]
    fn equality_ignores_trailing_zeros_but_scale_is_kept() {
        assert_eq!(d("1.0"), d("1.00"));
        assert_eq!(d("1.50").scale(), 2);
        assert_ne!(d("1.5"), d("1.05"));
    }

    #[test]
    fn equal_values_hash_alike() {
        use std::collections::HashSet;
        let set: HashSet<Decimal> = [d("2"), d("2.000")].into_iter().collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn order_is_numeric() {
        assert!(d("-1.5") < d("-1.25"));
        assert!(d("0.10") < d("0.2"));
        assert!(d("10") > d("9.999"));
    }

    #[test]
    fn bounds_are_checked() {
        assert!(Decimal::from_parts(COEFFICIENT_LIMIT - 1, 0).is_ok());
        assert_eq!(
            Decimal::from_parts(COEFFICIENT_LIMIT, 0),
            Err(DecimalError::Overflow)
        );
        // Extra fractional zeros are dropped; non-zero digits are not.
        let rescaled = Decimal::from_parts(1500, 30).unwrap();
        assert_eq!(rescaled, Decimal::from_parts(15, 28).unwrap());
        assert_eq!(rescaled.scale(), 28);
        assert_eq!(
            Decimal::from_parts(15, 30),
            Err(DecimalError::TooManyFractionalDigits { needed: 30 })
        );
    }
}
