//! Compile-time checks for `Currency` and `Locale` literals (B06).
//!
//! A string literal where a `Currency` or `Locale` is meant is one,
//! checked against the bundled CLDR data with `corvid_currency`, the
//! code the runtime formats with: `total.format_currency("EUR", "de-DE")`.
//! A code or tag CLDR does not know is a compile error, never a silent
//! fallback to another currency's symbol or the root locale's digits.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::Span;

impl<'a> Checker<'a> {
    pub(super) fn check_currency_literal(&mut self, code: &str, span: Span) -> Type {
        if let Err(error) = corvid_currency::Currency::parse(code) {
            self.errors.push(TypeError::new(TypeErrorKind::UnknownCurrencyOrLocale { problem: error.to_string() }, span));
        }
        Type::Currency
    }

    pub(super) fn check_locale_literal(&mut self, tag: &str, span: Span) -> Type {
        if let Err(error) = corvid_currency::Locale::parse(tag) {
            self.errors.push(TypeError::new(TypeErrorKind::UnknownCurrencyOrLocale { problem: error.to_string() }, span));
        }
        Type::Locale
    }
}
