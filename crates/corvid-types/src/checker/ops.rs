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
    pub(super) fn check_binop(
        &mut self,
        op: BinaryOp,
        l: &Expr,
        r: &Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        // B06 — literal operands read as `Decimal` when one is expected.
        let lt = self.check_operand(l, expected);
        let rt = self.check_operand(r, expected);
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
        // Slice 50i — taint contagion: an operator with a tainted
        // operand yields a tainted result. `"prefix " + untrusted`
        // does not launder the untrusted content.
        let tainted = matches!(lt, Type::Tainted(_)) || matches!(rt, Type::Tainted(_));
        let lt = lt.ungrounded().untainted().clone();
        let rt = rt.ungrounded().untainted().clone();

        // A side of a comparison written only as literals takes the other
        // side's type, as an annotated binding gives one its type:
        // `price == Some(10)` against an `Option<Decimal>`,
        // `zone == "Europe/Berlin"` against a `TimeZone`. It runs before
        // the type-specific rules below, so it applies to every type. Only
        // a comparison that would fail is re-read, so every accepted
        // comparison keeps its typing, and the re-read checks the literal
        // (an unknown zone is still an error).
        let (lt, rt) = if matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq)
            && !lt.is_assignable_to(&rt)
            && !rt.is_assignable_to(&lt)
        {
            self.read_literal_side_by_the_other(l, r, lt, rt)
        } else {
            (lt, rt)
        };

        // A `Secret` operand is checked first, so its message wins over
        // any other type's.
        let b06 = match self.check_secret_binop(op, l, r, &lt, &rt) {
            Some(secret) => Some(secret),
            None => match self.check_decimal_binop(op, l, r, &lt, &rt, span) {
                Some(decimal) => Some(decimal),
                None => match self.check_bytes_binop(op, l, r, &lt, &rt) {
                    Some(bytes) => Some(bytes),
                    None => self.check_time_binop(op, l, r, &lt, &rt),
                },
            },
        };
        let result = if let Some(b06) = b06 {
            b06
        } else {
            match op {
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
            }
        };

        // Re-wrap unless the operator itself failed (`Type::Unknown` is
        // the error/recovery sentinel — `Grounded<Unknown>` would add
        // no information and could mask the error downstream).
        let result = if contagious && !matches!(result, Type::Unknown) {
            Type::Grounded(Box::new(result))
        } else {
            result
        };
        if tainted && !matches!(result, Type::Unknown) {
            Type::Tainted(Box::new(result))
        } else {
            result
        }
    }

    pub(super) fn check_unop(&mut self, op: UnaryOp, operand: &Expr, expected: Option<&Type>) -> Type {
        let t = match op {
            UnaryOp::Neg | UnaryOp::Pos => self.check_operand(operand, expected),
            UnaryOp::Not => self.check_expr(operand),
        };

        // D1 part B — the contagion law covers unary operators too:
        // strip `Grounded<>`, run the normal rule, re-wrap if the
        // operand was grounded. Dormant until slice 2b (see
        // `check_binop` for the full rationale).
        let contagious = matches!(t, Type::Grounded(_));
        let tainted = matches!(t, Type::Tainted(_));
        let t = t.ungrounded().untainted().clone();

        let result = match op {
            UnaryOp::Neg | UnaryOp::Pos => match t {
                Type::Int => Type::Int,
                Type::Float => Type::Float,
                Type::Decimal => Type::Decimal,
                Type::Duration if op == UnaryOp::Neg => Type::Duration,
                Type::Unknown => Type::Unknown,
                other => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Int, Float or Decimal".into(),
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

        let result = if contagious && !matches!(result, Type::Unknown) {
            Type::Grounded(Box::new(result))
        } else {
            result
        };
        if tainted && !matches!(result, Type::Unknown) {
            Type::Tainted(Box::new(result))
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

    pub(super) fn check_try_retry(&mut self, body: &Expr, has_timeout: bool, fallback: Option<&Expr>, span: Span) -> Type {
        let body_ty = self.check_expr(body);
        if let Some(fallback) = fallback {
            return self.check_try_else(body, body_ty, fallback, span);
        }
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

    /// `try <body> ... else <fallback>`: the form has the body's type, of
    /// any kind, since the fallback answers an outage, not a value; the
    /// fallback must be assignable to it. A connector operation that
    /// declares `on failure` already returns its outages as a value, so
    /// wrapping its call in `else` is refused with what to match instead.
    fn check_try_else(&mut self, body: &Expr, body_ty: Type, fallback: &Expr, span: Span) -> Type {
        let fallback_ty = self.check_expr(fallback);
        if let Some((operation, variant)) = self.operation_with_on_failure(body) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TryElseAroundOnFailure { operation, variant },
                span,
            ));
            return body_ty;
        }
        if body_ty != Type::Unknown && fallback_ty != Type::Unknown && !fallback_ty.is_assignable_to(&body_ty) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TryElseFallbackType {
                    expected: body_ty.display_name(),
                    got: fallback_ty.display_name(),
                },
                fallback.span(),
            ));
        }
        body_ty
    }

    /// The connector operation `body` calls directly, with the variant
    /// its `on failure` names, when it declares one.
    fn operation_with_on_failure(&self, body: &Expr) -> Option<(String, String)> {
        let Expr::Call { callee, .. } = body else { return None };
        let Expr::Ident { name, .. } = callee.as_ref() else { return None };
        self.file.decls.iter().find_map(|decl| match decl {
            corvid_ast::Decl::Connector(connector) => connector.operations.iter().find_map(|op| {
                let variant = op.on_failure.as_ref()?;
                (op.name.name == name.name).then(|| (op.name.name.clone(), variant.name.clone()))
            }),
            _ => None,
        })
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

    /// In a comparison whose sides do not match, re-check the side that
    /// is written only as literals against the other side's type, and
    /// return both types. When neither or both sides are literals, the
    /// types are returned unchanged.
    fn read_literal_side_by_the_other(&mut self, l: &Expr, r: &Expr, lt: Type, rt: Type) -> (Type, Type) {
        let (left_literal, right_literal) = (self.is_literal_expr(l), self.is_literal_expr(r));
        if right_literal && !left_literal && !matches!(lt, Type::Unknown) {
            let rt = self.check_expr_as(r, Some(&lt)).ungrounded().untainted().clone();
            (lt, rt)
        } else if left_literal && !right_literal && !matches!(rt, Type::Unknown) {
            let lt = self.check_expr_as(l, Some(&rt)).ungrounded().untainted().clone();
            (lt, rt)
        } else {
            (lt, rt)
        }
    }

    /// Whether `e` is written only as literals: a text, number or bool
    /// literal, `None`, `Some`, `Ok` or `Err` of one, or a list of them.
    /// The constructors are recognised by what they resolve to, so a
    /// declared variant that happens to be called `Some` is not one.
    pub(super) fn is_literal_expr(&self, e: &Expr) -> bool {
        use corvid_resolve::{Binding, BuiltIn};
        match e {
            Expr::Literal { value, .. } => !matches!(value, corvid_ast::Literal::Nothing),
            _ if super::decimal::is_number_literal_expr(e) => true,
            Expr::Ident { name, .. } => {
                matches!(self.bindings.get(&name.span), Some(Binding::BuiltIn(BuiltIn::None)))
            }
            Expr::Call { callee, args, .. } => match callee.as_ref() {
                Expr::Ident { name, .. } => {
                    matches!(
                        self.bindings.get(&name.span),
                        Some(Binding::BuiltIn(BuiltIn::Some | BuiltIn::Ok | BuiltIn::Err))
                    ) && args.len() == 1
                        && self.is_literal_expr(&args[0])
                }
                _ => false,
            },
            Expr::List { items, .. } => !items.is_empty() && items.iter().all(|item| self.is_literal_expr(item)),
            _ => false,
        }
    }
}
