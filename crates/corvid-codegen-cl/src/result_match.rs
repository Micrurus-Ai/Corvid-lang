//! Pattern and operand shapes supported by native Option and Result matches.
use corvid_ir::{IrExpr, IrExprKind, IrLiteral, IrMatchArm, IrPattern};
use corvid_types::Type;

fn pattern_supported(p: &IrPattern) -> bool {
    match p {
        IrPattern::Wildcard | IrPattern::Bind { .. } | IrPattern::None_ => true,
        IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_supported(p),
        IrPattern::Variant { args, .. } => args.iter().all(pattern_supported),
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
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if (left.ty == Type::Int && right.ty == Type::Int) || (left.ty == Type::String && right.ty == Type::String)
                || crate::decimal_methods::is_decimal_binary(*op, &left.ty, &right.ty)
                || crate::time_methods::time_binary(*op, &left.ty, &right.ty).is_some()
                || crate::currency_methods::currency_binary(*op, &left.ty, &right.ty).is_some()
                || crate::bytes_methods::bytes_binary(*op, &left.ty, &right.ty).is_some()
                || (matches!(op, corvid_ast::BinaryOp::Eq | corvid_ast::BinaryOp::NotEq)
                    && matches!(crate::text_scalar::scalar_of(&left.ty), Type::Secret)) => {
                operand_supported(left) && operand_supported(right)
            }
        IrExprKind::Index { target, index } => operand_supported(target) && operand_supported(index),
        // `Err(problem) -> problem.message`: a record field read.
        IrExprKind::FieldAccess { target, .. } => operand_supported(target),
        IrExprKind::BuiltinMethod { receiver, args, kind } => {
            use corvid_types::BuiltinMethodKind::*;
            (crate::native_string_method(*kind) || crate::native_text_method(*kind) || crate::is_native_decimal_method(*kind) || crate::is_native_time_method(*kind) || crate::is_native_currency_method(*kind) || crate::is_native_bytes_method(*kind) || crate::native_numeric_conversion(*kind) || crate::native_scalar_to_string(*kind) || matches!(kind, ListLength | ListAppend | ListFirst | ListLast | MapKeys | MapValues | MapLength | JsonEncode | ResponseStatus | ResponseBody | ResponseHeader))
                && operand_supported(receiver)
                && args.iter().all(operand_supported)
        }
        // A tool or prompt call lowers in an arm as anywhere: each call
        // releases its own temporaries before it returns, so nothing it
        // allocates outlives the arm that made it.
        IrExprKind::Call {
            kind: corvid_ir::IrCallKind::Agent { .. }
                | corvid_ir::IrCallKind::StructConstructor { .. }
                | corvid_ir::IrCallKind::EnumConstructor { .. }
                | corvid_ir::IrCallKind::Tool { .. }
                | corvid_ir::IrCallKind::Prompt { .. },
            args,
            ..
        } => args.iter().all(operand_supported),
        _ => false,
    }
}

pub fn supports_match(ty: &Type, arms: &[IrMatchArm]) -> bool {
    matches!(ty, Type::Option(_) | Type::Result(_, _) | Type::Struct(_) | Type::ImportedStruct(_))
        && !arms.is_empty()
        && arms.iter().all(|arm| {
            pattern_supported(&arm.pattern)
                && arm.guard.as_ref().map_or(true, operand_supported)
                && operand_supported(&arm.body)
        })
}

pub(crate) fn pattern_binds(p: &IrPattern, id: corvid_resolve::LocalId) -> bool {
    match p {
        IrPattern::Variant { args, .. } => args.iter().any(|arg| pattern_binds(arg, id)),
        IrPattern::Bind { local_id, .. } => id == *local_id,
        IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_binds(p, id),
        _ => false,
    }
}
