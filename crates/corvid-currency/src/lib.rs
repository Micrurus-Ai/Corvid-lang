//! Currency formatting for Corvid's `Decimal` (B06), with Unicode CLDR
//! data through ICU4X.
//!
//! Every rule lives here once: the checker validates `Currency` and
//! `Locale` literals with [`Currency::parse`] and [`Locale::parse`], and
//! the interpreter and the native runtime format with [`format`], so
//! the tiers cannot disagree on a digit or a symbol.
//!
//! Formatting never rounds. An amount with more fractional digits than
//! the currency's minor unit is an error that asks the program to
//! round first with a declared mode, because a formatter that rounded
//! would be a second, hidden rounding policy on an invoice.

mod currency;
mod error;
mod format;
mod locale;

pub use currency::Currency;
pub use error::CurrencyError;
pub use format::format;
pub use locale::Locale;
