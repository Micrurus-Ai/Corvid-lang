//! Whether a type can hold a value of some kind, through type arguments
//! and the fields of declared records and variants.
//!
//! `Type::mentions` walks a type's own parts and stops at a declared
//! record. A tool whose parameter is a record with a `Secret` or an
//! `Upload` field takes one all the same, so the interpreter (deciding
//! which calls need revealed arguments, and which values cross as the
//! runtime's own) and the native tier picker ask this.

use crate::types::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use std::collections::HashMap;

/// Whether a value of type `ty` can hold a `Secret`.
pub fn holds_secret(ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> bool {
    holds(ty, types_by_id, &|ty| matches!(ty, Type::Secret))
}

/// Whether a value of type `ty` can hold a part `found` matches, looking
/// through records and variants. An imported record that kept its module
/// identity counts as holding one, which only means its values are checked.
pub fn holds(ty: &Type, types_by_id: &HashMap<DefId, &IrType>, found: &dyn Fn(&Type) -> bool) -> bool {
    walk(ty, types_by_id, found, &mut Vec::new())
}

fn walk(ty: &Type, types_by_id: &HashMap<DefId, &IrType>, found: &dyn Fn(&Type) -> bool, seen: &mut Vec<DefId>) -> bool {
    if found(ty) {
        return true;
    }
    match ty {
        Type::Struct(id) => {
            if seen.contains(id) {
                return false;
            }
            let Some(decl) = types_by_id.get(id) else { return false };
            seen.push(*id);
            let held = decl
                .fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .any(|field| walk(&field.ty, types_by_id, found, seen));
            seen.pop();
            held
        }
        // IR lowering gives imported records local ids; one that kept
        // its module identity is treated as holding the part.
        Type::ImportedStruct(_) => true,
        Type::List(inner) | Type::Option(inner) | Type::Tainted(inner) | Type::Grounded(inner) | Type::Weak(inner, _) => {
            walk(inner, types_by_id, found, seen)
        }
        Type::Map(a, b) | Type::Result(a, b) => walk(a, types_by_id, found, seen) || walk(b, types_by_id, found, seen),
        other => other.mentions(found),
    }
}
