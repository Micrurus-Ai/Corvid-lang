//! Where a native text-held scalar stops (B06): tool and prompt
//! boundaries.
//!
//! Inside a native program a `Decimal`, `Duration`, `Instant`,
//! `TimeZone`, `ZonedTime`, `Currency` or `Locale` is a String holding
//! its canonical text. A tool written in Rust or a prompt's model reply
//! would see or supply that String with nothing to check that it is
//! the type's text, so a tool or prompt whose signature holds one of
//! these types anywhere, including inside a record, runs on the
//! interpreter, which converts and checks at the boundary. That keeps
//! `time_now()`, a `std/time` tool returning an `Instant`, on the
//! interpreter, where replay records and substitutes the clock. The
//! tier picker and the tool and prompt lowering both ask this one
//! predicate.

use crate::text_scalar::is_text_scalar;
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use std::collections::HashSet;

/// Why a call is refused natively, for the compile error. `type_name`
/// is the text-held type the signature holds.
pub fn text_scalar_boundary_requirement(type_name: &str) -> String {
    format!(
        "a tool of the program's own whose signature holds a `{type_name}` runs on the interpreter tier, which checks the value's text at the boundary; the native tool bridge carries no `{type_name}` form yet (native scalar boundaries B2)"
    )
}

/// The first text-held scalar type `ty` holds, looking through
/// containers and record fields. `types` finds a record's declaration.
pub fn held_text_scalar(ty: &Type, types: &dyn Fn(DefId) -> Option<IrType>) -> Option<Type> {
    held(ty, types, &mut HashSet::new())
}

fn held(ty: &Type, types: &dyn Fn(DefId) -> Option<IrType>, seen: &mut HashSet<DefId>) -> Option<Type> {
    match ty {
        ty if is_text_scalar(ty) => Some(ty.clone()),
        Type::List(inner)
        | Type::Option(inner)
        | Type::Tainted(inner)
        | Type::Grounded(inner)
        | Type::Stream(inner)
        | Type::Partial(inner)
        | Type::Weak(inner, _) => held(inner, types, seen),
        Type::Map(key, value) | Type::Result(key, value) => held(key, types, seen).or_else(|| held(value, types, seen)),
        Type::Struct(id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id: id, .. }) => {
            if !seen.insert(*id) {
                return None;
            }
            let decl = types(*id)?;
            decl.fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .find_map(|field| held(&field.ty, types, seen))
        }
        Type::Function { params, ret, .. } => {
            held(ret, types, seen).or_else(|| params.iter().find_map(|param| held(param, types, seen)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::Span;
    use corvid_ir::IrField;

    #[test]
    fn a_text_scalar_inside_a_record_or_container_is_found() {
        let line = IrType {
            id: DefId(1),
            name: "Line".into(),
            fields: vec![IrField { name: "price".into(), ty: Type::Decimal, refinement: None, json_key: None, span: Span::new(0, 0) }],
            variants: vec![],
            span: Span::new(0, 0),
        };
        let types = |id: DefId| (id == DefId(1)).then(|| line.clone());
        assert_eq!(held_text_scalar(&Type::List(Box::new(Type::Struct(DefId(1)))), &types), Some(Type::Decimal));
        assert_eq!(held_text_scalar(&Type::Option(Box::new(Type::Instant)), &types), Some(Type::Instant));
        assert_eq!(held_text_scalar(&Type::Result(Box::new(Type::Int), Box::new(Type::Locale)), &types), Some(Type::Locale));
        assert_eq!(held_text_scalar(&Type::List(Box::new(Type::String)), &types), None);
        assert!(text_scalar_boundary_requirement("ZonedTime").contains("`ZonedTime`"));
    }
}
