//! `Decimal` on the interpreter tier (B06): operators and methods.
//!
//! Every rule is `corvid_decimal`'s, the code the checker parses
//! literals with and the native runtime will call, so this file only
//! moves values in and out of `Value` and turns a `DecimalError` into a
//! trap.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, Value};
use corvid_ast::{BinaryOp, Span};
use corvid_decimal::{Decimal, DecimalError};
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

fn trap(error: DecimalError, span: Span) -> InterpError {
    InterpError::new(InterpErrorKind::Arithmetic(error.to_string()), span)
}

fn mismatch(expected: &str, got: Option<&Value>, span: Span) -> InterpError {
    InterpError::new(
        InterpErrorKind::TypeMismatch {
            expected: expected.into(),
            got: got.map_or_else(|| "no argument".into(), Value::type_name),
        },
        span,
    )
}

/// A `Decimal` operand; an `Int` widens exactly.
fn operand(value: &Value, span: Span) -> Result<Decimal, InterpError> {
    match value {
        Value::Decimal(d) => Ok(*d),
        Value::Int(n) => Ok(Decimal::from_int(*n)),
        other => Err(mismatch("Decimal or Int", Some(other), span)),
    }
}

fn int_argument(args: &[Value], index: usize, span: Span) -> Result<i64, InterpError> {
    match args.get(index) {
        Some(Value::Int(n)) => Ok(*n),
        other => Err(mismatch("Int", other, span)),
    }
}

/// `l op r` where either side is a `Decimal`.
pub(super) fn eval_decimal_arithmetic(
    op: BinaryOp,
    l: &Value,
    r: &Value,
    span: Span,
) -> Result<Value, InterpError> {
    let (a, b) = (operand(l, span)?, operand(r, span)?);
    let result = match op {
        BinaryOp::Add => a.checked_add(&b),
        BinaryOp::Sub => a.checked_sub(&b),
        BinaryOp::Mul => a.checked_mul(&b),
        BinaryOp::Mod => a.checked_rem(&b),
        // The checker accepts a Decimal `/` only as the receiver of a
        // `round_<mode>` call, which IR lowering fuses into
        // `DecimalDivideRound`, so a bare one never runs.
        _ => {
            return Err(InterpError::new(
                InterpErrorKind::Other(
                    "internal: a Decimal division reached the interpreter without its rounding".into(),
                ),
                span,
            ))
        }
    };
    result.map(Value::Decimal).map_err(|error| trap(error, span))
}

/// `l < r` and the other orderings where either side is a `Decimal`.
pub(super) fn eval_decimal_ordering(
    op: BinaryOp,
    l: &Value,
    r: &Value,
    span: Span,
) -> Result<Value, InterpError> {
    let (a, b) = (operand(l, span)?, operand(r, span)?);
    Ok(Value::Bool(match op {
        BinaryOp::Lt => a < b,
        BinaryOp::LtEq => a <= b,
        BinaryOp::Gt => a > b,
        BinaryOp::GtEq => a >= b,
        _ => unreachable!("non-ordering op routed here"),
    }))
}

/// A `Decimal` builtin method, or a conversion into `Decimal`.
pub(super) fn eval_decimal_method(
    kind: BuiltinMethodKind,
    receiver: &Value,
    args: &[Value],
    span: Span,
) -> Result<Value, InterpError> {
    use BuiltinMethodKind::*;
    let decimal = |value: &Value| match value {
        Value::Decimal(d) => Ok(*d),
        other => Err(mismatch("Decimal", Some(other), span)),
    };
    let checked = |result: Result<Decimal, DecimalError>| {
        result.map(Value::Decimal).map_err(|error| trap(error, span))
    };
    match kind {
        DecimalRound(mode) => checked(decimal(receiver)?.round(int_argument(args, 0, span)?, mode)),
        DecimalDivideRound(mode) => {
            let divisor = operand(args.first().ok_or_else(|| mismatch("Decimal", None, span))?, span)?;
            checked(operand(receiver, span)?.divide(&divisor, int_argument(args, 1, span)?, mode))
        }
        DecimalAbs => Ok(Value::Decimal(decimal(receiver)?.abs())),
        DecimalScale => Ok(Value::Int(i64::from(decimal(receiver)?.scale()))),
        DecimalToString => Ok(Value::String(Arc::from(decimal(receiver)?.to_string()))),
        DecimalToFloat => Ok(Value::Float(decimal(receiver)?.to_float())),
        DecimalToIntTruncated => decimal(receiver)?
            .to_int_truncated()
            .map(Value::Int)
            .map_err(|error| trap(error, span)),
        IntToDecimal => match receiver {
            Value::Int(n) => Ok(Value::Decimal(Decimal::from_int(*n))),
            other => Err(mismatch("Int", Some(other), span)),
        },
        FloatToDecimal => match receiver {
            Value::Float(f) => checked(Decimal::from_float(*f)),
            other => Err(mismatch("Float", Some(other), span)),
        },
        StringParseDecimal => match receiver {
            Value::String(text) => Ok(match Decimal::parse_user_text(text) {
                Ok(d) => Value::ResultOk(BoxedValue::new(Value::Decimal(d))),
                Err(message) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(message)))),
            }),
            other => Err(mismatch("String", Some(other), span)),
        },
        other => Err(InterpError::new(
            InterpErrorKind::Other(format!("internal: `{other:?}` is not a Decimal method")),
            span,
        )),
    }
}
