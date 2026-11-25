//! Whether a type can hold a `Secret`, through type arguments and the
//! fields of declared records and variants.
//!
//! `Type::mentions_secret` walks a type's own parts and stops at a
//! declared record. A tool whose parameter is a record with a `Secret`
//! field takes a secret all the same, so the interpreter (deciding which
//! calls need revealed arguments) and the native tier picker (deciding
//! which calls native code cannot redact) both ask this.

use crate::types::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use std::collections::HashMap;

/// Whether a value of type `ty` can hold a `Secret`.
pub fn holds_secret(ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> bool {
    holds(ty, types_by_id, &mut Vec::new())
}

fn holds(ty: &Type, types_by_id: &HashMap<DefId, &IrType>, seen: &mut Vec<DefId>) -> bool {
    match ty {
        Type::Struct(id) => {
            if seen.contains(id) {
                return false;
            }
            let Some(decl) = types_by_id.get(id) else { return false };
            seen.push(*id);
            let found = decl
                .fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .any(|field| holds(&field.ty, types_by_id, seen));
            seen.pop();
            found
        }
        // IR lowering gives imported records local ids; one that kept
        // its module identity is treated as holding a secret, which
        // only means its values are checked for one.
        Type::ImportedStruct(_) => true,
        Type::List(inner) | Type::Option(inner) | Type::Tainted(inner) | Type::Grounded(inner) | Type::Weak(inner, _) => {
            holds(inner, types_by_id, seen)
        }
        Type::Map(a, b) | Type::Result(a, b) => holds(a, types_by_id, seen) || holds(b, types_by_id, seen),
        other => other.mentions_secret(),
    }
}
