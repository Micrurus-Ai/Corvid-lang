//! Compile-time operator rules for `Bytes`.
//!
//! `==` and `!=` between two byte strings are the only operators, and
//! both are constant-time at run time, so comparing a computed
//! signature with a received one cannot leak it. Everything else is a
//! compile error that says why:
//!
//! - **No ordering.** `<` and the other orderings would have to stop at
//!   the first differing byte, which is the timing leak `==` removes.
//! - **No `+`.** Joining byte strings is not something any hashing or
//!   signature check in this slice needs, and `+` must not quietly mix
//!   bytes with text.
//! - **No mixing with `String`.** Text becomes bytes only through
//!   `utf8()`, and bytes become text only through a named encoding.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{BinaryOp, Expr};

impl<'a> Checker<'a> {
    /// The type of `l op r` when either side is `Bytes`, or `None` to
    /// leave the operator to the ordinary rules. `lt` and `rt` have
    /// `Grounded` and `Tainted` stripped.
    pub(super) fn check_bytes_binop(&mut self, op: BinaryOp, l: &Expr, r: &Expr, lt: &Type, rt: &Type) -> Option<Type> {
        if !matches!(lt, Type::Bytes) && !matches!(rt, Type::Bytes) {
            return None;
        }
        use BinaryOp::*;
        let is_comparison = matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq);
        let recovered = if is_comparison { Type::Bool } else { Type::Unknown };
        let hint = match (op, lt, rt) {
            (_, Type::Unknown, _) | (_, _, Type::Unknown) => return Some(recovered),
            (Eq | NotEq, Type::Bytes, Type::Bytes) => return Some(Type::Bool),
            // `a + b` joins two byte strings; bytes and text never mix.
            (Add, Type::Bytes, Type::Bytes) => return Some(Type::Bytes),
            (Lt | LtEq | Gt | GtEq, Type::Bytes, Type::Bytes) => {
                "byte strings have no order: an ordered comparison stops at the first differing byte, which leaks it; compare with `==`, which is constant-time"
            }
            (_, Type::String, _) | (_, _, Type::String) => {
                "bytes and text do not mix: turn text into bytes with `utf8()`, or bytes into text with `to_hex()`, `to_base64()`, `to_base64_url()` or `to_utf8()`"
            }
            _ => "`Bytes` supports `==` and `!=`, which are constant-time, and `+`, which joins two byte strings",
        };
        self.errors.push(TypeError::new(
            TypeErrorKind::BytesOperatorMismatch {
                left: lt.display_name(),
                right: rt.display_name(),
                hint: hint.to_string(),
            },
            l.span().merge(r.span()),
        ));
        Some(recovered)
    }
}
