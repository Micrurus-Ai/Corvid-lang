//! Exact decimal numbers for Corvid's `Decimal` type (B06).
//!
//! Every rule behind `Decimal` lives here once: the checker parses
//! `Decimal("…")` literals with [`Decimal::parse`], and the interpreter
//! and the native runtime execute arithmetic, rounding and conversion
//! through the same functions, so the tiers cannot disagree on a digit.
//! The crate depends on no other Corvid crate.
//!
//! A value is a signed coefficient of at most [`MAX_DIGITS`] decimal
//! digits and a scale from 0 to [`MAX_SCALE`]: the value is
//! `coefficient × 10^-scale`. Every operation either produces the exact
//! result or returns a [`DecimalError`]. Nothing rounds unless the
//! caller names a [`RoundingMode`], which is how the language keeps
//! rounding a declared decision.

mod arithmetic;
mod conversion;
mod error;
mod rounding;
mod text;
mod value;

pub use error::DecimalError;
pub use rounding::{check_places, RoundingMode};
pub use value::{Decimal, MAX_DIGITS, MAX_SCALE};
