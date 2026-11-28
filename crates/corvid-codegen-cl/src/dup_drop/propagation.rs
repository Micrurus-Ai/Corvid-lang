use super::*;
use corvid_ir::{IrMatchArm, IrPattern};
use corvid_types::Type;

/// What the expansion needs to know about the agent it expands.
pub(crate) struct Cx<'a> {
    pub return_ty: &'a Type,
    /// Which calls can raise an outage (recoverable failures TE-3).
    pub outages: &'a crate::outage_plan::OutagePlan,
    /// Where an outage goes from here: out of the agent, or to the
    /// enclosing `try`.
    pub unwind: super::outage::Unwind,
}

/// Expose early returns to ownership analysis. Hoist only across operands
/// that are evaluated on the same path, preserving left-to-right evaluation.
pub(super) fn expand_block(block: &mut IrBlock, cx: &Cx, next: &mut u32) {
    let mut output = Vec::with_capacity(block.stmts.len());
    for mut stmt in std::mem::take(&mut block.stmts) {
        match &mut stmt {
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                expand_expr(cond, cx, next, &mut output);
                expand_block(then_block, cx, next);
                if let Some(block) = else_block {
                    expand_block(block, cx, next);
                }
            }
            IrStmt::For { iter, body, .. } => {
                expand_expr(iter, cx, next, &mut output);
                expand_block(body, cx, next);
            }
            IrStmt::While { cond, body, span } => {
                expand_block(body, cx, next);
                let mut checks = Vec::new();
                expand_expr(cond, cx, next, &mut checks);
                if !checks.is_empty() {
                    // Keep condition effects on every iteration, including after continue.
                    let span = *span;
                    let always = IrExpr {
                        kind: IrExprKind::Literal(corvid_ir::IrLiteral::Bool(true)),
                        ty: Type::Bool,
                        span,
                    };
                    let local_id = LocalId(*next);
                    *next += 1;
                    let name = "__corvid_loop_condition".to_owned();
                    checks.push(IrStmt::Let {
                        local_id, name: name.clone(), ty: Type::Bool, span,
                        value: std::mem::replace(cond, always),
                    });
                    checks.push(IrStmt::If {
                        cond: IrExpr { kind: IrExprKind::Local { local_id, name }, ty: Type::Bool, span },
                        then_block: IrBlock { stmts: Vec::new(), span },
                        else_block: Some(IrBlock { stmts: vec![IrStmt::Break { span }], span }),
                        span,
                    });
                    checks.append(&mut body.stmts);
                    body.stmts = checks;
                }
            }
            IrStmt::Let { value, .. }
            | IrStmt::Return {
                value: Some(value), ..
            }
            | IrStmt::Expr { expr: value, .. } => expand_expr(value, cx, next, &mut output),
            // Each is evaluated before the statement acts: an `approve`'s
            // arguments before the approval is asked, a destructured value
            // before its parts are bound.
            IrStmt::Approve { args, .. } => expand_operands(args.iter_mut(), cx, next, &mut output),
            IrStmt::Destructure { value, .. } => expand_expr(value, cx, next, &mut output),
            IrStmt::Assign { value, path, .. } => {
                let indices = path.iter_mut().filter_map(|part| match part {
                    corvid_ir::IrPathSeg::Index(index) => Some(index),
                    _ => None,
                });
                expand_operands(indices.chain(std::iter::once(value)), cx, next, &mut output)
            }
            _ => {}
        }
        output.push(stmt);
    }
    block.stmts = output;
}

pub(super) fn expand_expr(expr: &mut IrExpr, cx: &Cx, next: &mut u32, output: &mut Vec<IrStmt>) {
    use IrExprKind::*;
    match &mut expr.kind {
        TryPropagate { inner } => {
            expand_expr(inner, cx, next, output);
            expand_binding(expr, cx.return_ty, next, output);
        }
        Match { scrutinee, arms } => {
            expand_expr(scrutinee, cx, next, output);
            if super::outage::check_failing_arms(arms, &expr.ty, cx) {
                super::outage::hoist_checked_match(expr, cx, next, output);
            }
        }
        OptionSome { inner }
        | ResultOk { inner }
        | ResultErr { inner }
        | UnOp { operand: inner, .. }
        | WrappingUnOp { operand: inner, .. }
        | FieldAccess { target: inner, .. }
        | TrustBoundary { inner } => expand_expr(inner, cx, next, output),
        BinOp {
            op: op @ (corvid_ast::BinaryOp::And | corvid_ast::BinaryOp::Or),
            left,
            right,
        } => {
            expand_expr(left, cx, next, output);
            let mut selected = Vec::new();
            expand_expr(right, cx, next, &mut selected);
            if selected.is_empty() {
                return;
            }
            let span = expr.span;
            let local_id = LocalId(*next);
            *next += 1;
            let name = "__corvid_condition".to_owned();
            let local = IrExpr {
                kind: Local {
                    local_id,
                    name: name.clone(),
                },
                ty: Type::Bool,
                span,
            };
            let bind = |value| IrStmt::Let {
                local_id,
                name: name.clone(),
                value,
                ty: Type::Bool,
                span,
            };
            output.push(bind(std::mem::replace(left.as_mut(), local.clone())));
            selected.push(bind(std::mem::replace(right.as_mut(), local.clone())));
            let block = IrBlock {
                stmts: selected,
                span,
            };
            let (then_block, else_block) = if *op == corvid_ast::BinaryOp::And {
                (block, None)
            } else {
                (
                    IrBlock {
                        stmts: Vec::new(),
                        span,
                    },
                    Some(block),
                )
            };
            output.push(IrStmt::If {
                cond: local.clone(),
                then_block,
                else_block,
                span,
            });
            *expr = local;
        }
        BinOp { left, right, .. }
        | WrappingBinOp { left, right, .. }
        | Index {
            target: left,
            index: right,
        } => expand_operands(
            [left.as_mut(), right.as_mut()].into_iter(),
            cx,
            next,
            output,
        ),
        Call { kind, args, .. } => {
            let can_fail = cx.outages.call_can_fail(kind);
            expand_operands(args.iter_mut(), cx, next, output);
            if can_fail {
                super::outage::hoist_checked_call(expr, cx, next, output);
            }
        }
        List { items: args } => expand_operands(args.iter_mut(), cx, next, output),
        MapLiteral { keys, values } => expand_operands(
            keys.iter_mut().zip(values).flat_map(|(key, value)| [key, value]),
            cx, next, output,
        ),
        BuiltinMethod { receiver, args, .. } => expand_operands(
            std::iter::once(receiver.as_mut()).chain(args),
            cx,
            next,
            output,
        ),
        TryRetry { .. } => super::outage::expand_try(expr, cx, next, output),
        _ => {}
    }
}

// When a later operand introduces a branch, save each earlier operand
// before that branch. Otherwise its calls or reads could execute too late.
fn expand_operands<'a>(
    operands: impl DoubleEndedIterator<Item = &'a mut IrExpr>,
    cx: &Cx,
    next: &mut u32,
    output: &mut Vec<IrStmt>,
) {
    let mut groups = Vec::new();
    for operand in operands.rev() {
        let mut before = Vec::new();
        expand_expr(operand, cx, next, &mut before);
        if !groups.is_empty() {
            let id = LocalId(*next);
            *next += 1;
            let name = "__corvid_operand".to_owned();
            let local = IrExpr {
                kind: IrExprKind::Local {
                    local_id: id,
                    name: name.clone(),
                },
                ty: operand.ty.clone(),
                span: operand.span,
            };
            before.push(IrStmt::Let {
                local_id: id,
                name,
                ty: operand.ty.clone(),
                span: operand.span,
                value: std::mem::replace(operand, local),
            });
        }
        if !before.is_empty() {
            groups.push(before);
        }
    }
    for group in groups.into_iter().rev() {
        output.extend(group);
    }
}

fn expand_binding(value: &mut IrExpr, return_ty: &Type, next: &mut u32, output: &mut Vec<IrStmt>) {
    let IrExprKind::TryPropagate { inner } = &value.kind else {
        return;
    };
    let span = value.span;
    let wrapper_ty = inner.ty.clone();
    let local_id = LocalId(*next);
    *next += 1;
    let name = "__corvid_propagated".to_owned();
    let wrapper = IrExpr {
        kind: IrExprKind::Local {
            local_id,
            name: name.clone(),
        },
        ty: wrapper_ty.clone(),
        span,
    };
    let (failed, failure, success_pattern) = match (&wrapper_ty, return_ty) {
        (Type::Option(_), Type::Option(_)) => {
            let none = |ty| IrExpr {
                kind: IrExprKind::OptionNone,
                ty,
                span,
            };
            let cond = IrExprKind::BinOp {
                op: corvid_ast::BinaryOp::Eq,
                left: Box::new(wrapper.clone()),
                right: Box::new(none(wrapper_ty.clone())),
            };
            (
                cond,
                none(return_ty.clone()),
                IrPattern::Some_ as fn(_) -> _,
            )
        }
        (Type::Result(_, error), Type::Result(_, outer_error)) if error == outer_error => {
            let cond = IrExprKind::BuiltinMethod {
                kind: corvid_types::BuiltinMethodKind::ResultIsErr,
                receiver: Box::new(wrapper.clone()),
                args: vec![],
            };
            let failure = if wrapper_ty == *return_ty {
                wrapper.clone()
            } else {
                let error = project(&wrapper, *error.clone(), IrPattern::Err_, next);
                IrExpr {
                    kind: IrExprKind::ResultErr {
                        inner: Box::new(error),
                    },
                    ty: return_ty.clone(),
                    span,
                }
            };
            (cond, failure, IrPattern::Ok_ as fn(_) -> _)
        }
        _ => return,
    };
    output.push(IrStmt::Let {
        local_id,
        name,
        ty: wrapper_ty,
        value: *inner.clone(),
        span,
    });
    output.push(IrStmt::If {
        cond: IrExpr {
            kind: failed,
            ty: Type::Bool,
            span,
        },
        then_block: IrBlock {
            stmts: vec![IrStmt::Return {
                value: Some(failure),
                span,
            }],
            span,
        },
        else_block: None,
        span,
    });
    *value = project(&wrapper, value.ty.clone(), success_pattern, next);
}

// The branch above has established which constructor is present.
fn project(
    wrapper: &IrExpr,
    ty: Type,
    pattern: fn(Box<IrPattern>) -> IrPattern,
    next: &mut u32,
) -> IrExpr {
    let span = wrapper.span;
    let local_id = LocalId(*next);
    *next += 1;
    let name = "__corvid_payload".to_owned();
    let arm = IrMatchArm {
        pattern: pattern(Box::new(IrPattern::Bind {
            local_id,
            name: name.clone(),
        })),
        guard: None,
        body: IrExpr {
            kind: IrExprKind::Local { local_id, name },
            ty: ty.clone(),
            span,
        },
        span,
    };
    IrExpr {
        kind: IrExprKind::Match {
            scrutinee: Box::new(wrapper.clone()),
            arms: vec![arm],
        },
        ty,
        span,
    }
}
