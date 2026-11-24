//! Visiting every type slot in the parts of an IR file that code
//! generators compile: declared types, agent, tool and prompt
//! signatures, and agent bodies.
//!
//! The walk calls the visitor once per slot (a `Type` field in the IR)
//! and does not descend into the type itself, so a visitor that
//! rewrites `List<JsonError>` recurses through the type on its own.
//! Every match is exhaustive, so a new statement or expression kind
//! fails to compile here until its types are visited.

use crate::types::{
    IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrMatchArm, IrPathSeg, IrReplayArm, IrStmt,
};
use corvid_types::Type;

/// Call `visit` on every type slot in `file`'s declared types,
/// agent, tool and prompt signatures, and agent bodies.
pub fn for_each_compiled_type_mut(file: &mut IrFile, visit: &mut dyn FnMut(&mut Type)) {
    for ty in &mut file.types {
        for field in &mut ty.fields {
            visit(&mut field.ty);
        }
        for variant in &mut ty.variants {
            for field in &mut variant.fields {
                visit(&mut field.ty);
            }
        }
    }
    for tool in &mut file.tools {
        for param in &mut tool.params {
            visit(&mut param.ty);
        }
        visit(&mut tool.return_ty);
    }
    for prompt in &mut file.prompts {
        for param in &mut prompt.params {
            visit(&mut param.ty);
        }
        visit(&mut prompt.return_ty);
    }
    for agent in &mut file.agents {
        for param in &mut agent.params {
            visit(&mut param.ty);
        }
        visit(&mut agent.return_ty);
        visit_block(&mut agent.body, visit);
    }
}

fn visit_block(block: &mut IrBlock, visit: &mut dyn FnMut(&mut Type)) {
    for stmt in &mut block.stmts {
        visit_stmt(stmt, visit);
    }
}

fn visit_stmt(stmt: &mut IrStmt, visit: &mut dyn FnMut(&mut Type)) {
    match stmt {
        IrStmt::Let { ty, value, .. } => {
            visit(ty);
            visit_expr(value, visit);
        }
        IrStmt::Return { value, .. } => {
            if let Some(value) = value {
                visit_expr(value, visit);
            }
        }
        IrStmt::Yield { value, .. } => visit_expr(value, visit),
        IrStmt::If { cond, then_block, else_block, .. } => {
            visit_expr(cond, visit);
            visit_block(then_block, visit);
            if let Some(else_block) = else_block {
                visit_block(else_block, visit);
            }
        }
        IrStmt::For { iter, body, .. } => {
            visit_expr(iter, visit);
            visit_block(body, visit);
        }
        IrStmt::Parallel { arms, .. } => {
            for arm in arms {
                visit_expr(&mut arm.call, visit);
            }
        }
        IrStmt::Destructure { value, .. } => visit_expr(value, visit),
        IrStmt::Atomic { body, .. } => {
            visit_block(body, visit);
        }
        IrStmt::While { cond, body, .. } => {
            visit_expr(cond, visit);
            visit_block(body, visit);
        }
        IrStmt::Approve { args, .. } => {
            for arg in args {
                visit_expr(arg, visit);
            }
        }
        IrStmt::Expr { expr, .. } => visit_expr(expr, visit),
        IrStmt::Assign { root_ty, path, value, .. } => {
            visit(root_ty);
            for seg in path {
                match seg {
                    IrPathSeg::Field(_) => {}
                    IrPathSeg::Index(index) => visit_expr(index, visit),
                }
            }
            visit_expr(value, visit);
        }
        IrStmt::Break { .. }
        | IrStmt::Continue { .. }
        | IrStmt::Pass { .. }
        | IrStmt::Dup { .. }
        | IrStmt::Drop { .. } => {}
    }
}

fn visit_expr(expr: &mut IrExpr, visit: &mut dyn FnMut(&mut Type)) {
    visit(&mut expr.ty);
    match &mut expr.kind {
        IrExprKind::Literal(_)
        | IrExprKind::Local { .. }
        | IrExprKind::Decl { .. }
        | IrExprKind::OptionNone => {}
        IrExprKind::Call { kind, args, .. } => {
            match kind {
                IrCallKind::ClosureLocal { signature, .. } => visit(signature),
                IrCallKind::Tool { .. }
                | IrCallKind::Prompt { .. }
                | IrCallKind::Agent { .. }
                | IrCallKind::Fixture { .. }
                | IrCallKind::EnumConstructor { .. }
                | IrCallKind::StructConstructor { .. }
                | IrCallKind::Unknown => {}
            }
            visit_exprs(args, visit);
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            visit_expr(receiver, visit);
            visit_exprs(args, visit);
        }
        IrExprKind::FieldAccess { target, .. } => visit_expr(target, visit),
        IrExprKind::Index { target, index } => {
            visit_expr(target, visit);
            visit_expr(index, visit);
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            visit_expr(left, visit);
            visit_expr(right, visit);
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            visit_expr(operand, visit)
        }
        IrExprKind::Match { scrutinee, arms } => {
            visit_expr(scrutinee, visit);
            for IrMatchArm { guard, body, .. } in arms {
                if let Some(guard) = guard {
                    visit_expr(guard, visit);
                }
                visit_expr(body, visit);
            }
        }
        IrExprKind::StructLiteral { fields, spread, .. } => {
            for (_, value) in fields {
                visit_expr(value, visit);
            }
            if let Some(spread) = spread {
                visit_expr(spread, visit);
            }
        }
        IrExprKind::Lambda { body, .. } => visit_expr(body, visit),
        IrExprKind::MapLiteral { keys, values } => {
            visit_exprs(keys, visit);
            visit_exprs(values, visit);
        }
        IrExprKind::List { items } => visit_exprs(items, visit),
        IrExprKind::Ask { prompt, target_ty } => {
            visit(target_ty);
            visit_expr(prompt, visit);
        }
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
        | IrExprKind::TrustBoundary { inner } => visit_expr(inner, visit),
        IrExprKind::Replay { trace, arms, else_body } => {
            visit_expr(trace, visit);
            for IrReplayArm { body, .. } in arms {
                visit_expr(body, visit);
            }
            visit_expr(else_body, visit);
        }
        IrExprKind::PageNew { items, next_cursor } => {
            visit_expr(items, visit);
            visit_expr(next_cursor, visit);
        }
        IrExprKind::TableOp { arg, .. } => visit_expr(arg, visit),
        IrExprKind::TableQuery { filter, take, .. } => {
            if let Some(filter) = filter {
                filter.for_each_value_mut(&mut |value| visit_expr(value, visit));
            }
            if let Some(take) = take {
                visit_expr(take, visit);
            }
        }
    }
}

fn visit_exprs(exprs: &mut [IrExpr], visit: &mut dyn FnMut(&mut Type)) {
    for expr in exprs {
        visit_expr(expr, visit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;

    fn expr(kind: IrExprKind, ty: Type) -> IrExpr {
        IrExpr { kind, ty, span: Span::new(0, 0) }
    }

    #[test]
    fn nested_expression_types_are_all_visited() {
        let inner = expr(IrExprKind::OptionNone, Type::Int);
        let wrapped = expr(IrExprKind::ResultOk { inner: Box::new(inner) }, Type::String);
        let mut block = IrBlock {
            stmts: vec![IrStmt::Let {
                local_id: corvid_resolve::LocalId(0),
                name: "x".into(),
                ty: Type::Bool,
                value: wrapped,
                span: Span::new(0, 0),
            }],
            span: Span::new(0, 0),
        };
        let mut seen = Vec::new();
        visit_block(&mut block, &mut |ty| seen.push(ty.clone()));
        assert_eq!(seen, vec![Type::Bool, Type::String, Type::Int]);
    }
}
