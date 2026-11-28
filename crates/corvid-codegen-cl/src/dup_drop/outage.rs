//! Outage unwinding as ordinary control flow (recoverable failures TE-3).
//!
//! A call that can raise an outage is hoisted into its own statement and
//! followed by a test of the pending slot. Out of a `try`, the test
//! returns from the agent; inside one, it retries the body or leaves the
//! `try` for its fallback. Both are a plain `return`, `continue` or
//! `break`, so the ownership pass that runs next releases whatever is
//! live on that path, exactly as it does for `?`.
//!
//! `try E retry N else F` becomes, with `T` the type of `E`:
//!
//! ```text
//! value: T = <no value>        # zero bits, never read
//! left: Int = N
//! failed = false
//! value_failed = false         # an attempt gave Err or None
//! while true:
//!     left = left - 1
//!     <E's statements; after each failing call:
//!         if outage: (if left > 0: clear; continue) failed = true; break>
//!     value = E
//!     <Result or Option body with retries:
//!         if left > 0 and value is Err or None: value_failed = true; continue>
//!     break
//! if failed:
//!     if value_failed: clear          # the last Err or None stands
//!     else: clear; value = F           # or, with no `else`, unwind on
//! ```
//!
//! That is the interpreter's rule: an outage, an `Err` and a `None` each
//! earn another attempt; anything else stops the program at once.

use super::propagation::{expand_expr, Cx};
use corvid_ast::{BinaryOp, Span};
use corvid_ir::{IrBlock, IrExpr, IrExprKind, IrLiteral, IrOutage, IrStmt};
use corvid_resolve::LocalId;
use corvid_types::Type;

/// Where an outage found by a test goes.
#[derive(Clone, Copy)]
pub(crate) enum Unwind {
    /// Out of the agent, by `return`.
    Return,
    /// To the enclosing `try`'s loop.
    Try { left: LocalId, failed: LocalId },
}

fn local(id: LocalId, name: &str, ty: Type, span: Span) -> IrExpr {
    IrExpr { kind: IrExprKind::Local { local_id: id, name: name.to_owned() }, ty, span }
}

fn literal(value: IrLiteral, ty: Type, span: Span) -> IrExpr {
    IrExpr { kind: IrExprKind::Literal(value), ty, span }
}

fn outage(part: IrOutage, ty: Type, span: Span) -> IrExpr {
    IrExpr { kind: IrExprKind::Outage(part), ty, span }
}

fn binop(op: BinaryOp, left: IrExpr, right: IrExpr, ty: Type, span: Span) -> IrExpr {
    IrExpr { kind: IrExprKind::BinOp { op, left: Box::new(left), right: Box::new(right) }, ty, span }
}

fn fresh(next: &mut u32) -> LocalId {
    let id = LocalId(*next);
    *next += 1;
    id
}

fn let_(id: LocalId, name: &str, ty: Type, value: IrExpr, span: Span) -> IrStmt {
    IrStmt::Let { local_id: id, name: name.to_owned(), ty, value, span }
}

/// Rebinding a local is a `let` of the same local, as the front end
/// lowers `x = v`.
fn assign(id: LocalId, name: &str, ty: Type, value: IrExpr, span: Span) -> IrStmt {
    let_(id, name, ty, value, span)
}

fn if_(cond: IrExpr, then: Vec<IrStmt>, otherwise: Option<Vec<IrStmt>>, span: Span) -> IrStmt {
    IrStmt::If {
        cond,
        then_block: IrBlock { stmts: then, span },
        else_block: otherwise.map(|stmts| IrBlock { stmts, span }),
        span,
    }
}

fn clear(span: Span) -> IrStmt {
    IrStmt::Expr { expr: outage(IrOutage::Clear, Type::Bool, span), span }
}

/// The test after a call that can fail, sending an outage to `cx.unwind`.
fn test(cx: &Cx, span: Span) -> IrStmt {
    let pending = outage(IrOutage::Pending, Type::Bool, span);
    let leave = match cx.unwind {
        Unwind::Return => {
            let value = match cx.return_ty {
                Type::Nothing => None,
                ty => Some(outage(IrOutage::Unwind, ty.clone(), span)),
            };
            vec![IrStmt::Return { value, span }]
        }
        Unwind::Try { left, failed } => {
            let more = binop(
                BinaryOp::Gt,
                local(left, "__corvid_try_left", Type::Int, span),
                literal(IrLiteral::Int(0), Type::Int, span),
                Type::Bool,
                span,
            );
            vec![
                if_(more, vec![clear(span), IrStmt::Continue { span }], None, span),
                assign(failed, "__corvid_try_failed", Type::Bool, literal(IrLiteral::Bool(true), Type::Bool, span), span),
                IrStmt::Break { span },
            ]
        }
    };
    if_(pending, leave, None, span)
}

/// Hoist the call `expr`, whose arguments are already expanded, into its
/// own statement followed by the test, and leave its result in `expr`.
pub(super) fn hoist_checked_call(expr: &mut IrExpr, cx: &Cx, next: &mut u32, output: &mut Vec<IrStmt>) {
    let span = expr.span;
    let ty = expr.ty.clone();
    let call = IrExpr { kind: IrExprKind::Outage(IrOutage::Checked(Box::new(expr.clone()))), ty: ty.clone(), span };
    if ty == Type::Nothing {
        output.push(IrStmt::Expr { expr: call, span });
        output.push(test(cx, span));
        *expr = literal(IrLiteral::Nothing, Type::Nothing, span);
        return;
    }
    let id = fresh(next);
    output.push(let_(id, "__corvid_outcome", ty.clone(), call, span));
    output.push(test(cx, span));
    *expr = local(id, "__corvid_outcome", ty, span);
}

/// Mark each `match` arm body or guard that is a call that can fail as
/// checked (failing ones were lifted into such calls by `outage_lift`),
/// returning whether any was. The `match` is then tested after it.
///
/// An outage in a guard leaves it `false`, and the `match` would go on to
/// later arms. So every arm after the first failing guard is guarded by
/// "no outage is pending" as well, and a last arm yields the unwind
/// value: once a guard fails, no later guard or body runs.
pub(super) fn check_failing_arms(arms: &mut Vec<corvid_ir::IrMatchArm>, ty: &Type, cx: &Cx) -> bool {
    let fails = |expr: &IrExpr| matches!(&expr.kind, IrExprKind::Call { kind, .. } if cx.outages.call_can_fail(kind));
    let checked = |expr: &mut IrExpr| {
        let span = expr.span;
        let ty = expr.ty.clone();
        let call = std::mem::replace(expr, literal(IrLiteral::Nothing, Type::Nothing, span));
        *expr = IrExpr { kind: IrExprKind::Outage(IrOutage::Checked(Box::new(call))), ty, span };
    };
    let mut any = false;
    let mut first_failing_guard = None;
    for (index, arm) in arms.iter_mut().enumerate() {
        if let Some(guard) = &mut arm.guard {
            if fails(guard) {
                checked(guard);
                any = true;
                first_failing_guard.get_or_insert(index);
            }
        }
        if fails(&arm.body) {
            checked(&mut arm.body);
            any = true;
        }
    }
    if let Some(first) = first_failing_guard {
        let span = arms[first].span;
        let no_outage = || IrExpr {
            kind: IrExprKind::UnOp {
                op: corvid_ast::UnaryOp::Not,
                operand: Box::new(outage(IrOutage::Pending, Type::Bool, span)),
            },
            ty: Type::Bool,
            span,
        };
        for arm in arms.iter_mut().skip(first + 1) {
            arm.guard = Some(match arm.guard.take() {
                Some(guard) => binop(BinaryOp::And, no_outage(), guard, Type::Bool, span),
                None => no_outage(),
            });
        }
        arms.push(corvid_ir::IrMatchArm {
            pattern: corvid_ir::IrPattern::Wildcard,
            guard: None,
            body: outage(IrOutage::Unwind, ty.clone(), span),
            span,
        });
    }
    any
}

/// Hoist a `match` with checked arms into its own statement followed by
/// the test, leaving its value in `expr`.
pub(super) fn hoist_checked_match(expr: &mut IrExpr, cx: &Cx, next: &mut u32, output: &mut Vec<IrStmt>) {
    let span = expr.span;
    let ty = expr.ty.clone();
    let matched = std::mem::replace(expr, literal(IrLiteral::Nothing, Type::Nothing, span));
    if ty == Type::Nothing {
        output.push(IrStmt::Expr { expr: matched, span });
        output.push(test(cx, span));
        return;
    }
    let id = fresh(next);
    output.push(let_(id, "__corvid_matched", ty.clone(), matched, span));
    output.push(test(cx, span));
    *expr = local(id, "__corvid_matched", ty, span);
}

/// Expand `try ... [retry N] [else F]` into the loop above when native
/// code catches its outages exactly. A `try` it cannot catch exactly, or
/// one with a `timeout`, is left for native lowering to refuse by name.
pub(super) fn expand_try(expr: &mut IrExpr, cx: &Cx, next: &mut u32, output: &mut Vec<IrStmt>) {
    let IrExprKind::TryRetry { body, attempts, timeout_ms, fallback, .. } = &expr.kind else {
        return;
    };
    let catches = fallback.is_some() || cx.outages.can_fail(body);
    if !catches || timeout_ms.is_some() || cx.outages.try_refusal(body).is_some() {
        // Retry without outages keeps its own lowering; the rest is refused.
        return;
    }
    let span = expr.span;
    let ty = expr.ty.clone();
    let attempts = (*attempts).max(1);
    let retries_values = attempts > 1 && matches!(body.ty, Type::Result(_, _) | Type::Option(_));
    let mut body = (**body).clone();
    let fallback = fallback.as_deref().cloned();

    let value = fresh(next);
    let left = fresh(next);
    let failed = fresh(next);
    let value_failed = fresh(next);
    let bool_lit = |b| literal(IrLiteral::Bool(b), Type::Bool, span);
    output.push(let_(value, "__corvid_try_value", ty.clone(), outage(IrOutage::Unwind, ty.clone(), span), span));
    output.push(let_(left, "__corvid_try_left", Type::Int, literal(IrLiteral::Int(attempts as i64), Type::Int, span), span));
    output.push(let_(failed, "__corvid_try_failed", Type::Bool, bool_lit(false), span));
    output.push(let_(value_failed, "__corvid_try_value_failed", Type::Bool, bool_lit(false), span));

    let inner = Cx { return_ty: cx.return_ty, outages: cx.outages, unwind: Unwind::Try { left, failed } };
    let left_local = || local(left, "__corvid_try_left", Type::Int, span);
    let mut attempt = vec![assign(
        left,
        "__corvid_try_left",
        Type::Int,
        binop(BinaryOp::Sub, left_local(), literal(IrLiteral::Int(1), Type::Int, span), Type::Int, span),
        span,
    )];
    expand_expr(&mut body, &inner, next, &mut attempt);
    attempt.push(assign(value, "__corvid_try_value", ty.clone(), body, span));
    if retries_values {
        let value_local = local(value, "__corvid_try_value", ty.clone(), span);
        let unsuccessful = match &ty {
            Type::Result(_, _) => IrExpr {
                kind: IrExprKind::BuiltinMethod {
                    kind: corvid_types::BuiltinMethodKind::ResultIsErr,
                    receiver: Box::new(value_local),
                    args: Vec::new(),
                },
                ty: Type::Bool,
                span,
            },
            _ => binop(
                BinaryOp::Eq,
                value_local,
                IrExpr { kind: IrExprKind::OptionNone, ty: ty.clone(), span },
                Type::Bool,
                span,
            ),
        };
        let more = binop(BinaryOp::Gt, left_local(), literal(IrLiteral::Int(0), Type::Int, span), Type::Bool, span);
        attempt.push(if_(
            binop(BinaryOp::And, more, unsuccessful, Type::Bool, span),
            vec![
                assign(value_failed, "__corvid_try_value_failed", Type::Bool, bool_lit(true), span),
                IrStmt::Continue { span },
            ],
            None,
            span,
        ));
    }
    attempt.push(IrStmt::Break { span });
    output.push(IrStmt::While { cond: bool_lit(true), body: IrBlock { stmts: attempt, span }, span });

    // After the loop: an outage left `failed` set and the slot held.
    let failed_local = local(failed, "__corvid_try_failed", Type::Bool, span);
    let value_failed_local = local(value_failed, "__corvid_try_value_failed", Type::Bool, span);
    let on_outage = match fallback {
        Some(mut fallback) => {
            let mut stmts = vec![clear(span)];
            expand_expr(&mut fallback, cx, next, &mut stmts);
            stmts.push(assign(value, "__corvid_try_value", ty.clone(), fallback, span));
            stmts
        }
        // No `else`: the outage goes on to whatever encloses this `try`.
        None => vec![test(cx, span)],
    };
    output.push(if_(
        failed_local,
        vec![if_(value_failed_local, vec![clear(span)], Some(on_outage), span)],
        None,
        span,
    ));
    *expr = local(value, "__corvid_try_value", ty, span);
}
