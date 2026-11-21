use super::*;
use corvid_ir::{IrMatchArm, IrPattern};
use corvid_types::Type;

/// Make the failure edge of a binding like `n = parse(text)?` visible to
/// ordinary branch liveness. Evaluate the wrapped expression exactly once.
pub(super) fn expand_block(block: &mut IrBlock, return_ty: &Type, next: &mut u32) {
    let mut output = Vec::with_capacity(block.stmts.len());
    for mut stmt in std::mem::take(&mut block.stmts) {
        match &mut stmt {
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                expand_expr(cond, return_ty, next, &mut output);
                expand_block(then_block, return_ty, next);
                if let Some(block) = else_block {
                    expand_block(block, return_ty, next);
                }
            }
            IrStmt::For { iter, body, .. } => {
                expand_expr(iter, return_ty, next, &mut output);
                expand_block(body, return_ty, next);
            }
            IrStmt::While { body, .. } => expand_block(body, return_ty, next),
            IrStmt::Let { value, .. }
            | IrStmt::Return {
                value: Some(value), ..
            }
            | IrStmt::Expr { expr: value, .. } => expand_expr(value, return_ty, next, &mut output),
            IrStmt::Assign { value, path, .. } if path.is_empty() => {
                expand_expr(value, return_ty, next, &mut output)
            }
            _ => {}
        }
        output.push(stmt);
    }
    block.stmts = output;
}

fn expand_expr(expr: &mut IrExpr, ret: &Type, next: &mut u32, output: &mut Vec<IrStmt>) {
    use IrExprKind::*;
    match &mut expr.kind {
        TryPropagate { inner } => {
            expand_expr(inner, ret, next, output);
            expand_binding(expr, ret, next, output);
        }
        OptionSome { inner }
        | ResultOk { inner }
        | ResultErr { inner }
        | UnOp { operand: inner, .. }
        | WrappingUnOp { operand: inner, .. }
        | FieldAccess { target: inner, .. }
        | Match {
            scrutinee: inner, ..
        } => expand_expr(inner, ret, next, output),
        BinOp {
            op: op @ (corvid_ast::BinaryOp::And | corvid_ast::BinaryOp::Or),
            left,
            right,
        } => {
            expand_expr(left, ret, next, output);
            let mut selected = Vec::new();
            expand_expr(right, ret, next, &mut selected);
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
            ret,
            next,
            output,
        ),
        Call { args, .. } | List { items: args } => {
            expand_operands(args.iter_mut(), ret, next, output)
        }
        BuiltinMethod { receiver, args, .. } => expand_operands(
            std::iter::once(receiver.as_mut()).chain(args),
            ret,
            next,
            output,
        ),
        _ => {}
    }
}

// When a later operand introduces a branch, save each earlier operand
// before that branch. Otherwise its calls or reads could execute too late.
fn expand_operands<'a>(
    operands: impl DoubleEndedIterator<Item = &'a mut IrExpr>,
    ret: &Type,
    next: &mut u32,
    output: &mut Vec<IrStmt>,
) {
    let mut groups = Vec::new();
    for operand in operands.rev() {
        let mut before = Vec::new();
        expand_expr(operand, ret, next, &mut before);
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
