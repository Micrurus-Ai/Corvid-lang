//! Map shapes shared by native eligibility, ownership analysis and lowering.
pub const NATIVE_MAP_REQUIREMENTS: &str = "native maps require String, Decimal, time, currency, locale or Bytes keys and Int, String, text-held scalar, local record, List or Option values of supported types";

/// Whether native code holds a map of type `ty`: its keys are one text
/// slot (a `String`, or a value held as canonical text, which the map finds
/// by value through the type's own comparison, so `1.0` and `1.00` are one
/// `Decimal` key), and its values are of a supported kind (B3).
pub fn native_string_map(ty: &corvid_types::Type) -> bool {
    use corvid_types::Type;
    fn supported_value(ty: &Type) -> bool {
        match ty {
            Type::String | Type::Int | Type::Struct(_) => true,
            text if crate::text_scalar::is_text_scalar(text) => true,
            Type::Option(inner) | Type::List(inner) => supported_value(inner),
            _ => false,
        }
    }
    matches!(ty, Type::Map(key, value)
        if corvid_abi::equality::EqualityLayout::of_text_slot(crate::text_scalar::scalar_of(key)).is_some()
            && supported_value(value))
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
