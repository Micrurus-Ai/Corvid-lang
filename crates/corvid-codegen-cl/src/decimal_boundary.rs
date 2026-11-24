//! Where a native `Decimal` stops (B06): tool and prompt boundaries.
//!
//! Inside a native program a `Decimal` is a String holding its exact
//! text. A tool written in Rust or a prompt's model reply would see or
//! supply that String with nothing to check it is decimal text, so a
//! tool or prompt whose signature holds a `Decimal` anywhere, including
//! inside a record, runs on the interpreter, which converts at the
//! boundary. The tier picker and the tool and prompt lowering both ask
//! this one predicate.

use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use std::collections::HashSet;

/// Why a call is refused natively, for the compile error.
pub const DECIMAL_BOUNDARY_REQUIREMENT: &str = "a tool or prompt whose signature holds a `Decimal` runs on the interpreter tier, which checks the decimal text at the boundary; native tool and prompt bridges carry no `Decimal` form yet";

/// Whether `ty` holds a `Decimal`, looking through containers and
/// record fields. `types` finds a record's declaration.
pub fn holds_decimal(ty: &Type, types: &dyn Fn(DefId) -> Option<IrType>) -> bool {
    holds(ty, types, &mut HashSet::new())
}

fn holds(ty: &Type, types: &dyn Fn(DefId) -> Option<IrType>, seen: &mut HashSet<DefId>) -> bool {
    match ty {
        Type::Decimal => true,
        Type::List(inner)
        | Type::Option(inner)
        | Type::Tainted(inner)
        | Type::Grounded(inner)
        | Type::Stream(inner)
        | Type::Partial(inner)
        | Type::Weak(inner, _) => holds(inner, types, seen),
        Type::Map(key, value) | Type::Result(key, value) => holds(key, types, seen) || holds(value, types, seen),
        Type::Struct(id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id: id, .. }) => {
            if !seen.insert(*id) {
                return false;
            }
            types(*id).is_some_and(|decl| {
                decl.fields
                    .iter()
                    .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                    .any(|field| holds(&field.ty, types, seen))
            })
        }
        Type::Function { params, ret, .. } => holds(ret, types, seen) || params.iter().any(|param| holds(param, types, seen)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;
    use corvid_ir::IrField;

    #[test]
    fn a_decimal_inside_a_record_or_container_is_found() {
        let line = IrType {
            id: DefId(1),
            name: "Line".into(),
            fields: vec![IrField { name: "price".into(), ty: Type::Decimal, refinement: None, span: Span::new(0, 0) }],
            variants: vec![],
            span: Span::new(0, 0),
        };
        let types = |id: DefId| (id == DefId(1)).then(|| line.clone());
        assert!(holds_decimal(&Type::List(Box::new(Type::Struct(DefId(1)))), &types));
        assert!(holds_decimal(&Type::Option(Box::new(Type::Decimal)), &types));
        assert!(!holds_decimal(&Type::List(Box::new(Type::String)), &types));
    }
}
