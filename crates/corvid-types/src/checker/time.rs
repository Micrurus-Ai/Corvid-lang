//! Compile-time rules for `Duration` and `Instant` (B06).
//!
//! The operators follow what the two types mean:
//!
//! | Expression | Type |
//! | --- | --- |
//! | `duration + duration`, `duration - duration` | `Duration` |
//! | `duration * n`, `n * duration`, `duration / n` (`n: Int`) | `Duration` |
//! | `instant + duration`, `duration + instant`, `instant - duration` | `Instant` |
//! | `instant - instant` | `Duration` |
//! | comparisons between two durations or two instants | `Bool` |
//! | `zoned + duration`, `duration + zoned`, `zoned - duration` | `ZonedTime` |
//! | `zoned - zoned` | `Duration` |
//! | `==` and `!=` between two zoned times or two zones | `Bool` |
//!
//! A string literal where a `TimeZone` is meant is a zone, checked
//! against the bundled IANA database at compile time:
//! `at.in_zone("Europe/Berlin")`.
//!
//! Everything else that involves a time type is a compile error that
//! says what would make sense. Adding an `Int` to a duration is the
//! papercut bare-millisecond durations kept, so it is refused with the
//! reason; a bare number has no unit.

use super::Checker;
use crate::builtin_methods::builtin_type_method;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{BinaryOp, Expr, Ident, Span};

fn is_time(t: &Type) -> bool {
    matches!(t, Type::Duration | Type::Instant | Type::ZonedTime | Type::TimeZone)
}

fn is_comparison(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq | BinaryOp::NotEq | BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq
    )
}

impl<'a> Checker<'a> {
    /// The type of `l op r` when either side is a `Duration` or an
    /// `Instant`, or `None` to leave the operator to the ordinary
    /// rules. `lt` and `rt` have `Grounded` and `Tainted` stripped.
    pub(super) fn check_time_binop(
        &mut self,
        op: BinaryOp,
        l: &Expr,
        r: &Expr,
        lt: &Type,
        rt: &Type,
    ) -> Option<Type> {
        if !is_time(lt) && !is_time(rt) {
            return None;
        }
        use BinaryOp::*;
        use Type::{Duration, Instant, Int, TimeZone, Unknown, ZonedTime};
        let result = match (op, lt, rt) {
            (_, Unknown, _) | (_, _, Unknown) => Some(if is_comparison(op) { Type::Bool } else { Unknown }),
            (Add | Sub, Duration, Duration) => Some(Duration),
            (Mul, Duration, Int) | (Mul, Int, Duration) | (Div, Duration, Int) => Some(Duration),
            (Add, Instant, Duration) | (Add, Duration, Instant) | (Sub, Instant, Duration) => Some(Instant),
            (Sub, Instant, Instant) => Some(Duration),
            (Add, ZonedTime, Duration) | (Add, Duration, ZonedTime) | (Sub, ZonedTime, Duration) => Some(ZonedTime),
            (Sub, ZonedTime, ZonedTime) => Some(Duration),
            (Eq | NotEq, ZonedTime, ZonedTime) | (Eq | NotEq, TimeZone, TimeZone) => Some(Type::Bool),
            (Eq | NotEq | Lt | LtEq | Gt | GtEq, Duration, Duration)
            | (Eq | NotEq | Lt | LtEq | Gt | GtEq, Instant, Instant) => Some(Type::Bool),
            _ => None,
        };
        if let Some(result) = result {
            return Some(result);
        }
        let hint = match (lt, rt) {
            (Duration, Int) | (Int, Duration) if matches!(op, Add | Sub) => {
                "a bare number has no unit; add a `Duration`, such as one read with `parse_duration()` or the difference of two instants"
            }
            (Instant, Instant) => "two instants can be subtracted, giving the `Duration` between them, but not added",
            (Duration, Duration) => "divide a duration by an `Int`, or compare two durations",
            (Instant, Int) | (Int, Instant) => "an instant moves by a `Duration`, not by a number",
            (ZonedTime, ZonedTime) => {
                "zoned times in different zones can be the same instant, so compare their instants: `a.instant() < b.instant()`"
            }
            (ZonedTime, Int) | (Int, ZonedTime) => {
                "a zoned time moves by a `Duration`, or by calendar days with `add_days(n)`"
            }
            _ => "durations combine with durations and `Int` factors; instants move by durations",
        };
        self.errors.push(TypeError::new(
            TypeErrorKind::TimeOperatorMismatch {
                left: lt.display_name(),
                right: rt.display_name(),
                hint: hint.to_string(),
            },
            l.span().merge(r.span()),
        ));
        Some(if is_comparison(op) { Type::Bool } else { Unknown })
    }

    /// Types a string literal that sits in a `TimeZone` slot, checking
    /// its name against the IANA database the runtime uses.
    pub(super) fn check_time_zone_literal(&mut self, name: &str, span: Span) -> Type {
        if let Err(corvid_time::TimeError::UnknownZone { suggestion, .. }) = corvid_time::TimeZone::parse(name) {
            self.errors.push(TypeError::new(
                TypeErrorKind::UnknownTimeZone { name: name.to_string(), suggestion: suggestion.map(str::to_string) },
                span,
            ));
        }
        Type::TimeZone
    }

    /// `Instant.from_epoch_milliseconds(n)`: a method of a compiler-known
    /// type name. Returns `None` when `target` is not such a name or the
    /// method is not one of its type methods.
    pub(super) fn check_builtin_type_method_call(
        &mut self,
        target: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        let receiver = crate::builtin_methods::builtin_type_receiver(target, self.bindings)?;
        let sig = builtin_type_method(&receiver, &method.name)?;
        if args.len() != sig.params.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch { callee: method.name.clone(), expected: sig.params.len(), got: args.len() },
                span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Some(sig.ret);
        }
        for (arg, param) in args.iter().zip(&sig.params) {
            let got = self.check_expr_as(arg, Some(param));
            if !got.is_assignable_to(param) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: param.display_name(),
                        got: got.display_name(),
                        context: format!("argument to `{}`", method.name),
                    },
                    arg.span(),
                ));
            }
        }
        Some(sig.ret)
    }
}
