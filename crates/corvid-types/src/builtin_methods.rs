//! The builtin-method table (slice 45c) — the single source of truth
//! for methods on built-in receiver types (`String`, `List<T>`,
//! `Int`, `Float`, `Option<T>`, `Result<T,E>`, `Grounded<T>`, …).
//!
//! Three consumers share this table so a method exists exactly once:
//!
//! 1. The **checker** (`check_method_call`) looks up the signature,
//!    checks arity + argument types, and returns the result type.
//! 2. The **IR lowerer** re-derives the same lookup from the
//!    receiver's checked type and lowers to
//!    `IrExprKind::BuiltinMethod { kind, .. }`.
//! 3. The **interpreter** executes on the `BuiltinMethodKind` enum —
//!    one match arm per method.
//!
//! Adding a method = one arm in `builtin_method` + one interpreter
//! arm + tests. The table is a FUNCTION rather than a static map so
//! generic receivers work naturally (`List<T>.first() -> Option<T>`
//! computes its return type from the receiver's element type).
//!
//! Method batches ship in dedicated slices: strings (45d),
//! conversions (45e), lists (45f), Option/Result helpers (45l),
//! `Grounded<T>` named unwraps (44f addendum). This slice ships the
//! machinery plus one pilot: `String.length()`.
//!
//! Grounded receivers are NOT auto-unwrapped by this table in 45c —
//! `Grounded<String>.length()` stays an error until the contagion
//! rule for method calls is decided alongside the Grounded unwrap
//! batch.

use crate::types::Type;
use serde::{Deserialize, Serialize};

/// Stable identity of a builtin method — the contract between the
/// checker, the lowerer, and the interpreter.
///
/// String-method semantics (slice 45d), decided once here:
/// - Indices and lengths count Unicode scalar values (Python's
///   `len(str)`), never UTF-8 bytes.
/// - `to_upper`/`to_lower` are full Unicode case mappings.
/// - `split` with an empty separator TRAPS at runtime (Python-like;
///   Rust's empty-pattern split yields surprising empty pieces).
/// - `replace` replaces ALL occurrences (Python/Rust behavior).
/// - `substring(start, end)` clamps out-of-range indices to the
///   string bounds and returns "" when start >= end (Python slice
///   behavior); negative indices are not supported in 45d.
/// - There is no `chars()` — strings are already `for c in s`
///   iterable at the statement level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinMethodKind {
    /// `String.length() -> Int` — the number of Unicode scalar
    /// values (code points), matching Python's `len(str)`, not the
    /// UTF-8 byte count.
    StringLength,
    /// `String.to_upper() -> String` — Unicode uppercase mapping.
    StringToUpper,
    /// `String.to_lower() -> String` — Unicode lowercase mapping.
    StringToLower,
    /// `String.trim() -> String` — strip Unicode whitespace from
    /// both ends.
    StringTrim,
    /// `String.contains(needle: String) -> Bool`.
    StringContains,
    /// `String.starts_with(prefix: String) -> Bool`.
    StringStartsWith,
    /// `String.ends_with(suffix: String) -> Bool`.
    StringEndsWith,
    /// `String.split(sep: String) -> List<String>` — empty separator
    /// traps at runtime.
    StringSplit,
    /// `String.replace(from: String, to: String) -> String` — every
    /// occurrence.
    StringReplace,
    /// `String.substring(start: Int, end: Int) -> String` — Unicode
    /// scalar indices, clamped to bounds; "" when start >= end.
    StringSubstring,

    // ----- conversions (slice 45e) --------------------------------
    //
    // Number→String rendering is Python-style: a Float ALWAYS shows
    // a decimal point or exponent (`42.0` → "42.0", never "42") so
    // string output round-trips visibly typed — these strings feed
    // LLM prompts and JSON. `to_int_truncated` truncates toward
    // zero and TRAPS on NaN / out-of-i64-range (the always-checked
    // arithmetic rule; no silent wrapping). The parse methods trim
    // ASCII whitespace first (Python's `int(" 42 ")` convenience)
    // and return `Result<_, String>` with the offending input named
    // in the Err.
    /// `Int.to_string() -> String`.
    IntToString,
    /// `Float.to_string() -> String` — always shows `.` or exponent.
    FloatToString,
    /// `Bool.to_string() -> String` — "true" / "false".
    BoolToString,
    /// `Int.to_float() -> Float` — exact for |n| <= 2^53, nearest
    /// otherwise.
    IntToFloat,
    /// `Float.to_int_truncated() -> Int` — toward zero; traps on
    /// NaN or out-of-range.
    FloatToIntTruncated,
    /// `String.parse_int() -> Result<Int, String>`.
    StringParseInt,
    /// `String.parse_float() -> Result<Float, String>`.
    StringParseFloat,

    // ----- lists (slice 45f) --------------------------------------
    //
    // `append`, `reverse`, and `sort` mutate IN PLACE and return
    // `Nothing` — coherent with the 45b reference-semantics decision
    // and Python's list API. `sort` is only offered where the
    // element type has a natural order (Int / Float / String; the
    // table returns no signature otherwise, so unsortable element
    // types get the standard "no builtin method" diagnostic).
    // Floats sort by IEEE total order (NaN last). `slice` clamps
    // like `substring` and returns a NEW list. `join` is offered on
    // `List<String>` only. `first`/`last` return `Option<T>` — the
    // first generic-return methods, computed from the receiver's
    // element type (the reason the table is a function).
    /// `List<T>.length() -> Int`.
    ListLength,
    /// `List<T>.append(item: T) -> Nothing` — in place.
    ListAppend,
    /// `List<T>.contains(item: T) -> Bool` — structural equality.
    ListContains,
    /// `List<T>.first() -> Option<T>`.
    ListFirst,
    /// `List<T>.last() -> Option<T>`.
    ListLast,
    /// `List<T>.slice(start: Int, end: Int) -> List<T>` — clamped,
    /// new list.
    ListSlice,
    /// `List<T>.reverse() -> Nothing` — in place.
    ListReverse,
    /// `List<T>.sort() -> Nothing` — in place; Int/Float/String
    /// elements only (table-gated).
    ListSort,
    /// `List<String>.join(sep: String) -> String`.
    ListJoin,

    /// `range(start: Int, end: Int) -> List<Int>` — the free builtin
    /// function (half-open, step 1, empty when start >= end). Lowered
    /// through the BuiltinMethod IR with `start` as the receiver so
    /// no new IR variant is needed.
    RangeIntList,
}

/// A builtin method's checked signature for a CONCRETE receiver
/// type. Parameter types are the declared types of the call's
/// arguments (the receiver is not included).
#[derive(Debug, Clone)]
pub struct BuiltinMethodSig {
    pub kind: BuiltinMethodKind,
    pub params: Vec<Type>,
    pub ret: Type,
}

/// Look up `receiver.name(...)` in the builtin-method table.
/// Returns `None` when no builtin method matches — the caller falls
/// through to extend-method dispatch or its existing diagnostics.
pub fn builtin_method(receiver: &Type, name: &str) -> Option<BuiltinMethodSig> {
    use BuiltinMethodKind::*;
    let sig = |kind, params, ret| Some(BuiltinMethodSig { kind, params, ret });
    match (receiver, name) {
        (Type::String, "length") => sig(StringLength, vec![], Type::Int),
        (Type::String, "to_upper") => sig(StringToUpper, vec![], Type::String),
        (Type::String, "to_lower") => sig(StringToLower, vec![], Type::String),
        (Type::String, "trim") => sig(StringTrim, vec![], Type::String),
        (Type::String, "contains") => sig(StringContains, vec![Type::String], Type::Bool),
        (Type::String, "starts_with") => sig(StringStartsWith, vec![Type::String], Type::Bool),
        (Type::String, "ends_with") => sig(StringEndsWith, vec![Type::String], Type::Bool),
        (Type::String, "split") => sig(
            StringSplit,
            vec![Type::String],
            Type::List(Box::new(Type::String)),
        ),
        (Type::String, "replace") => sig(
            StringReplace,
            vec![Type::String, Type::String],
            Type::String,
        ),
        (Type::String, "substring") => {
            sig(StringSubstring, vec![Type::Int, Type::Int], Type::String)
        }
        (Type::Int, "to_string") => sig(IntToString, vec![], Type::String),
        (Type::Float, "to_string") => sig(FloatToString, vec![], Type::String),
        (Type::Bool, "to_string") => sig(BoolToString, vec![], Type::String),
        (Type::Int, "to_float") => sig(IntToFloat, vec![], Type::Float),
        (Type::Float, "to_int_truncated") => sig(FloatToIntTruncated, vec![], Type::Int),
        (Type::String, "parse_int") => sig(
            StringParseInt,
            vec![],
            Type::Result(Box::new(Type::Int), Box::new(Type::String)),
        ),
        (Type::String, "parse_float") => sig(
            StringParseFloat,
            vec![],
            Type::Result(Box::new(Type::Float), Box::new(Type::String)),
        ),
        (Type::List(_), "length") => sig(ListLength, vec![], Type::Int),
        (Type::List(elem), "append") => sig(ListAppend, vec![(**elem).clone()], Type::Nothing),
        (Type::List(elem), "contains") => sig(ListContains, vec![(**elem).clone()], Type::Bool),
        (Type::List(elem), "first") => sig(ListFirst, vec![], Type::Option(elem.clone())),
        (Type::List(elem), "last") => sig(ListLast, vec![], Type::Option(elem.clone())),
        (Type::List(elem), "slice") => sig(
            ListSlice,
            vec![Type::Int, Type::Int],
            Type::List(elem.clone()),
        ),
        (Type::List(_), "reverse") => sig(ListReverse, vec![], Type::Nothing),
        (Type::List(elem), "sort")
            if matches!(**elem, Type::Int | Type::Float | Type::String) =>
        {
            sig(ListSort, vec![], Type::Nothing)
        }
        (Type::List(elem), "join") if matches!(**elem, Type::String) => {
            sig(ListJoin, vec![Type::String], Type::String)
        }
        _ => None,
    }
}
