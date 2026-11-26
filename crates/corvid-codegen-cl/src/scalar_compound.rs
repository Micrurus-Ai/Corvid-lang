//! Compound assignment on a local of a text-held scalar type (`Decimal`,
//! `Duration`, `Instant`, `TimeZone`, `ZonedTime`, `Currency`, `Locale`).
//!
//! `total += line.price` on a `Decimal` local means exactly
//! `total = total + line.price`, and that rebinding is native already:
//! the operator is one runtime call, and the rebinding releases the old
//! value through the path every reassignment of a reference-counted local
//! takes. So the code generator rewrites the one into the other once,
//! before ownership analysis, and the analysis and lowering both see a
//! form they already handle. The checker has already refused what the
//! interpreter refuses (`total /= 3` without a rounding mode, mixing
//! `Decimal` and `Float`).

use corvid_ir::{IrBlock, IrExpr, IrExprKind, IrFile, IrStmt};

/// Whether `stmt` is a compound assignment this module rewrites: a local
/// of a text-held scalar type, with no field or index path.
pub fn native_scalar_compound(stmt: &IrStmt) -> bool {
    matches!(stmt, IrStmt::Assign { root_ty, path, op: Some(_), .. }
        if path.is_empty() && crate::text_scalar::is_text_scalar(crate::text_scalar::scalar_of(root_ty)))
}

/// Rewrite every scalar compound assignment in `ir` as its rebinding.
pub(crate) fn rewrite_scalar_compound_assignments(ir: &mut IrFile) {
    for agent in &mut ir.agents {
        rewrite_block(&mut agent.body);
    }
}

fn rewrite_block(block: &mut IrBlock) {
    for stmt in &mut block.stmts {
        match stmt {
            IrStmt::If { then_block, else_block, .. } => {
                rewrite_block(then_block);
                if let Some(else_block) = else_block {
                    rewrite_block(else_block);
                }
            }
            IrStmt::For { body, .. } | IrStmt::While { body, .. } | IrStmt::Atomic { body, .. } => rewrite_block(body),
            _ if native_scalar_compound(stmt) => {
                let IrStmt::Assign { local_id, root_ty, name, op: Some(op), value, span, .. } = stmt else {
                    unreachable!("checked by native_scalar_compound")
                };
                let current = IrExpr { kind: IrExprKind::Local { local_id: *local_id, name: name.clone() }, ty: root_ty.clone(), span: *span };
                let combined = IrExpr {
                    kind: IrExprKind::BinOp { op: *op, left: Box::new(current), right: Box::new(value.clone()) },
                    ty: root_ty.clone(),
                    span: *span,
                };
                *stmt = IrStmt::Let { local_id: *local_id, name: name.clone(), ty: root_ty.clone(), value: combined, span: *span };
            }
            _ => {}
        }
    }
}
