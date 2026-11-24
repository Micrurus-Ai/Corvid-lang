//! Declared rounding: `round_<mode>(places)` and `divide_<mode>(b, places)`.
//!
//! These are the only operations that drop digits, and each one names
//! its mode. Both compute the exact quotient in arbitrary precision and
//! round it once, so there is never double rounding.

use crate::error::DecimalError;
use crate::value::{Decimal, MAX_SCALE};
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{One, Signed, Zero};

/// How a result with more digits than requested is rounded. Each mode
/// is one builtin method suffix in Corvid (`round_half_even`,
/// `divide_half_even`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RoundingMode {
    /// Ties go to the even neighbour (banker's rounding).
    HalfEven,
    /// Ties go away from zero (school rounding).
    HalfUp,
    /// Drop the extra digits (truncate).
    TowardZero,
    /// Any non-zero remainder moves away from zero.
    AwayFromZero,
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceiling,
}

impl RoundingMode {
    pub const ALL: [RoundingMode; 6] = [
        RoundingMode::HalfEven,
        RoundingMode::HalfUp,
        RoundingMode::TowardZero,
        RoundingMode::AwayFromZero,
        RoundingMode::Floor,
        RoundingMode::Ceiling,
    ];

    /// The mode a method-name suffix names: `half_even` in `round_half_even`.
    pub fn from_suffix(suffix: &str) -> Option<RoundingMode> {
        RoundingMode::ALL.into_iter().find(|mode| mode.suffix() == suffix)
    }

    /// The method-name suffix: `half_even` in `round_half_even`.
    pub fn suffix(self) -> &'static str {
        match self {
            RoundingMode::HalfEven => "half_even",
            RoundingMode::HalfUp => "half_up",
            RoundingMode::TowardZero => "toward_zero",
            RoundingMode::AwayFromZero => "away_from_zero",
            RoundingMode::Floor => "floor",
            RoundingMode::Ceiling => "ceiling",
        }
    }
}

/// Checks a `places` argument; the checker calls this for literals.
pub fn check_places(places: i64) -> Result<u8, DecimalError> {
    if (0..=i64::from(MAX_SCALE)).contains(&places) {
        Ok(places as u8)
    } else {
        Err(DecimalError::PlacesOutOfRange { places })
    }
}

/// `numerator / denominator` rounded to an integer with `mode`.
/// `denominator` is non-zero.
fn divide_rounded(numerator: &BigInt, denominator: &BigInt, mode: RoundingMode) -> BigInt {
    let (quotient, remainder) = numerator.div_rem(denominator);
    if remainder.is_zero() {
        return quotient;
    }
    let negative = (numerator.sign() == Sign::Minus) != (denominator.sign() == Sign::Minus);
    let away = if negative {
        &quotient - BigInt::one()
    } else {
        &quotient + BigInt::one()
    };
    let twice_remainder: BigInt = remainder.abs() * BigInt::from(2);
    let half = twice_remainder.cmp(&denominator.abs());
    let round_away = match mode {
        RoundingMode::TowardZero => false,
        RoundingMode::AwayFromZero => true,
        RoundingMode::Floor => negative,
        RoundingMode::Ceiling => !negative,
        RoundingMode::HalfUp => half.is_ge(),
        RoundingMode::HalfEven => half.is_gt() || (half.is_eq() && quotient.is_odd()),
    };
    if round_away {
        away
    } else {
        quotient
    }
}

impl Decimal {
    /// Rounds to exactly `places` fractional digits. Fewer digits are
    /// padded with zeros, so the result always has scale `places`.
    pub fn round(&self, places: i64, mode: RoundingMode) -> Result<Decimal, DecimalError> {
        let places = check_places(places)?;
        if places >= self.scale() {
            return Decimal::from_big(self.coefficient_at_scale(places), usize::from(places));
        }
        let divisor = BigInt::from(10).pow(u32::from(self.scale() - places));
        let rounded = divide_rounded(&self.big_coefficient(), &divisor, mode);
        Decimal::from_big(rounded, usize::from(places))
    }

    /// `self / divisor`, rounded once to exactly `places` fractional
    /// digits with `mode`.
    pub fn divide(
        &self,
        divisor: &Decimal,
        places: i64,
        mode: RoundingMode,
    ) -> Result<Decimal, DecimalError> {
        let places = check_places(places)?;
        if divisor.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }
        // self / divisor × 10^places
        //   = (a × 10^-sa) / (b × 10^-sb) × 10^places
        //   = (a × 10^(places + sb)) / (b × 10^sa)
        let ten = BigInt::from(10);
        let numerator =
            self.big_coefficient() * ten.pow(u32::from(places) + u32::from(divisor.scale()));
        let denominator = divisor.big_coefficient() * ten.pow(u32::from(self.scale()));
        let rounded = divide_rounded(&numerator, &denominator, mode);
        Decimal::from_big(rounded, usize::from(places))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RoundingMode::*;

    fn d(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }

    fn rounded(text: &str, places: i64, mode: RoundingMode) -> String {
        d(text).round(places, mode).unwrap().to_string()
    }

    #[test]
    fn every_mode_on_ties_and_non_ties() {
        // value: half_even, half_up, toward_zero, away_from_zero, floor, ceiling
        let table: &[(&str, [&str; 6])] = &[
            ("2.5", ["2", "3", "2", "3", "2", "3"]),
            ("3.5", ["4", "4", "3", "4", "3", "4"]),
            ("-2.5", ["-2", "-3", "-2", "-3", "-3", "-2"]),
            ("2.4", ["2", "2", "2", "3", "2", "3"]),
            ("2.6", ["3", "3", "2", "3", "2", "3"]),
            ("-2.6", ["-3", "-3", "-2", "-3", "-3", "-2"]),
            ("2", ["2", "2", "2", "2", "2", "2"]),
        ];
        for (value, expected) in table {
            for (mode, want) in RoundingMode::ALL.iter().zip(expected) {
                assert_eq!(&rounded(value, 0, *mode), want, "{value} {}", mode.suffix());
            }
        }
    }

    #[test]
    fn rounding_pads_to_the_requested_places() {
        assert_eq!(rounded("2.5", 2, HalfEven), "2.50");
        assert_eq!(rounded("1.005", 2, HalfUp), "1.01");
        assert_eq!(rounded("1.005", 2, HalfEven), "1.00");
        assert_eq!(rounded("-0.004", 2, HalfUp), "0.00");
    }

    #[test]
    fn places_out_of_range_is_an_error() {
        assert_eq!(d("1").round(-1, HalfEven), Err(DecimalError::PlacesOutOfRange { places: -1 }));
        assert_eq!(d("1").round(29, HalfEven), Err(DecimalError::PlacesOutOfRange { places: 29 }));
    }

    #[test]
    fn division_rounds_once() {
        let third = d("1").divide(&d("3"), 4, HalfEven).unwrap();
        assert_eq!(third.to_string(), "0.3333");
        assert_eq!(d("2").divide(&d("3"), 2, HalfEven).unwrap().to_string(), "0.67");
        assert_eq!(d("2").divide(&d("3"), 2, TowardZero).unwrap().to_string(), "0.66");
        assert_eq!(d("-2").divide(&d("3"), 2, Floor).unwrap().to_string(), "-0.67");
        assert_eq!(d("10.00").divide(&d("4"), 2, HalfEven).unwrap().to_string(), "2.50");
        assert_eq!(d("1").divide(&d("0.125"), 0, HalfEven).unwrap().to_string(), "8");
        // Splitting 100.00 three ways: the declared rounding is visible.
        assert_eq!(d("100.00").divide(&d("3"), 2, HalfEven).unwrap().to_string(), "33.33");
    }

    #[test]
    fn division_by_zero_is_an_error() {
        assert_eq!(d("1").divide(&d("0.00"), 2, HalfEven), Err(DecimalError::DivisionByZero));
    }

    #[test]
    fn rounding_to_more_places_can_overflow() {
        let big = d(&"9".repeat(30));
        assert_eq!(big.round(10, HalfEven), Err(DecimalError::Overflow));
    }
}
