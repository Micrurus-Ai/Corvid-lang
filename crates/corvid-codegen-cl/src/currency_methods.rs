//! The `Currency`, `Locale` and `Decimal.format_currency` operations
//! native code calls (B06): which runtime symbol runs each one.
//!
//! Native code holds a `Currency` as a String containing its ISO 4217
//! code and a `Locale` as one containing its canonical BCP 47 tag.
//! Every operation is one call into
//! `corvid-runtime/src/ffi_bridge/currency.rs`, which runs
//! `corvid-currency` over the CLDR data the interpreter uses. Every
//! parameter and return is one word. The import declarations, the
//! lowering, the tier picker and native match arms all read this table.

use crate::text_scalar::{scalar_of, BinaryReading, NativeScalarMethod, ScalarBinary};
use corvid_ast::BinaryOp;
use corvid_types::{BuiltinMethodKind, Type};

/// One runtime entry point for currency formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CurrencyOperation {
    /// `amount.format_currency(currency, locale)`.
    FormatCurrency,
    MinorDigits,
    /// 1 when two currencies are the same currency.
    CurrencyEqual,
    /// 1 when two locales are the same locale.
    LocaleEqual,
    ParseCurrency,
    ParseLocale,
}

impl CurrencyOperation {
    /// Every operation, for declaring the imports.
    pub fn all() -> Vec<CurrencyOperation> {
        use CurrencyOperation::*;
        vec![FormatCurrency, MinorDigits, CurrencyEqual, LocaleEqual, ParseCurrency, ParseLocale]
    }

    pub fn symbol(self) -> &'static str {
        use CurrencyOperation::*;
        match self {
            FormatCurrency => "corvid_decimal_format_currency",
            MinorDigits => "corvid_currency_minor_digits",
            CurrencyEqual => "corvid_currency_equal",
            LocaleEqual => "corvid_locale_equal",
            ParseCurrency => "corvid_string_parse_currency",
            ParseLocale => "corvid_string_parse_locale",
        }
    }

    /// How many one-word parameters the entry point takes; every entry
    /// point returns one word.
    pub fn arity(self) -> usize {
        use CurrencyOperation::*;
        match self {
            FormatCurrency => 3,
            MinorDigits => 1,
            CurrencyEqual | LocaleEqual | ParseCurrency | ParseLocale => 2,
        }
    }

    /// Whether this fills a `Result<T, String>` payload slot and returns its tag.
    pub fn is_parse(self) -> bool {
        matches!(self, CurrencyOperation::ParseCurrency | CurrencyOperation::ParseLocale)
    }
}

/// `==` and `!=` between two currencies or two locales.
pub fn currency_binary(op: BinaryOp, left: &Type, right: &Type) -> Option<ScalarBinary<CurrencyOperation>> {
    let operation = match (op, scalar_of(left), scalar_of(right)) {
        (BinaryOp::Eq | BinaryOp::NotEq, Type::Currency, Type::Currency) => CurrencyOperation::CurrencyEqual,
        (BinaryOp::Eq | BinaryOp::NotEq, Type::Locale, Type::Locale) => CurrencyOperation::LocaleEqual,
        _ => return None,
    };
    Some(ScalarBinary { operation, swapped: false, reading: BinaryReading::Equality })
}

/// Whether either operand is a `Currency` or a `Locale`.
pub fn has_currency_operand(left: &Type, right: &Type) -> bool {
    [left, right].into_iter().any(|ty| matches!(scalar_of(ty), Type::Currency | Type::Locale))
}

/// The native form of a currency method. Every `is_currency()` kind has one.
pub fn native_currency_method(kind: BuiltinMethodKind) -> Option<NativeScalarMethod<CurrencyOperation>> {
    use BuiltinMethodKind::*;
    use CurrencyOperation as Op;
    use NativeScalarMethod::*;
    Some(match kind {
        DecimalFormatCurrency => Call(Op::FormatCurrency),
        CurrencyMinorDigits => Call(Op::MinorDigits),
        CurrencyToString | LocaleToString => CanonicalText,
        StringParseCurrency => Call(Op::ParseCurrency),
        StringParseLocale => Call(Op::ParseLocale),
        _ => return None,
    })
}

/// Whether native code compiles this currency method.
pub fn is_native_currency_method(kind: BuiltinMethodKind) -> bool {
    native_currency_method(kind).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_currency_method_has_a_native_form() {
        use BuiltinMethodKind::*;
        for kind in [DecimalFormatCurrency, CurrencyMinorDigits, CurrencyToString, LocaleToString, StringParseCurrency, StringParseLocale] {
            assert!(kind.is_currency(), "{kind:?}");
            assert!(is_native_currency_method(kind), "{kind:?} needs a native form");
        }
        let symbols: std::collections::HashSet<&str> = CurrencyOperation::all().iter().map(|op| op.symbol()).collect();
        assert_eq!(symbols.len(), CurrencyOperation::all().len());
    }

    #[test]
    fn only_currencies_and_locales_compare_here() {
        assert!(currency_binary(BinaryOp::Eq, &Type::Currency, &Type::Currency).is_some());
        assert!(currency_binary(BinaryOp::NotEq, &Type::Locale, &Type::Locale).is_some());
        assert!(currency_binary(BinaryOp::Lt, &Type::Currency, &Type::Currency).is_none());
        assert!(has_currency_operand(&Type::Locale, &Type::String));
    }
}
