use corvid_ir::{IrCallKind, IrExpr, IrExprKind, IrLiteral, IrMatchArm, IrPattern};
use corvid_types::Type;
use corvid_resolve::LocalId;

pub(crate) fn pattern_binds(pattern: &IrPattern, id: LocalId) -> bool {
    match pattern {
        IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_binds(p, id),
        IrPattern::Variant { args, .. } => args.iter().any(|p| pattern_binds(p, id)),
        IrPattern::Bind { local_id, .. } => *local_id == id,
        _ => false,
    }
}

fn scalar_expression(expr: &IrExpr) -> bool {
    if !matches!(expr.ty, Type::Int | Type::Float | Type::Bool | Type::String) {
        return false;
    }
    match &expr.kind {
        IrExprKind::Local { .. } | IrExprKind::Literal(_) => true,
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right }
            if crate::lowering::uses_structural_equality(*op, &left.ty) =>
            native_argument(left) && native_argument(right),
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            scalar_expression(left) && scalar_expression(right)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            scalar_expression(operand)
        }
        IrExprKind::Call { .. } => native_call(expr),
        IrExprKind::BuiltinMethod { kind, receiver, args } if crate::native_string_method(*kind) || crate::native_numeric_conversion(*kind) =>
            native_argument(receiver) && args.iter().all(native_argument),
        _ => false,
    }
}

fn native_call(expr: &IrExpr) -> bool {
    matches!(&expr.kind, IrExprKind::Call { kind: IrCallKind::Agent { .. } | IrCallKind::StructConstructor { .. } | IrCallKind::EnumConstructor { .. }, args, .. }
        if args.iter().all(native_argument))
}

fn native_argument(expr: &IrExpr) -> bool {
    match &expr.kind {
        IrExprKind::OptionNone => true,
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::StringSplit, receiver, args } =>
            native_argument(receiver) && args.iter().all(native_argument),
        IrExprKind::Local { .. } | IrExprKind::Literal(_) => true,
        IrExprKind::Call { .. } => native_call(expr),
        _ => scalar_expression(expr),
    }
}

fn append_operands(body: &IrExpr) -> Option<[&IrExpr; 2]> {
    let IrExprKind::BuiltinMethod {
        kind: corvid_types::BuiltinMethodKind::ListAppend, receiver, args,
    } = &body.kind else { return None };
    let [item] = args.as_slice() else { return None };
    (matches!(receiver.kind, IrExprKind::Local { .. })
        && native_argument(item)
        && crate::native_list_append(&receiver.ty, &item.ty))
        .then_some([receiver.as_ref(), item])
}

/// Visit local reads for liveness. Match operands take ownership at the use site.
pub(crate) fn visit_arm_operands(body: &IrExpr, mut visit: impl FnMut(&IrExpr, LocalId)) {
    fn argument(expr: &IrExpr, visit: &mut impl FnMut(&IrExpr, LocalId)) {
        match &expr.kind {
            IrExprKind::Local { local_id, .. } => visit(expr, *local_id),
            IrExprKind::BuiltinMethod { receiver, args, .. } => {
                argument(receiver, visit);
                for arg in args {
                    argument(arg, visit);
                }
            }
            IrExprKind::Call { args, .. } => {
                for arg in args {
                    argument(arg, visit);
                }
            }
            IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
                argument(left, visit);
                argument(right, visit);
            }
            IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => argument(operand, visit),
            _ => {}
        }
    }
    if let Some([receiver, item]) = append_operands(body) {
        if let IrExprKind::Local { local_id, .. } = receiver.kind {
            visit(receiver, local_id);
        }
        argument(item, &mut visit);
    } else {
        argument(body, &mut visit);
    }
}

/// Return the arms in runtime tag order for two-arm Result matches whose
/// bodies extract a payload, produce a literal, compute on scalars, append
/// to a list, or call an ordinary agent.
pub fn result_match_arms<'a>(ty: &Type, arms: &'a [IrMatchArm]) -> Option<[&'a IrMatchArm; 2]> {
    if !matches!(ty, Type::Result(_, _)) {
        return None;
    }
    native_match_arms(ty, arms)
}

pub fn native_match_arms<'a>(ty: &Type, arms: &'a [IrMatchArm]) -> Option<[&'a IrMatchArm; 2]> {
    let [first, second] = arms else { return None };
    if !matches!(ty, Type::Result(_, _) | Type::Option(_)) {
        return None;
    }
    for arm in arms {
        let binding = match &arm.pattern {
            IrPattern::Some_(inner) | IrPattern::Ok_(inner) | IrPattern::Err_(inner) => match inner.as_ref() {
                IrPattern::Bind { local_id, .. } => Some(local_id),
                IrPattern::Wildcard => None,
                _ => return None,
            },
            IrPattern::None_ | IrPattern::Wildcard => None,
            _ => return None,
        };
        if arm.guard.is_some() {
            return None;
        }
        match &arm.body.kind {
            IrExprKind::Local { local_id, .. } if binding == Some(local_id) => {}
            IrExprKind::Literal(literal) if !matches!(literal, IrLiteral::Nothing) => {}
            _ if scalar_expression(&arm.body) || append_operands(&arm.body).is_some() => {}
            _ if native_call(&arm.body) => {}
            _ => return None,
        }
    }
    match (ty, &first.pattern, &second.pattern) {
        (Type::Result(_, _), IrPattern::Ok_(_), IrPattern::Err_(_) | IrPattern::Wildcard) => Some([first, second]),
        (Type::Result(_, _), IrPattern::Err_(_), IrPattern::Ok_(_) | IrPattern::Wildcard) => Some([second, first]),
        (Type::Option(_), IrPattern::Some_(_), IrPattern::None_ | IrPattern::Wildcard) => Some([first, second]),
        (Type::Option(_), IrPattern::None_, IrPattern::Some_(_) | IrPattern::Wildcard) => Some([second, first]),
        _ => None,
    }
}

/// The type checker verifies exhaustiveness. Preserve source order so a failed
/// guard can fall through to another arm for the same constructor.
pub fn supports_match(ty: &Type, arms: &[IrMatchArm]) -> bool {
    fn pattern_supported(pattern: &IrPattern) -> bool {
        match pattern {
            IrPattern::Wildcard | IrPattern::Bind { .. } | IrPattern::None_ => true,
            IrPattern::Some_(p) | IrPattern::Ok_(p) | IrPattern::Err_(p) => pattern_supported(p),
            IrPattern::Variant { args, .. } => args.iter().all(pattern_supported),
            _ => false,
        }
    }
    matches!(ty, Type::Option(_) | Type::Result(_, _) | Type::Struct(_) | Type::ImportedStruct(_)) && !arms.is_empty() && arms.iter().all(|arm| {
        pattern_supported(&arm.pattern)
            && arm.guard.as_ref().is_none_or(scalar_expression)
            && (native_argument(&arm.body) || append_operands(&arm.body).is_some())
            && !matches!(arm.body.kind, IrExprKind::Literal(IrLiteral::Nothing))
    })
}
