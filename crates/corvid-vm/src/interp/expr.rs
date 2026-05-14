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
        UnaryOp::Not => {
            let b = require_bool(&v, span, "operand of `not`")?;
            Ok(Value::Bool(!b))
        }
    }
}

pub(super) fn require_bool(v: &Value, span: Span, context: &str) -> Result<bool, InterpError> {
    match v {
        Value::Bool(b) => Ok(*b),
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
}
