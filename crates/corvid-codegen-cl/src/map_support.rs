//! Map shapes shared by native eligibility, ownership analysis and lowering.
pub const NATIVE_MAP_REQUIREMENTS: &str = "native maps require String keys and Int, String, local record, List or Option values of supported types";

pub fn native_string_map(ty: &corvid_types::Type) -> bool {
    use corvid_types::Type;
    fn supported_value(ty: &Type) -> bool {
        match ty {
            Type::String | Type::Int | Type::Struct(_) => true,
            Type::Option(inner) | Type::List(inner) => supported_value(inner),
            _ => false,
        }
    }
    matches!(ty, Type::Map(key, value) if **key == Type::String && supported_value(value))
}

pub fn native_string_map_lookup(
    expr: &corvid_ir::IrExpr,
) -> Option<(&corvid_ir::IrExpr, &corvid_ir::IrExpr)> {
    use corvid_ir::IrExprKind;
    let (receiver, key) = match &expr.kind {
        IrExprKind::Index { target, index } => (target.as_ref(), index.as_ref()),
        IrExprKind::BuiltinMethod {
            kind:
                corvid_types::BuiltinMethodKind::MapGet
                | corvid_types::BuiltinMethodKind::MapContainsKey,
            receiver,
            args,
        } => {
            let [key] = args.as_slice() else { return None };
            (receiver.as_ref(), key)
        }
        _ => return None,
    };
    native_string_map(&receiver.ty).then_some((receiver, key))
}

pub fn string_map_assignment(
    stmt: &corvid_ir::IrStmt,
) -> Option<(
    corvid_resolve::LocalId,
    &corvid_ir::IrExpr,
    &corvid_ir::IrExpr,
)> {
    let corvid_ir::IrStmt::Assign {
        local_id,
        root_ty,
        path,
        op,
        value,
        ..
    } = stmt
    else {
        return None;
    };
    let [corvid_ir::IrPathSeg::Index(key)] = path.as_slice() else {
        return None;
    };
    let supported_op = op.is_none()
        || matches!(root_ty, corvid_types::Type::Map(_, elem) if **elem == corvid_types::Type::Int);
    (native_string_map(root_ty) && supported_op).then_some((*local_id, key, value))
}
