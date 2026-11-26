//! Currency formatting on the interpreter tier (B06).
//!
//! Every rule is `corvid_currency`'s, over the same CLDR data the
//! checker validated literals with, so this file only moves values in
//! and out of `Value`. An amount with more digits than its currency
//! shows traps: formatting never rounds.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, Value};
use corvid_ast::Span;
use corvid_currency::{Currency, Locale};
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

fn mismatch(expected: &str, got: Option<&Value>, span: Span) -> InterpError {
    InterpError::new(
        InterpErrorKind::TypeMismatch {
            expected: expected.into(),
            got: got.map_or_else(|| "no argument".into(), Value::type_name),
        },
        span,
    )
}

pub(super) fn eval_currency_method(
    kind: BuiltinMethodKind,
    receiver: &Value,
    args: &[Value],
    span: Span,
) -> Result<Value, InterpError> {
    use BuiltinMethodKind::*;
    let text = |value: String| Value::String(Arc::from(value));
    let parsed = |result: Result<Value, String>| match result {
        Ok(value) => Value::ResultOk(BoxedValue::new(value)),
        Err(message) => Value::ResultErr(BoxedValue::new(text(message))),
    };
    match (kind, receiver) {
        (DecimalFormatCurrency, Value::Decimal(amount)) => {
            let currency = match args.first() {
                Some(Value::Currency(c)) => *c,
                other => return Err(mismatch("Currency", other, span)),
            };
            let locale = match args.get(1) {
                Some(Value::Locale(l)) => l.clone(),
                other => return Err(mismatch("Locale", other, span)),
            };
            corvid_currency::format(amount, currency, &locale)
                .map(text)
                .map_err(|error| InterpError::new(InterpErrorKind::Arithmetic(error.to_string()), span))
        }
        (CurrencyMinorDigits, Value::Currency(c)) => Ok(Value::Int(i64::from(c.minor_digits()))),
        (CurrencyToString, Value::Currency(c)) => Ok(text(c.to_string())),
        (LocaleToString, Value::Locale(l)) => Ok(text(l.to_string())),
        (StringParseCurrency, Value::String(s)) => Ok(parsed(Currency::parse_user_text(s).map(Value::Currency))),
        (StringParseLocale, Value::String(s)) => Ok(parsed(Locale::parse_user_text(s).map(|l| Value::Locale(Arc::new(l))))),
        (other, value) => Err(InterpError::new(
            InterpErrorKind::Other(format!("internal: `{other:?}` cannot run on {}", value.type_name())),
            span,
        )),
    }
}
