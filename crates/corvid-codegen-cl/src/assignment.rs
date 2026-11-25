//! Native assignment paths and element layouts.
use crate::native_string_map;

/// Compound arithmetic on a numeric local reuses its existing binding.
pub fn native_local_compound(stmt: &corvid_ir::IrStmt) -> bool {
    use corvid_types::Type;
    matches!(stmt, corvid_ir::IrStmt::Assign { root_ty: Type::Int | Type::Float, path, op: Some(_), .. } if path.is_empty())
}

/// Native record writes can traverse a chain of fields and use numeric compound operators.
pub fn native_record_assignment(stmt: &corvid_ir::IrStmt) -> bool {
    use corvid_ir::{IrPathSeg, IrStmt};
    use corvid_types::Type;
    matches!(stmt, IrStmt::Assign { root_ty: Type::Struct(_), path, op, value, .. }
        if !path.is_empty() && path.iter().all(|part| matches!(part, IrPathSeg::Field(_)))
            && (op.is_none() || matches!(value.ty, Type::Int | Type::Float)))
}

/// Collect index expressions in evaluation order and resolve the final slot type.
pub fn native_indexed_assignment(
    stmt: &corvid_ir::IrStmt,
) -> Option<(
    corvid_resolve::LocalId,
    Vec<&corvid_ir::IrExpr>,
    &corvid_ir::IrExpr,
    &corvid_types::Type,
)> {
    use corvid_types::Type;
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
    if path.is_empty() {
        return None;
    }
    let mut element = root_ty;
    let mut indices = Vec::with_capacity(path.len());
    for part in path {
        let corvid_ir::IrPathSeg::Index(index) = part else {
            return None;
        };
        element = match element {
            Type::List(item) => item,
            Type::Map(_, item) if path.len() == 1 && native_string_map(root_ty) => item,
            _ => return None,
        };
        indices.push(index);
    }
    if op.is_some() && !matches!(element, Type::Int | Type::Float) {
        return None;
    }
    Some((*local_id, indices, value, element))
}

/// Element layouts supported by the native list append operation.
pub fn native_list_append(receiver: &corvid_types::Type, item: &corvid_types::Type) -> bool {
    use corvid_types::Type;
    let Type::List(element) = receiver else {
        return false;
    };
    if **element == Type::Float && *item == Type::Int {
        return true;
    }
    // These slots have a concrete native representation and, for heap
    // elements, a corresponding list element destructor and tracer.
    element.as_ref() == item
        && matches!(
            item,
            Type::Int
                | Type::Float
                | Type::Bool
                | Type::String
                | Type::Decimal
                | Type::Duration
                | Type::Instant
                | Type::TimeZone
                | Type::ZonedTime
                | Type::Currency
                | Type::Locale
                | Type::Bytes
                | Type::Struct(_)
                | Type::List(_)
                | Type::Result(_, _)
                | Type::Option(_)
                | Type::Weak(_, _)
        )
}
