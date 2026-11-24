//! `Duration` and `Instant` on the interpreter tier (B06): operators
//! and methods.
//!
//! Every rule is `corvid_time`'s, the code the native runtime will
//! call, so this file only moves values in and out of `Value` and turns
//! a `TimeError` into a trap. Reading the clock is not here: it is the
//! `time_now` tool, so replay records and substitutes it.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, Value};
use corvid_ast::{BinaryOp, Span};
use corvid_time::{Duration, Instant, TimeError, TimeZone, ZonedTime};
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

fn trap(error: TimeError, span: Span) -> InterpError {
    InterpError::new(InterpErrorKind::Arithmetic(error.to_string()), span)
}

fn mismatch(expected: &str, got: &Value, span: Span) -> InterpError {
    InterpError::new(
        InterpErrorKind::TypeMismatch { expected: expected.into(), got: got.type_name() },
        span,
    )
}

/// Whether `l op r` is a time operation the interpreter evaluates here.
pub(super) fn is_time_operand(value: &Value) -> bool {
    matches!(value, Value::Duration(_) | Value::Instant(_) | Value::ZonedTime(_) | Value::TimeZone(_))
}

/// `l op r` where either side is a `Duration` or an `Instant`. The
/// checker admits only the combinations in `checker/time.rs`.
pub(super) fn eval_time_arithmetic(op: BinaryOp, l: &Value, r: &Value, span: Span) -> Result<Value, InterpError> {
    use BinaryOp::*;
    let duration = |result: Result<Duration, TimeError>| result.map(Value::Duration).map_err(|e| trap(e, span));
    let instant = |result: Result<Instant, TimeError>| result.map(Value::Instant).map_err(|e| trap(e, span));
    match (op, l, r) {
        (Add, Value::Duration(a), Value::Duration(b)) => duration(a.checked_add(b)),
        (Sub, Value::Duration(a), Value::Duration(b)) => duration(a.checked_sub(b)),
        (Mul, Value::Duration(d), Value::Int(n)) | (Mul, Value::Int(n), Value::Duration(d)) => duration(d.checked_mul(*n)),
        (Div, Value::Duration(d), Value::Int(n)) => duration(d.checked_div(*n)),
        (Add, Value::Instant(i), Value::Duration(d)) | (Add, Value::Duration(d), Value::Instant(i)) => {
            instant(i.checked_add(d))
        }
        (Sub, Value::Instant(i), Value::Duration(d)) => instant(i.checked_sub(d)),
        (Sub, Value::Instant(a), Value::Instant(b)) => Ok(Value::Duration(a.duration_since(b))),
        (Add, Value::ZonedTime(z), Value::Duration(d)) | (Add, Value::Duration(d), Value::ZonedTime(z)) => {
            z.checked_add(d).map(Value::ZonedTime).map_err(|e| trap(e, span))
        }
        (Sub, Value::ZonedTime(z), Value::Duration(d)) => z.checked_sub(d).map(Value::ZonedTime).map_err(|e| trap(e, span)),
        (Sub, Value::ZonedTime(a), Value::ZonedTime(b)) => Ok(Value::Duration(a.instant().duration_since(&b.instant()))),
        (_, a, b) => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "a duration or instant operation".into(),
                got: format!("{} and {}", a.type_name(), b.type_name()),
            },
            span,
        )),
    }
}

/// `l < r` and the other orderings between two durations or two instants.
pub(super) fn eval_time_ordering(op: BinaryOp, l: &Value, r: &Value, span: Span) -> Result<Value, InterpError> {
    let ordering = match (l, r) {
        (Value::Duration(a), Value::Duration(b)) => a.cmp(b),
        (Value::Instant(a), Value::Instant(b)) => a.cmp(b),
        (a, _) => return Err(mismatch("two durations or two instants", a, span)),
    };
    Ok(Value::Bool(match op {
        BinaryOp::Lt => ordering.is_lt(),
        BinaryOp::LtEq => ordering.is_le(),
        BinaryOp::Gt => ordering.is_gt(),
        BinaryOp::GtEq => ordering.is_ge(),
        _ => unreachable!("non-ordering op routed here"),
    }))
}

/// A `Duration` or `Instant` builtin method, or a parse into one.
pub(super) fn eval_time_method(
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
    let zone_argument = || match args.first() {
        Some(Value::TimeZone(zone)) => Ok(*zone),
        Some(other) => Err(mismatch("TimeZone", other, span)),
        None => Err(mismatch("TimeZone", &Value::Nothing, span)),
    };
    let count_argument = || match args.first() {
        Some(Value::Int(n)) => Ok(*n),
        Some(other) => Err(mismatch("Int", other, span)),
        None => Err(mismatch("Int", &Value::Nothing, span)),
    };
    let zoned = |result: Result<ZonedTime, TimeError>| result.map(Value::ZonedTime).map_err(|e| trap(e, span));
    match (kind, receiver) {
        (InstantInZone, Value::Instant(i)) => Ok(Value::ZonedTime(i.in_zone(zone_argument()?))),
        (ZonedTimeField(field), Value::ZonedTime(z)) => Ok(Value::Int(z.field(field))),
        (ZonedTimeZone, Value::ZonedTime(z)) => Ok(Value::TimeZone(z.zone())),
        (ZonedTimeInstant, Value::ZonedTime(z)) => Ok(Value::Instant(z.instant())),
        (ZonedTimeAddDays, Value::ZonedTime(z)) => zoned(z.add_days(count_argument()?)),
        (ZonedTimeAddMonths, Value::ZonedTime(z)) => zoned(z.add_months(count_argument()?)),
        (ZonedTimeAddYears, Value::ZonedTime(z)) => zoned(z.add_years(count_argument()?)),
        (ZonedTimeWithZone, Value::ZonedTime(z)) => Ok(Value::ZonedTime(z.with_zone(zone_argument()?))),
        (ZonedTimeToString, Value::ZonedTime(z)) => Ok(text(z.to_string())),
        (TimeZoneToString, Value::TimeZone(z)) => Ok(text(z.to_string())),
        (StringParseTimeZone, Value::String(s)) => Ok(parsed(
            TimeZone::parse(s.trim()).map(Value::TimeZone).map_err(|e| e.to_string()),
        )),
        (StringParseZonedTime, Value::String(s)) => Ok(parsed(
            ZonedTime::parse(s.trim())
                .map(Value::ZonedTime)
                .map_err(|e| format!("`{s}` is not a zoned time: {e}")),
        )),
        (StringParseLocalTime, Value::String(s)) => Ok(parsed(
            ZonedTime::parse_local(s.trim(), zone_argument()?).map(Value::ZonedTime).map_err(|e| e.to_string()),
        )),
        (DurationWhole(unit), Value::Duration(d)) => d.whole(unit).map(Value::Int).map_err(|e| trap(e, span)),
        (DurationAbs, Value::Duration(d)) => Ok(Value::Duration(d.abs())),
        (DurationToString, Value::Duration(d)) => Ok(text(d.to_string())),
        (InstantEpochMilliseconds, Value::Instant(i)) => Ok(Value::Int(i.epoch_milliseconds())),
        (InstantToString, Value::Instant(i)) => Ok(text(i.to_string())),
        (InstantFromEpochMilliseconds, Value::Int(n)) => {
            Instant::from_epoch_milliseconds(*n).map(Value::Instant).map_err(|e| trap(e, span))
        }
        (StringParseDuration, Value::String(s)) => Ok(parsed(
            Duration::parse(s.trim())
                .map(Value::Duration)
                .map_err(|e| format!("`{s}` is not a duration: {e}")),
        )),
        (StringParseInstant, Value::String(s)) => Ok(parsed(
            Instant::parse(s.trim())
                .map(Value::Instant)
                .map_err(|e| format!("`{s}` is not an instant: {e}")),
        )),
        (other, value) => Err(InterpError::new(
            InterpErrorKind::Other(format!("internal: `{other:?}` cannot run on {}", value.type_name())),
            span,
        )),
    }
}
