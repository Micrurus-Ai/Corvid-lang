use super::effect_compose::overflow;
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{GroundedValue, Value};
use corvid_ast::{BinaryOp, Span, UnaryOp};
use corvid_ir::IrLiteral;
use corvid_runtime::ProvenanceChain;
use std::sync::Arc;

pub(super) fn eval_literal(lit: &IrLiteral) -> Value {
    match lit {
        IrLiteral::Int(n) => Value::Int(*n),
        IrLiteral::Float(f) => Value::Float(*f),
        IrLiteral::String(s) => Value::String(Arc::from(s.as_str())),
        IrLiteral::Bool(b) => Value::Bool(*b),
        IrLiteral::Nothing => Value::Nothing,
    }
}

pub(super) fn eval_binop(
    op: BinaryOp,
    l: Value,
    r: Value,
    span: Span,
    wrapping: bool,
) -> Result<Value, InterpError> {
    use BinaryOp::*;

    // Provenance Propagation contagion law (D1) at the value level.
    // If either operand is grounded, lift the operation through
    // `Grounded`: strip the wrappers, run the normal operator on the
    // inner values, then re-wrap the result with the `Derived`
    // provenance chain (D3) and `Min`-composed confidence (D4).
    // `Grounded` is an applicative functor — every binary operator
    // lifts through it at this one site, so arithmetic, equality, and
    // ordering all propagate uniformly. `&&` / `||` never reach here
    // (short-circuited upstream), matching D1's scope.
    if matches!(l, Value::Grounded(_)) || matches!(r, Value::Grounded(_)) {
        let (l_inner, l_chain, l_conf) = unwrap_for_op(l);
        let (r_inner, r_chain, r_conf) = unwrap_for_op(r);
        let inner = eval_binop(op, l_inner, r_inner, span, wrapping)?;
        let provenance =
            ProvenanceChain::derived(binop_label(op), vec![l_chain, r_chain], 0);
        return Ok(Value::Grounded(GroundedValue::with_confidence(
            inner,
            provenance,
            l_conf.min(r_conf),
        )));
    }

    match op {
        Add | Sub | Mul | Div | Mod => eval_arithmetic(op, l, r, span, wrapping),
        Eq => Ok(Value::Bool(l == r)),
        NotEq => Ok(Value::Bool(l != r)),
        Lt | LtEq | Gt | GtEq => eval_ordering(op, l, r, span),
        And | Or => unreachable!("and/or is short-circuited upstream"),
    }
}

/// Fully strip `Grounded` wrappers from an operand for the contagion
/// lift in `eval_binop`. Returns the inner (un-grounded) value, the
/// provenance chain, and the confidence. An un-grounded operand
/// contributes an empty chain and confidence `1.0` (D4). Nested
/// `Grounded<Grounded<T>>` — which the type system prevents but which
/// this stays robust against — has its chains merged and confidences
/// `Min`-composed rather than producing a malformed value.
fn unwrap_for_op(v: Value) -> (Value, ProvenanceChain, f64) {
    match v {
        Value::Grounded(g) => {
            let (inner, mut chain, conf) = unwrap_for_op(g.inner.get());
            chain.merge(&g.provenance);
            (inner, chain, conf.min(g.confidence))
        }
        other => (other, ProvenanceChain::new(), 1.0),
    }
}

/// Stable operator label for the `Derived` provenance entry's `op`
/// field. Must stay byte-stable — it is recorded provenance and
/// feeds replay determinism.
fn binop_label(op: BinaryOp) -> &'static str {
    use BinaryOp::*;
    match op {
        Add => "add",
        Sub => "sub",
        Mul => "mul",
        Div => "div",
        Mod => "mod",
        Eq => "eq",
        NotEq => "ne",
        Lt => "lt",
        LtEq => "le",
        Gt => "gt",
        GtEq => "ge",
        And | Or => unreachable!("and/or is short-circuited upstream"),
    }
}

fn eval_arithmetic(
    op: BinaryOp,
    l: Value,
    r: Value,
    span: Span,
    wrapping: bool,
) -> Result<Value, InterpError> {
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(int_arith(op, a, b, span, wrapping)?)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(float_arith(op, a, b, span)?)),
        (Value::Int(a), Value::Float(b)) => {
            Ok(Value::Float(float_arith(op, a as f64, b, span)?))
        }
        (Value::Float(a), Value::Int(b)) => {
            Ok(Value::Float(float_arith(op, a, b as f64, span)?))
        }
        (Value::String(a), Value::String(b)) if matches!(op, BinaryOp::Add) => {
            let mut out = String::with_capacity(a.len() + b.len());
            out.push_str(&a);
            out.push_str(&b);
            Ok(Value::String(Arc::from(out)))
        }
        (Value::List(a), Value::List(b)) if matches!(op, BinaryOp::Add) => {
            let mut items = a.iter_cloned();
            items.extend(b.iter_cloned());
            Ok(Value::List(crate::value::ListValue::new(items)))
        }
        (a, b) => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "Int, Float, String, or List".into(),
                got: format!("{} and {}", a.type_name(), b.type_name()),
            },
            span,
        )),
    }
}

fn int_arith(
    op: BinaryOp,
    a: i64,
    b: i64,
    span: Span,
    wrapping: bool,
) -> Result<i64, InterpError> {
    use BinaryOp::*;
    match op {
        Add if wrapping => Ok(a.wrapping_add(b)),
        Add => a.checked_add(b).ok_or_else(|| overflow(span)),
        Sub if wrapping => Ok(a.wrapping_sub(b)),
        Sub => a.checked_sub(b).ok_or_else(|| overflow(span)),
        Mul if wrapping => Ok(a.wrapping_mul(b)),
        Mul => a.checked_mul(b).ok_or_else(|| overflow(span)),
        Div => {
            if b == 0 {
                Err(InterpError::new(
                    InterpErrorKind::Arithmetic("division by zero".into()),
                    span,
                ))
            } else {
                Ok(a.wrapping_div(b))
            }
        }
        Mod => {
            if b == 0 {
                Err(InterpError::new(
                    InterpErrorKind::Arithmetic("modulo by zero".into()),
                    span,
                ))
            } else {
                Ok(a.wrapping_rem(b))
            }
        }
        _ => unreachable!("non-arithmetic op routed here"),
    }
}

fn float_arith(op: BinaryOp, a: f64, b: f64, _span: Span) -> Result<f64, InterpError> {
    // Float arithmetic follows IEEE 754: `1.0 / 0.0 = +Inf`, `0.0 / 0.0
    // = NaN`, `Inf - Inf = NaN`. NaN propagation is the platform's
    // safety story for floats — telling callers "something went wrong
    // upstream" without aborting. Int arithmetic still traps on
    // overflow / div-by-zero because integers have no defined `Inf`.
    use BinaryOp::*;
    Ok(match op {
        Add => a + b,
        Sub => a - b,
        Mul => a * b,
        Div => a / b,
        Mod => a % b,
        _ => unreachable!("non-arithmetic op routed here"),
    })
}

fn eval_ordering(op: BinaryOp, l: Value, r: Value, span: Span) -> Result<Value, InterpError> {
    use BinaryOp::*;
    let ordering_result = |a: f64, b: f64| -> bool {
        match op {
            Lt => a < b,
            LtEq => a <= b,
            Gt => a > b,
            GtEq => a >= b,
            _ => unreachable!("non-ordering op routed here"),
        }
    };
    match (l, r) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(match op {
            Lt => a < b,
            LtEq => a <= b,
            Gt => a > b,
            GtEq => a >= b,
            _ => unreachable!(),
        })),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(ordering_result(a, b))),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool(ordering_result(a as f64, b))),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(ordering_result(a, b as f64))),
        (Value::String(a), Value::String(b)) => Ok(Value::Bool(match op {
            Lt => a.as_ref() < b.as_ref(),
            LtEq => a.as_ref() <= b.as_ref(),
            Gt => a.as_ref() > b.as_ref(),
            GtEq => a.as_ref() >= b.as_ref(),
            _ => unreachable!(),
        })),
        (a, b) => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "orderable (Int / Float / String)".into(),
                got: format!("{} and {}", a.type_name(), b.type_name()),
            },
            span,
        )),
    }
}

pub(super) fn eval_unop(
    op: UnaryOp,
    v: Value,
    span: Span,
    wrapping: bool,
) -> Result<Value, InterpError> {
    match op {
        UnaryOp::Neg => match v {
            Value::Int(n) if wrapping => Ok(Value::Int(n.wrapping_neg())),
            Value::Int(n) => n.checked_neg().map(Value::Int).ok_or_else(|| overflow(span)),
            Value::Float(f) => Ok(Value::Float(-f)),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Int or Float".into(),
                    got: other.type_name(),
                },
                span,
            )),
        },
        // Elided at IR lowering (numeric identity); kept for match
        // exhaustiveness.
        UnaryOp::Pos => Ok(v),
        UnaryOp::Not => {
            let b = require_bool(&v, span, "operand of `not`")?;
            Ok(Value::Bool(!b))
        }
    }
}

pub(super) fn require_bool(v: &Value, span: Span, context: &str) -> Result<bool, InterpError> {
    // D2 Provenance Propagation: control-flow conditions and other
    // bool-consumption sites accept `Value::Grounded<Bool>` —
    // branching/asserting consumes the bool to pick a path or
    // produce a verdict, it does not emit a laundered value, so
    // unwrapping to the inner bool is sound. Recursive strip handles
    // nested grounding (which the type system prevents but which
    // this stays robust against). The unwrap is recorded + IR-visible
    // when the D5 discard node lands (slice 7).
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Grounded(g) => {
            let inner = g.inner.get();
            require_bool(&inner, span, context)
        }
        other => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: format!("Bool for {context}"),
                got: other.type_name(),
            },
            span,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::ProvenanceKind;

    fn span() -> Span {
        Span::new(0, 0)
    }

    /// A `Grounded<Int>` value carrying a single retrieval source and
    /// the given confidence — the runtime shape `data: grounded`
    /// calls produce.
    fn grounded_int(n: i64, source: &str, confidence: f64) -> Value {
        Value::Grounded(GroundedValue::with_confidence(
            Value::Int(n),
            ProvenanceChain::with_retrieval(source, 1),
            confidence,
        ))
    }

    #[test]
    fn grounded_operand_makes_the_arithmetic_result_grounded() {
        // D1 contagion at the value level: `Grounded<Int> + Int`
        // evaluates to `Grounded<Int>`. Before slice 4 this was the
        // original `TypeMismatch` crash that opened the whole phase.
        let result = eval_binop(BinaryOp::Add, grounded_int(40, "search", 0.9), Value::Int(2), span(), false)
            .expect("grounded arithmetic must not error");
        let Value::Grounded(g) = result else {
            panic!("expected a Grounded result, got {result:?}");
        };
        // The arithmetic ran on the unwrapped operands.
        assert_eq!(g.inner.get(), Value::Int(42));
        // D3: a single `Derived` entry recording the op + operand chains.
        assert_eq!(g.provenance.entries.len(), 1);
        match &g.provenance.entries[0].kind {
            ProvenanceKind::Derived { op, inputs } => {
                assert_eq!(op, "add");
                assert_eq!(inputs.len(), 2);
                // Left operand grounded -> its source survives; right
                // operand was a plain `Int` -> empty input chain.
                assert!(inputs[0].has_source("search"));
                assert!(inputs[1].entries.is_empty());
            }
            other => panic!("expected Derived, got {other:?}"),
        }
        // D4: Min(0.9 grounded, 1.0 ungrounded) = 0.9.
        assert_eq!(g.confidence, 0.9);
    }

    #[test]
    fn both_operands_grounded_merge_into_one_derived_tree() {
        let result = eval_binop(
            BinaryOp::Mul,
            grounded_int(6, "left_src", 0.8),
            grounded_int(7, "right_src", 0.95),
            span(),
            false,
        )
        .expect("eval");
        let Value::Grounded(g) = result else {
            panic!("expected Grounded");
        };
        assert_eq!(g.inner.get(), Value::Int(42));
        match &g.provenance.entries[0].kind {
            ProvenanceKind::Derived { op, inputs } => {
                assert_eq!(op, "mul");
                assert!(inputs[0].has_source("left_src"));
                assert!(inputs[1].has_source("right_src"));
            }
            other => panic!("expected Derived, got {other:?}"),
        }
        // D4: Min(0.8, 0.95) = 0.8.
        assert_eq!(g.confidence, 0.8);
    }

    #[test]
    fn grounded_comparison_yields_a_grounded_bool() {
        // D1: a comparison with a grounded operand is `Grounded<Bool>`.
        let result = eval_binop(BinaryOp::Lt, grounded_int(1, "src", 1.0), Value::Int(2), span(), false)
            .expect("eval");
        let Value::Grounded(g) = result else {
            panic!("expected Grounded<Bool>");
        };
        assert_eq!(g.inner.get(), Value::Bool(true));
        assert!(matches!(
            g.provenance.entries[0].kind,
            ProvenanceKind::Derived { .. }
        ));
    }

    #[test]
    fn ungrounded_operands_stay_ungrounded() {
        // The contagion lift fires only when an operand is grounded —
        // ordinary arithmetic is untouched.
        let result =
            eval_binop(BinaryOp::Add, Value::Int(2), Value::Int(3), span(), false).expect("eval");
        assert_eq!(result, Value::Int(5));
    }

    /// A `Grounded<Bool>` value carrying a single retrieval source.
    fn grounded_bool(b: bool, source: &str) -> Value {
        Value::Grounded(GroundedValue::with_confidence(
            Value::Bool(b),
            ProvenanceChain::with_retrieval(source, 1),
            1.0,
        ))
    }

    #[test]
    fn require_bool_strips_grounded_for_control_flow() {
        // D2 Provenance Propagation: a control-flow condition (`if`,
        // assert) accepts `Value::Grounded<Bool>` — `require_bool`
        // strips to the inner bool so the branch decision proceeds.
        // Without slice 6 this was a `TypeMismatch` "expected Bool"
        // crash at runtime.
        let span = span();
        assert_eq!(
            require_bool(&grounded_bool(true, "src"), span, "test").unwrap(),
            true
        );
        assert_eq!(
            require_bool(&grounded_bool(false, "src"), span, "test").unwrap(),
            false
        );
        // Plain `Value::Bool` still works (regression guard).
        assert_eq!(require_bool(&Value::Bool(true), span, "test").unwrap(), true);
        // Non-bool still errors (the strip is grounding-only, not
        // type-coercion).
        assert!(require_bool(&Value::Int(1), span, "test").is_err());
    }
}

/// Execute a builtin method (slices 45c/45d) on already-evaluated
/// receiver and argument values. Pure value operations — semantics
/// documented on `corvid_types::BuiltinMethodKind`. The checker
/// guarantees receiver/argument types, so mismatches here indicate
/// an internal inconsistency and surface as typed runtime errors.
/// Python-style float rendering: always shows a decimal point
/// or exponent so string output round-trips visibly typed.
fn render_float(f: f64) -> String {
    let rendered = format!("{f}");
    if rendered.contains('.')
        || rendered.contains('e')
        || rendered.contains('E')
        || rendered.contains("inf")
        || rendered.contains("NaN")
    {
        rendered
    } else {
        format!("{rendered}.0")
    }
}

/// Checked Float->Int conversion shared by `floor`/`ceil`/`round`
/// (slice 45m) — the same always-checked rule as
/// `to_int_truncated`: NaN and out-of-i64-range TRAP.
fn float_to_int_checked(f: f64, method: &str, span: Span) -> Result<Value, InterpError> {
    if f.is_nan() || f < (i64::MIN as f64) || f >= (i64::MAX as f64) {
        return Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: format!("a Float within Int range for `{method}`"),
                got: format!("`{}`", render_float(f)),
            },
            span,
        ));
    }
    Ok(Value::Int(f as i64))
}

pub(super) fn eval_builtin_method(
    kind: corvid_types::BuiltinMethodKind,
    recv: Value,
    mut args: Vec<Value>,
    span: Span,
) -> Result<Value, InterpError> {
    use corvid_types::BuiltinMethodKind::*;

    fn want_string(v: &Value, span: Span) -> Result<std::sync::Arc<str>, InterpError> {
        match v {
            Value::String(s) => Ok(s.clone()),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "String".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }
    fn want_int(v: &Value, span: Span) -> Result<i64, InterpError> {
        match v {
            Value::Int(i) => Ok(*i),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Int".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }

    fn want_float(v: &Value, span: Span) -> Result<f64, InterpError> {
        match v {
            Value::Float(f) => Ok(*f),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Float".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }
    fn want_string_recv(
        v: &Value,
        span: Span,
    ) -> Result<std::sync::Arc<str>, InterpError> {
        match v {
            Value::String(s) => Ok(s.clone()),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "String".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }
    match kind {
        IntToString => Ok(Value::String(std::sync::Arc::from(
            want_int(&recv, span)?.to_string(),
        ))),
        FloatToString => Ok(Value::String(std::sync::Arc::from(render_float(
            want_float(&recv, span)?,
        )))),
        BoolToString => match &recv {
            Value::Bool(b) => Ok(Value::String(std::sync::Arc::from(
                if *b { "true" } else { "false" },
            ))),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Bool".into(),
                    got: other.type_name(),
                },
                span,
            )),
        },
        IntToFloat => Ok(Value::Float(want_int(&recv, span)? as f64)),
        FloatToIntTruncated => {
            let f = want_float(&recv, span)?;
            // Always-checked rule: NaN and out-of-range trap rather
            // than silently wrapping.
            if f.is_nan() || f < (i64::MIN as f64) || f >= (i64::MAX as f64) {
                return Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "a Float within Int range for `to_int_truncated`".into(),
                        got: format!("`{}`", render_float(f)),
                    },
                    span,
                ));
            }
            Ok(Value::Int(f.trunc() as i64))
        }
        StringParseInt => {
            let s = want_string_recv(&recv, span)?;
            match s.trim().parse::<i64>() {
                Ok(n) => Ok(Value::ResultOk(crate::value::BoxedValue::new(Value::Int(n)))),
                Err(_) => Ok(Value::ResultErr(crate::value::BoxedValue::new(
                    Value::String(std::sync::Arc::from(format!(
                        "not an integer: `{s}`"
                    ))),
                ))),
            }
        }
        StringParseFloat => {
            let s = want_string_recv(&recv, span)?;
            match s.trim().parse::<f64>() {
                Ok(f) => Ok(Value::ResultOk(crate::value::BoxedValue::new(
                    Value::Float(f),
                ))),
                Err(_) => Ok(Value::ResultErr(crate::value::BoxedValue::new(
                    Value::String(std::sync::Arc::from(format!(
                        "not a number: `{s}`"
                    ))),
                ))),
            }
        }
        ListLength
        | ListAppend
        | ListContains
        | ListFirst
        | ListLast
        | ListSlice
        | ListReverse
        | ListSort
        | ListJoin => {
            let lv = match &recv {
                Value::List(l) => l.clone(),
                other => {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "List".into(),
                            got: other.type_name(),
                        },
                        span,
                    ))
                }
            };
            eval_list_method(kind, lv, args, span)
        }
        MapLength | MapGet | MapContainsKey | MapKeys | MapValues | MapRemove => {
            let mv = match &recv {
                Value::Map(m) => m.clone(),
                other => {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Map".into(),
                            got: other.type_name(),
                        },
                        span,
                    ))
                }
            };
            let mut args = args;
            Ok(match kind {
                MapLength => Value::Int(mv.len() as i64),
                MapGet => match mv.get_by_key(&args[0]) {
                    Some(v) => Value::OptionSome(crate::value::BoxedValue::new(v)),
                    None => Value::OptionNone,
                },
                MapContainsKey => Value::Bool(mv.get_by_key(&args[0]).is_some()),
                MapKeys => Value::List(crate::value::ListValue::new(mv.keys_cloned())),
                MapValues => Value::List(crate::value::ListValue::new(mv.values_cloned())),
                MapRemove => match mv.remove(&args.remove(0)) {
                    Some(v) => Value::OptionSome(crate::value::BoxedValue::new(v)),
                    None => Value::OptionNone,
                },
                _ => unreachable!("gated above"),
            })
        }
        // Math (45m): checked Int methods, IEEE Float methods,
        // trapping Float->Int conversions (same rule as
        // `to_int_truncated`).
        IntAbs => {
            let n = want_int(&recv, span)?;
            n.checked_abs().map(Value::Int).ok_or_else(|| {
                InterpError::new(
                    InterpErrorKind::Arithmetic(
                        "abs() overflows on the minimum Int".into(),
                    ),
                    span,
                )
            })
        }
        IntMin => Ok(Value::Int(want_int(&recv, span)?.min(want_int(&args[0], span)?))),
        IntMax => Ok(Value::Int(want_int(&recv, span)?.max(want_int(&args[0], span)?))),
        IntPow => {
            let base = want_int(&recv, span)?;
            let exp = want_int(&args[0], span)?;
            if exp < 0 {
                return Err(InterpError::new(
                    InterpErrorKind::Arithmetic(
                        "pow() with a negative exponent — use to_float().pow(...) for roots"
                            .into(),
                    ),
                    span,
                ));
            }
            u32::try_from(exp)
                .ok()
                .and_then(|e| base.checked_pow(e))
                .map(Value::Int)
                .ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::Arithmetic(format!(
                            "pow() overflows: {base}^{exp} does not fit in Int"
                        )),
                        span,
                    )
                })
        }
        FloatAbs => Ok(Value::Float(want_float(&recv, span)?.abs())),
        FloatMin => Ok(Value::Float(
            want_float(&recv, span)?.min(want_float(&args[0], span)?),
        )),
        FloatMax => Ok(Value::Float(
            want_float(&recv, span)?.max(want_float(&args[0], span)?),
        )),
        FloatPow => Ok(Value::Float(
            want_float(&recv, span)?.powf(want_float(&args[0], span)?),
        )),
        FloatSqrt => {
            let x = want_float(&recv, span)?;
            if x < 0.0 {
                return Err(InterpError::new(
                    InterpErrorKind::Arithmetic(format!(
                        "sqrt() of a negative number ({x}) — a silent NaN would poison downstream arithmetic"
                    )),
                    span,
                ));
            }
            Ok(Value::Float(x.sqrt()))
        }
        FloatFloor => float_to_int_checked(want_float(&recv, span)?.floor(), "floor", span),
        FloatCeil => float_to_int_checked(want_float(&recv, span)?.ceil(), "ceil", span),
        FloatRound => float_to_int_checked(want_float(&recv, span)?.round(), "round", span),

        // Option / Result ergonomics (45l). `unwrap_or` returns the
        // payload or the default; `ok_or` converts the envelope.
        OptionUnwrapOr => Ok(match recv {
            Value::OptionSome(v) => v.get(),
            Value::OptionNone => args.remove(0),
            other => {
                return Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "Option".into(),
                        got: other.type_name(),
                    },
                    span,
                ))
            }
        }),
        OptionIsSome => Ok(Value::Bool(matches!(recv, Value::OptionSome(_)))),
        OptionIsNone => Ok(Value::Bool(matches!(recv, Value::OptionNone))),
        OptionOkOr => Ok(match recv {
            Value::OptionSome(v) => Value::ResultOk(crate::value::BoxedValue::new(v.get())),
            Value::OptionNone => Value::ResultErr(crate::value::BoxedValue::new(args.remove(0))),
            other => {
                return Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "Option".into(),
                        got: other.type_name(),
                    },
                    span,
                ))
            }
        }),
        ResultUnwrapOr => Ok(match recv {
            Value::ResultOk(v) => v.get(),
            Value::ResultErr(_) => args.remove(0),
            other => {
                return Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "Result".into(),
                        got: other.type_name(),
                    },
                    span,
                ))
            }
        }),
        ResultIsOk => Ok(Value::Bool(matches!(recv, Value::ResultOk(_)))),
        ResultIsErr => Ok(Value::Bool(matches!(recv, Value::ResultErr(_)))),

        // The lambda-taking methods (45j) are intercepted in the
        // ASYNC evaluator (applying a closure re-enters eval); this
        // arm is a loud backstop, not a code path.
        ListMap | ListFilter | ListFold | ListAny | ListAll | ResultMapErr => Err(InterpError::new(
            InterpErrorKind::DispatchFailed(
                "higher-order list methods dispatch through the async evaluator".into(),
            ),
            span,
        )),
        RangeIntList => {
            let start = want_int(&recv, span)?;
            let end = want_int(&args[0], span)?;
            let items: Vec<Value> = (start..end).map(Value::Int).collect();
            Ok(Value::List(crate::value::ListValue::new(items)))
        }
        _ => {
            let s = want_string(&recv, span)?;
            eval_string_method(kind, s, args, span)
        }
    }
}

/// The list-receiver methods (slice 45f). `append`/`reverse`/`sort`
/// mutate the shared cell in place and return `Nothing` (the 45b
/// reference-semantics rule: every alias sees the mutation).
fn eval_list_method(
    kind: corvid_types::BuiltinMethodKind,
    lv: crate::value::ListValue,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, InterpError> {
    use corvid_types::BuiltinMethodKind::*;

    fn want_int(v: &Value, span: Span) -> Result<i64, InterpError> {
        match v {
            Value::Int(i) => Ok(*i),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Int".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }

    match kind {
        ListLength => Ok(Value::Int(lv.len() as i64)),
        ListAppend => {
            lv.push(args.into_iter().next().expect("checked arity"));
            Ok(Value::Nothing)
        }
        ListContains => {
            let needle = &args[0];
            let found = lv.iter_cloned().iter().any(|v| v == needle);
            Ok(Value::Bool(found))
        }
        ListFirst => Ok(match lv.get(0) {
            Some(v) => Value::OptionSome(crate::value::BoxedValue::new(v)),
            None => Value::OptionNone,
        }),
        ListLast => {
            let len = lv.len();
            Ok(if len == 0 {
                Value::OptionNone
            } else {
                match lv.get(len - 1) {
                    Some(v) => Value::OptionSome(crate::value::BoxedValue::new(v)),
                    None => Value::OptionNone,
                }
            })
        }
        ListSlice => {
            let len = lv.len() as i64;
            let start = want_int(&args[0], span)?.clamp(0, len) as usize;
            let end = want_int(&args[1], span)?.clamp(0, len) as usize;
            let items = if start >= end {
                Vec::new()
            } else {
                lv.iter_cloned()[start..end].to_vec()
            };
            Ok(Value::List(crate::value::ListValue::new(items)))
        }
        ListReverse => {
            lv.reverse_in_place();
            Ok(Value::Nothing)
        }
        ListSort => {
            lv.sort_in_place_by(|a, b| match (a, b) {
                (Value::Int(x), Value::Int(y)) => x.cmp(y),
                (Value::Float(x), Value::Float(y)) => x.total_cmp(y),
                (Value::String(x), Value::String(y)) => x.cmp(y),
                // Mixed/unsupported elements keep their order; the
                // checker gates sort() to Int/Float/String lists.
                _ => std::cmp::Ordering::Equal,
            });
            Ok(Value::Nothing)
        }
        ListJoin => {
            let sep = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "String".into(),
                            got: other.type_name(),
                        },
                        span,
                    ))
                }
            };
            let mut pieces: Vec<String> = Vec::with_capacity(lv.len());
            for v in lv.iter_cloned() {
                match v {
                    Value::String(s) => pieces.push(s.to_string()),
                    other => {
                        return Err(InterpError::new(
                            InterpErrorKind::TypeMismatch {
                                expected: "String".into(),
                                got: other.type_name(),
                            },
                            span,
                        ))
                    }
                }
            }
            Ok(Value::String(std::sync::Arc::from(
                pieces.join(sep.as_ref()),
            )))
        }
        other => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "a list-receiver builtin method".into(),
                got: format!("{other:?}"),
            },
            span,
        )),
    }
}

/// The string-receiver methods (slice 45d), split out so the
/// conversion arms above can dispatch on other receiver types.
fn eval_string_method(
    kind: corvid_types::BuiltinMethodKind,
    s: std::sync::Arc<str>,
    args: Vec<Value>,
    span: Span,
) -> Result<Value, InterpError> {
    use corvid_types::BuiltinMethodKind::*;

    fn want_string(v: &Value, span: Span) -> Result<std::sync::Arc<str>, InterpError> {
        match v {
            Value::String(s) => Ok(s.clone()),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "String".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }
    fn want_int(v: &Value, span: Span) -> Result<i64, InterpError> {
        match v {
            Value::Int(i) => Ok(*i),
            other => Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "Int".into(),
                    got: other.type_name(),
                },
                span,
            )),
        }
    }

    match kind {
        StringLength => Ok(Value::Int(s.chars().count() as i64)),
        StringToUpper => Ok(Value::String(std::sync::Arc::from(s.to_uppercase()))),
        StringToLower => Ok(Value::String(std::sync::Arc::from(s.to_lowercase()))),
        StringTrim => Ok(Value::String(std::sync::Arc::from(s.trim()))),
        StringContains => {
            let needle = want_string(&args[0], span)?;
            Ok(Value::Bool(s.contains(needle.as_ref())))
        }
        StringStartsWith => {
            let prefix = want_string(&args[0], span)?;
            Ok(Value::Bool(s.starts_with(prefix.as_ref())))
        }
        StringEndsWith => {
            let suffix = want_string(&args[0], span)?;
            Ok(Value::Bool(s.ends_with(suffix.as_ref())))
        }
        StringSplit => {
            let sep = want_string(&args[0], span)?;
            if sep.is_empty() {
                return Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "a non-empty separator for `split` (iterate the string with `for c in s` to walk characters)".into(),
                        got: "an empty String".into(),
                    },
                    span,
                ));
            }
            let pieces: Vec<Value> = s
                .split(sep.as_ref())
                .map(|piece| Value::String(std::sync::Arc::from(piece)))
                .collect();
            Ok(Value::List(crate::value::ListValue::new(pieces)))
        }
        StringReplace => {
            let from = want_string(&args[0], span)?;
            let to = want_string(&args[1], span)?;
            Ok(Value::String(std::sync::Arc::from(s.replace(from.as_ref(), to.as_ref()))))
        }
        StringSubstring => {
            let len = s.chars().count() as i64;
            let start = want_int(&args[0], span)?.clamp(0, len) as usize;
            let end = want_int(&args[1], span)?.clamp(0, len) as usize;
            if start >= end {
                return Ok(Value::String(std::sync::Arc::from(String::new())));
            }
            let piece: String = s.chars().skip(start).take(end - start).collect();
            Ok(Value::String(std::sync::Arc::from(piece)))
        }
        // Conversion kinds are handled by the caller before
        // delegating here; reaching this arm is an internal
        // dispatch bug.
        other => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "a string-receiver builtin method".into(),
                got: format!("{other:?}"),
            },
            span,
        )),
    }
}
