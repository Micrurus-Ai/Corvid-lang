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
use corvid_ast::Effect;
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

    // ----- maps (slice 45g) --------------------------------------
    //
    // `m[k] = v` is the insert-or-update path (place assignment);
    // `get` is the method spelling of the `m[k]` Option read.
    // `remove` returns the removed value as Option<V>. `keys` /
    // `values` return snapshot lists in insertion order.
    /// `Map<K,V>.length() -> Int`.
    MapLength,
    /// `Map<K,V>.get(key: K) -> Option<V>`.
    MapGet,
    /// `Map<K,V>.contains_key(key: K) -> Bool`.
    MapContainsKey,
    /// `Map<K,V>.keys() -> List<K>` — insertion order.
    MapKeys,
    /// `Map<K,V>.values() -> List<V>` — insertion order.
    MapValues,
    /// `Map<K,V>.remove(key: K) -> Option<V>`.
    MapRemove,

    // ----- higher-order list methods (slice 45j) ------------------
    //
    // The lambda-taking batch. `map`'s result element type and
    // `fold`'s accumulator type cannot come from the receiver alone,
    // so the CHECKER refines these signatures while checking the
    // arguments in order: `fold`'s `init` argument fixes `U` in both
    // the lambda parameter and the result; `map`'s checked lambda
    // return type fixes the result element type. The interpreter
    // applies the closure once per element, left to right; `any` /
    // `all` short-circuit.
    /// `List<T>.map(f: (T) -> U) -> List<U>`.
    ListMap,
    /// `List<T>.filter(pred: (T) -> Bool) -> List<T>`.
    ListFilter,
    /// `List<T>.fold(init: U, f: (U, T) -> U) -> U`.
    ListFold,
    /// `List<T>.any(pred: (T) -> Bool) -> Bool` — short-circuits.
    ListAny,
    /// `List<T>.all(pred: (T) -> Bool) -> Bool` — short-circuits.
    ListAll,

    // ----- math (slice 45m) ----------------------------------------
    //
    // Pure numeric methods under the always-checked rule: Int
    // methods TRAP instead of wrapping (`abs` on i64::MIN, `pow`
    // overflow or negative exponent); Float->Int conversions
    // (`floor`/`ceil`/`round`) trap on NaN or out-of-i64-range,
    // exactly like `to_int_truncated`. `round` is half-AWAY-FROM-
    // ZERO (2.5 -> 3, -2.5 -> -3) — deliberately not Python's
    // half-to-even, which surprises far more readers than it
    // helps. Float `min`/`max` follow IEEE/Rust: a NaN operand
    // loses. `sqrt` of a negative traps (a silent NaN would poison
    // downstream arithmetic invisibly). All are pure — replay
    // stays deterministic; the effectful numeric source (`random`)
    // lives in std/random.cor as a TOOL.
    /// `Int.abs() -> Int` — traps on i64::MIN.
    IntAbs,
    /// `Int.min(other: Int) -> Int`.
    IntMin,
    /// `Int.max(other: Int) -> Int`.
    IntMax,
    /// `Int.pow(exp: Int) -> Int` — checked; traps on overflow or
    /// a negative exponent (use `to_float().pow(...)` for roots).
    IntPow,
    /// `Float.abs() -> Float`.
    FloatAbs,
    /// `Float.min(other: Float) -> Float` — NaN operand loses.
    FloatMin,
    /// `Float.max(other: Float) -> Float` — NaN operand loses.
    FloatMax,
    /// `Float.pow(exp: Float) -> Float`.
    FloatPow,
    /// `Float.sqrt() -> Float` — traps on negative input.
    FloatSqrt,
    /// `Float.floor() -> Int` — traps on NaN / out-of-range.
    FloatFloor,
    /// `Float.ceil() -> Int` — traps on NaN / out-of-range.
    FloatCeil,
    /// `Float.round() -> Int` — half away from zero; traps on
    /// NaN / out-of-range.
    FloatRound,

    // ----- Option / Result ergonomics (slice 45l) -----------------
    //
    // The point-of-use shorthands: defaulting an absent Option,
    // asking which side a Result landed on, and converting between
    // the two envelopes without a full `match`. `ok_or`'s error
    // type comes from its argument and `map_err`'s from its
    // lambda's checked return type — the same sequential signature
    // refinement `map`/`fold` use. All are pure; `map_err` applies
    // its closure at most once (only on the Err side).
    /// `Option<T>.unwrap_or(default: T) -> T`.
    OptionUnwrapOr,
    /// `Option<T>.is_some() -> Bool`.
    OptionIsSome,
    /// `Option<T>.is_none() -> Bool`.
    OptionIsNone,
    /// `Option<T>.ok_or(err: E) -> Result<T, E>` — Some(v) becomes
    /// Ok(v), None becomes Err(err).
    OptionOkOr,
    /// `Result<T, E>.unwrap_or(default: T) -> T`.
    ResultUnwrapOr,
    /// `Result<T, E>.is_ok() -> Bool`.
    ResultIsOk,
    /// `Result<T, E>.is_err() -> Bool`.
    ResultIsErr,
    /// `Result<T, E>.map_err(f: (E) -> F) -> Result<T, F>` — Ok
    /// passes through untouched; the closure runs only on Err.
    ResultMapErr,

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
    let func = |params: Vec<Type>, ret: Type| Type::Function {
        params,
        ret: Box::new(ret),
        effect: Effect::Safe,
    };
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
        (Type::List(elem), "map") => sig(
            ListMap,
            vec![func(vec![(**elem).clone()], Type::Unknown)],
            Type::List(Box::new(Type::Unknown)),
        ),
        (Type::List(elem), "filter") => sig(
            ListFilter,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::List(elem.clone()),
        ),
        (Type::List(elem), "fold") => sig(
            ListFold,
            vec![
                Type::Unknown,
                func(vec![Type::Unknown, (**elem).clone()], Type::Unknown),
            ],
            Type::Unknown,
        ),
        (Type::List(elem), "any") => sig(
            ListAny,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::Bool,
        ),
        (Type::List(elem), "all") => sig(
            ListAll,
            vec![func(vec![(**elem).clone()], Type::Bool)],
            Type::Bool,
        ),
        (Type::Int, "abs") => sig(IntAbs, vec![], Type::Int),
        (Type::Int, "min") => sig(IntMin, vec![Type::Int], Type::Int),
        (Type::Int, "max") => sig(IntMax, vec![Type::Int], Type::Int),
        (Type::Int, "pow") => sig(IntPow, vec![Type::Int], Type::Int),
        (Type::Float, "abs") => sig(FloatAbs, vec![], Type::Float),
        (Type::Float, "min") => sig(FloatMin, vec![Type::Float], Type::Float),
        (Type::Float, "max") => sig(FloatMax, vec![Type::Float], Type::Float),
        (Type::Float, "pow") => sig(FloatPow, vec![Type::Float], Type::Float),
        (Type::Float, "sqrt") => sig(FloatSqrt, vec![], Type::Float),
        (Type::Float, "floor") => sig(FloatFloor, vec![], Type::Int),
        (Type::Float, "ceil") => sig(FloatCeil, vec![], Type::Int),
        (Type::Float, "round") => sig(FloatRound, vec![], Type::Int),
        (Type::Option(inner), "unwrap_or") => {
            sig(OptionUnwrapOr, vec![(**inner).clone()], (**inner).clone())
        }
        (Type::Option(_), "is_some") => sig(OptionIsSome, vec![], Type::Bool),
        (Type::Option(_), "is_none") => sig(OptionIsNone, vec![], Type::Bool),
        (Type::Option(inner), "ok_or") => sig(
            OptionOkOr,
            vec![Type::Unknown],
            Type::Result(inner.clone(), Box::new(Type::Unknown)),
        ),
        (Type::Result(ok, _), "unwrap_or") => {
            sig(ResultUnwrapOr, vec![(**ok).clone()], (**ok).clone())
        }
        (Type::Result(_, _), "is_ok") => sig(ResultIsOk, vec![], Type::Bool),
        (Type::Result(_, _), "is_err") => sig(ResultIsErr, vec![], Type::Bool),
        (Type::Result(ok, err), "map_err") => sig(
            ResultMapErr,
            vec![func(vec![(**err).clone()], Type::Unknown)],
            Type::Result(ok.clone(), Box::new(Type::Unknown)),
        ),
        (Type::Map(_, _), "length") => sig(MapLength, vec![], Type::Int),
        (Type::Map(k, v), "get") => sig(MapGet, vec![(**k).clone()], Type::Option(v.clone())),
        (Type::Map(k, _), "contains_key") => {
            sig(MapContainsKey, vec![(**k).clone()], Type::Bool)
        }
        (Type::Map(k, _), "keys") => sig(MapKeys, vec![], Type::List(k.clone())),
        (Type::Map(_, v), "values") => sig(MapValues, vec![], Type::List(v.clone())),
        (Type::Map(k, v), "remove") => {
            sig(MapRemove, vec![(**k).clone()], Type::Option(v.clone()))
        }
        _ => None,
    }
}
