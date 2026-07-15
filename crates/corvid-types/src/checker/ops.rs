//! Operator and try-form expression checks.
//!
//! Covers the subexpression shapes that aren't primary values or
//! calls: binary operators (arithmetic + comparison + logical),
//! unary operators (negation + boolean not), and the two `try`
//! forms (`expr?` propagation and `try expr on error retry …`).
//!
//! Extracted from `checker.rs` as part of Phase 20i responsibility
//! decomposition. All four methods extend the `Checker` impl in a
//! sibling submodule.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{BinaryOp, Expr, Span, UnaryOp};

impl<'a> Checker<'a> {
    pub(super) fn check_binop(&mut self, op: BinaryOp, l: &Expr, r: &Expr, _span: Span) -> Type {
        let lt = self.check_expr(l);
        let rt = self.check_expr(r);
        use BinaryOp::*;

        // `&&` / `||` short-circuit; the Provenance Propagation design
        // (D1) scopes them out of the contagion law. Original logic,
        // operating on the un-stripped operand types.
        if matches!(op, And | Or) {
            if !matches!(lt, Type::Bool | Type::Unknown) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "Bool".into(),
                        got: lt.display_name(),
                        context: "logical operator".into(),
                    },
                    l.span(),
                ));
            }
            if !matches!(rt, Type::Bool | Type::Unknown) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "Bool".into(),
                        got: rt.display_name(),
                        context: "logical operator".into(),
                    },
                    r.span(),
                ));
            }
            return Type::Bool;
        }

        // D1 part B — the contagion law. `Grounded<T>` is an applicative
        // functor: strip the `Grounded<>` wrapper(s) from the operands,
        // run the operator's normal type rule on the inner types, then
        // re-wrap the result if either operand was grounded.
        //
        // Dormant until slice 2b makes `data: grounded` produce
        // `Type::Grounded`. No program today carries an effect-grounded
        // type into an operator, and an explicit `Grounded<T>`
        // annotation flowing into an operator was a hard type error
        // before this slice — so for the current corpus this is purely
        // additive: it never changes an existing acceptance, only adds
        // new ones.
        let contagious = matches!(lt, Type::Grounded(_)) || matches!(rt, Type::Grounded(_));
        let lt = lt.ungrounded().clone();
        let rt = rt.ungrounded().clone();

        let result = match op {
            // `+` is overloaded: numeric addition OR string concatenation.
            Add => match (&lt, &rt) {
                (Type::Int, Type::Int) => Type::Int,
                (Type::Float, Type::Float)
                | (Type::Int, Type::Float)
                | (Type::Float, Type::Int) => Type::Float,
                (Type::String, Type::String) => Type::String,
                (Type::List(a), Type::List(b)) if a.is_assignable_to(b) => Type::List(b.clone()),
                (Type::List(a), Type::List(b)) if b.is_assignable_to(a) => Type::List(a.clone()),
                (Type::Unknown, _) | (_, Type::Unknown) => Type::Unknown,
                (a, b) => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Int, Float, two Strings, or two compatible Lists".into(),
                            got: format!("{} and {}", a.display_name(), b.display_name()),
                            context: "`+` operator".into(),
                        },
                        l.span().merge(r.span()),
                    ));
                    Type::Unknown
                }
            },
            Sub | Mul | Div | Mod => match (&lt, &rt) {
                (Type::Int, Type::Int) => Type::Int,
                (Type::Float, Type::Float)
                | (Type::Int, Type::Float)
                | (Type::Float, Type::Int) => Type::Float,
                (Type::Unknown, _) | (_, Type::Unknown) => Type::Unknown,
                (a, b) => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Int or Float".into(),
                            got: format!("{} and {}", a.display_name(), b.display_name()),
                            context: "arithmetic operator".into(),
                        },
                        l.span().merge(r.span()),
                    ));
                    Type::Unknown
                }
            },
            Eq | NotEq | Lt | LtEq | Gt | GtEq => {
                if !lt.is_assignable_to(&rt) && !rt.is_assignable_to(&lt) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: lt.display_name(),
                            got: rt.display_name(),
                            context: "comparison".into(),
                        },
                        l.span().merge(r.span()),
                    ));
                }
                Type::Bool
            }
            And | Or => unreachable!("handled by the early return above"),
        };

        // Re-wrap unless the operator itself failed (`Type::Unknown` is
        // the error/recovery sentinel — `Grounded<Unknown>` would add
        // no information and could mask the error downstream).
        if contagious && !matches!(result, Type::Unknown) {
            Type::Grounded(Box::new(result))
        } else {
            result
        }
    }

    pub(super) fn check_unop(&mut self, op: UnaryOp, operand: &Expr) -> Type {
        let t = self.check_expr(operand);

        // D1 part B — the contagion law covers unary operators too:
        // strip `Grounded<>`, run the normal rule, re-wrap if the
        // operand was grounded. Dormant until slice 2b (see
        // `check_binop` for the full rationale).
        let contagious = matches!(t, Type::Grounded(_));
        let t = t.ungrounded().clone();

        let result = match op {
            UnaryOp::Neg | UnaryOp::Pos => match t {
                Type::Int => Type::Int,
                Type::Float => Type::Float,
                Type::Unknown => Type::Unknown,
                other => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Int or Float".into(),
                            got: other.display_name(),
                            context: match op {
                                UnaryOp::Pos => "unary `+`".into(),
                                _ => "unary `-`".into(),
                            },
                        },
                        operand.span(),
                    ));
                    Type::Unknown
                }
            },
            UnaryOp::Not => match t {
                Type::Bool | Type::Unknown => Type::Bool,
                other => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Bool".into(),
                            got: other.display_name(),
                            context: "unary `not`".into(),
                        },
                        operand.span(),
                    ));
                    Type::Bool
                }
            },
        };

        if contagious && !matches!(result, Type::Unknown) {
            Type::Grounded(Box::new(result))
        } else {
            result
        }
    }

    pub(super) fn check_try_propagate(&mut self, inner: &Expr, span: Span) -> Type {
        let inner_ty = self.check_expr(inner);
        match inner_ty {
            Type::Result(ok, err) => {
                self.ensure_try_return_context(
                    &Type::Result(Box::new(Type::Unknown), err.clone()),
                    span,
                );
                (*ok).clone()
            }
            Type::Option(inner) => {
                self.ensure_try_return_context(&Type::Option(Box::new(Type::Unknown)), span);
                (*inner).clone()
            }
            Type::Unknown => Type::Unknown,
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::InvalidTryPropagate {
                        got: other.display_name(),
                    },
                    span,
                ));
                Type::Unknown
            }
        }
    }

    pub(super) fn check_try_retry(&mut self, body: &Expr, has_timeout: bool, span: Span) -> Type {
        let body_ty = self.check_expr(body);
        match body_ty {
            Type::Result(_, _) | Type::Option(_) | Type::Stream(_) | Type::Unknown => body_ty,
            // A `timeout` clause legitimizes ANY body type (slice
            // 50k): any call can hang, and expiry surfaces as a
            // runtime error (retryable when a retry clause is also
            // present). The Result/Option requirement only ever
            // existed for value-level retry semantics.
            other if has_timeout => other,
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::InvalidRetryTarget {
                        got: other.display_name(),
                    },
                    span,
                ));
                Type::Unknown
            }
        }
    }

    fn ensure_try_return_context(&mut self, required: &Type, span: Span) {
        match &self.current_return {
            Some(current) if required.is_assignable_to(current) => {}
            Some(current) => self.errors.push(TypeError::new(
                TypeErrorKind::TryPropagateReturnMismatch {
                    expected: required.display_name(),
                    got: current.display_name(),
                },
                span,
            )),
            None => self.errors.push(TypeError::new(
                TypeErrorKind::TryPropagateReturnMismatch {
                    expected: required.display_name(),
                    got: "no enclosing return type".into(),
                },
                span,
            )),
        }
    }
}

