//! Visiting every expression in an IR file's agent bodies, read-only.
//!
//! The walk calls the visitor on each expression before its
//! sub-expressions, including the values inside a table query's
//! condition. Every match is exhaustive, so a new statement or expression
//! kind fails to compile here until it is walked, and no pass that relies
//! on seeing every expression (such as deriving a table's indexes from
//! its queries) can silently miss one.

use crate::types::{IrBlock, IrExpr, IrExprKind, IrFile, IrMatchArm, IrPathSeg, IrReplayArm, IrStmt};

/// Call `visit` with the agent's name and every expression in that
/// agent's body, for each agent in `file`.
pub fn for_each_expr(file: &IrFile, visit: &mut dyn FnMut(&str, &IrExpr)) {
    for agent in &file.agents {
        walk_block(&agent.body, &mut |expr| visit(&agent.name, expr));
    }
}

fn walk_block(block: &IrBlock, visit: &mut dyn FnMut(&IrExpr)) {
    for stmt in &block.stmts {
        walk_stmt(stmt, visit);
    }
}

fn walk_stmt(stmt: &IrStmt, visit: &mut dyn FnMut(&IrExpr)) {
    match stmt {
        IrStmt::Let { value, .. } | IrStmt::Yield { value, .. } | IrStmt::Destructure { value, .. } => walk_expr(value, visit),
        IrStmt::Return { value, .. } => {
            if let Some(value) = value {
                walk_expr(value, visit);
            }
        }
        IrStmt::If { cond, then_block, else_block, .. } => {
            walk_expr(cond, visit);
            walk_block(then_block, visit);
            if let Some(else_block) = else_block {
                walk_block(else_block, visit);
            }
        }
        IrStmt::For { iter, body, .. } => {
            walk_expr(iter, visit);
            walk_block(body, visit);
        }
        IrStmt::While { cond, body, .. } => {
            walk_expr(cond, visit);
            walk_block(body, visit);
        }
        IrStmt::Parallel { arms, .. } => {
            for arm in arms {
                walk_expr(&arm.call, visit);
            }
        }
        IrStmt::Atomic { body, .. } => walk_block(body, visit),
        IrStmt::Approve { args, .. } => walk_exprs(args, visit),
        IrStmt::Expr { expr, .. } => walk_expr(expr, visit),
        IrStmt::Assign { path, value, .. } => {
            for seg in path {
                match seg {
                    IrPathSeg::Field(_) => {}
                    IrPathSeg::Index(index) => walk_expr(index, visit),
                }
            }
            walk_expr(value, visit);
        }
        IrStmt::Break { .. } | IrStmt::Continue { .. } | IrStmt::Pass { .. } | IrStmt::Dup { .. } | IrStmt::Drop { .. } => {}
    }
}

fn walk_expr(expr: &IrExpr, visit: &mut dyn FnMut(&IrExpr)) {
    visit(expr);
    match &expr.kind {
        IrExprKind::Literal(_) | IrExprKind::Local { .. } | IrExprKind::Decl { .. } | IrExprKind::OptionNone => {}
        IrExprKind::Call { args, .. } => walk_exprs(args, visit),
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            walk_expr(receiver, visit);
            walk_exprs(args, visit);
        }
        IrExprKind::FieldAccess { target, .. } => walk_expr(target, visit),
        IrExprKind::Index { target, index } => {
            walk_expr(target, visit);
            walk_expr(index, visit);
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            walk_expr(left, visit);
            walk_expr(right, visit);
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => walk_expr(operand, visit),
        IrExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, visit);
            for IrMatchArm { guard, body, .. } in arms {
                if let Some(guard) = guard {
                    walk_expr(guard, visit);
                }
                walk_expr(body, visit);
            }
        }
        IrExprKind::StructLiteral { fields, spread, .. } => {
            for (_, value) in fields {
                walk_expr(value, visit);
            }
            if let Some(spread) = spread {
                walk_expr(spread, visit);
            }
        }
        IrExprKind::Lambda { body, .. } => walk_expr(body, visit),
        IrExprKind::MapLiteral { keys, values } => {
            walk_exprs(keys, visit);
            walk_exprs(values, visit);
        }
        IrExprKind::List { items } => walk_exprs(items, visit),
        IrExprKind::Ask { prompt, .. } => walk_expr(prompt, visit),
        IrExprKind::UnwrapGrounded { value: inner }
        | IrExprKind::WeakNew { strong: inner }
        | IrExprKind::WeakUpgrade { weak: inner }
        | IrExprKind::StreamSplitBy { stream: inner, .. }
        | IrExprKind::StreamMerge { groups: inner, .. }
        | IrExprKind::StreamOrderedBy { stream: inner, .. }
        | IrExprKind::StreamResumeToken { stream: inner }
        | IrExprKind::ResumeStream { token: inner, .. }
        | IrExprKind::ResultOk { inner }
        | IrExprKind::ResultErr { inner }
        | IrExprKind::OptionSome { inner }
        | IrExprKind::Choose { options: inner }
        | IrExprKind::TryPropagate { inner }
        | IrExprKind::TryRetry { body: inner, .. }
        | IrExprKind::TrustBoundary { inner } => walk_expr(inner, visit),
        IrExprKind::Replay { trace, arms, else_body } => {
            walk_expr(trace, visit);
            for IrReplayArm { body, .. } in arms {
                walk_expr(body, visit);
            }
            walk_expr(else_body, visit);
        }
        IrExprKind::PageNew { items, next_cursor } => {
            walk_expr(items, visit);
            walk_expr(next_cursor, visit);
        }
        IrExprKind::TableOp { args, .. } => walk_exprs(args, visit),
        IrExprKind::TableQuery { filter, take, .. } => {
            if let Some(filter) = filter {
                for value in filter.values() {
                    walk_expr(value, visit);
                }
            }
            if let Some(take) = take {
                walk_expr(take, visit);
            }
        }
    }
}

fn walk_exprs(exprs: &[IrExpr], visit: &mut dyn FnMut(&IrExpr)) {
    for expr in exprs {
        walk_expr(expr, visit);
    }
}
