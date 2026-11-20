//! Pattern and operand shapes supported by native Option and Result matches.
use corvid_ir::{IrExpr, IrExprKind, IrLiteral, IrMatchArm, IrPattern};
use corvid_types::Type;

fn pattern_supported(p: &IrPattern) -> bool {
    match p {
        IrPattern::Wildcard | IrPattern::Bind { .. } | IrPattern::None_ => true,
        IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_supported(p),
        _ => false,
    }
}

fn operand_supported(expr: &IrExpr) -> bool {
    match &expr.kind {
        IrExprKind::Literal(IrLiteral::Nothing) => false,
        IrExprKind::Local { .. } | IrExprKind::Literal(_) => true,
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            operand_supported(operand)
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. }
            if left.ty == Type::Int && right.ty == Type::Int => {
                operand_supported(left) && operand_supported(right)
            }
        IrExprKind::Index { target, index } => operand_supported(target) && operand_supported(index),
        IrExprKind::BuiltinMethod { receiver, args, kind } => {
            use corvid_types::BuiltinMethodKind::*;
            matches!(kind, ListLength | ListAppend | ListFirst | ListLast | MapKeys | MapValues | MapLength)
                && operand_supported(receiver)
                && args.iter().all(operand_supported)
        }
        IrExprKind::Call {
            kind: corvid_ir::IrCallKind::Agent { .. },
            args,
            ..
        } => args.iter().all(operand_supported),
        _ => false,
    }
}

pub fn supports_match(ty: &Type, arms: &[IrMatchArm]) -> bool {
    matches!(ty, Type::Option(_) | Type::Result(_, _))
        && !arms.is_empty()
        && arms.iter().all(|arm| {
            pattern_supported(&arm.pattern)
                && arm.guard.as_ref().map_or(true, operand_supported)
                && operand_supported(&arm.body)
        })
}

pub(crate) fn pattern_binds(p: &IrPattern, id: corvid_resolve::LocalId) -> bool {
    match p {
        IrPattern::Bind { local_id, .. } => id == *local_id,
        IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_binds(p, id),
        _ => false,
    }
}
