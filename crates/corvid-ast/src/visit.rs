//! Walking every statement and expression in a block, in source order.
//!
//! Passes that ask "what does this block contain?" (an `atomic:` block's
//! table operations and the effects it may not hold, for example) read
//! the tree through these two functions, so each one sees the same
//! statements and expressions and none skips a variant. A pass that
//! needs scope or ordering rules of its own still writes its own walk.

use crate::expr::{Expr, Pattern};
use crate::stmt::{Block, Stmt};

/// Call `on_stmt` for every statement in `block`, nested blocks
/// included, and `on_expr` for every expression, nested ones included.
pub fn walk_block<'a>(block: &'a Block, on_stmt: &mut dyn FnMut(&'a Stmt), on_expr: &mut dyn FnMut(&'a Expr)) {
    for stmt in &block.stmts {
        walk_stmt(stmt, on_stmt, on_expr);
    }
}

pub fn walk_stmt<'a>(stmt: &'a Stmt, on_stmt: &mut dyn FnMut(&'a Stmt), on_expr: &mut dyn FnMut(&'a Expr)) {
    on_stmt(stmt);
    match stmt {
        Stmt::Let { value, .. } | Stmt::Yield { value, .. } | Stmt::Destructure { value, .. } => {
            walk_expr(value, on_expr)
        }
        Stmt::Return { value, .. } => {
            if let Some(value) = value {
                walk_expr(value, on_expr);
            }
        }
        Stmt::If { cond, then_block, else_block, .. } => {
            walk_expr(cond, on_expr);
            walk_block(then_block, on_stmt, on_expr);
            if let Some(else_block) = else_block {
                walk_block(else_block, on_stmt, on_expr);
            }
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, on_expr);
            walk_block(body, on_stmt, on_expr);
        }
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, on_expr);
            walk_block(body, on_stmt, on_expr);
        }
        Stmt::Atomic { body, .. } => walk_block(body, on_stmt, on_expr),
        Stmt::Parallel { arms, .. } => {
            for arm in arms {
                walk_expr(&arm.call, on_expr);
            }
        }
        Stmt::Approve { action, .. } => walk_expr(action, on_expr),
        Stmt::Expr { expr, .. } => walk_expr(expr, on_expr),
        Stmt::Assign { target, value, .. } => {
            walk_expr(target, on_expr);
            walk_expr(value, on_expr);
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Pass { .. } => {}
    }
}

/// Call `on_expr` for `expr` and every expression inside it.
pub fn walk_expr<'a>(expr: &'a Expr, on_expr: &mut dyn FnMut(&'a Expr)) {
    on_expr(expr);
    match expr {
        Expr::Literal { .. } | Expr::Ident { .. } => {}
        Expr::Call { callee, args, .. } => {
            walk_expr(callee, on_expr);
            for arg in args {
                walk_expr(arg, on_expr);
            }
        }
        Expr::FieldAccess { target, .. } => walk_expr(target, on_expr),
        Expr::Index { target, index, .. } => {
            walk_expr(target, on_expr);
            walk_expr(index, on_expr);
        }
        Expr::BinOp { left, right, .. } => {
            walk_expr(left, on_expr);
            walk_expr(right, on_expr);
        }
        Expr::UnOp { operand, .. } => walk_expr(operand, on_expr),
        Expr::List { items, .. } => {
            for item in items {
                walk_expr(item, on_expr);
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            walk_expr(scrutinee, on_expr);
            for arm in arms {
                walk_pattern(&arm.pattern);
                if let Some(guard) = &arm.guard {
                    walk_expr(guard, on_expr);
                }
                walk_expr(&arm.body, on_expr);
            }
        }
        Expr::StructLiteral { fields, spread, .. } => {
            for field in fields {
                if let Some(value) = &field.value {
                    walk_expr(value, on_expr);
                }
            }
            if let Some(spread) = spread {
                walk_expr(spread, on_expr);
            }
        }
        Expr::Lambda { body, .. } => walk_expr(body, on_expr),
        Expr::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_expr(key, on_expr);
                walk_expr(value, on_expr);
            }
        }
        Expr::TryPropagate { inner, .. } | Expr::TrustBoundary { inner, .. } => walk_expr(inner, on_expr),
        Expr::TryRetry { body, .. } => walk_expr(body, on_expr),
        Expr::Replay { trace, arms, else_body, .. } => {
            walk_expr(trace, on_expr);
            for arm in arms {
                walk_expr(&arm.body, on_expr);
            }
            walk_expr(else_body, on_expr);
        }
    }
}

/// Patterns hold no expressions a pass needs to see.
fn walk_pattern(_pattern: &Pattern) {}
