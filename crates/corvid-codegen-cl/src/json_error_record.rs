//! The native backend represents `JsonError` as an ordinary record.
//!
//! The checker gives `JsonError` its own type so that `T.from_json`
//! failures are typed precisely, but at run time it is a record of four
//! strings, which is how the interpreter represents it too. Rewriting
//! the type once, before ownership analysis, lets every native record
//! mechanism apply unchanged: field access, reference counting, tracing,
//! equality and trace encoding. The record carries the reserved
//! [`corvid_types::JSON_ERROR_TYPE_ID`] and the fields
//! [`corvid_types::json_error_fields`] lists, in that order.

use corvid_ir::{IrFile, IrType};
use corvid_types::{Type, JSON_ERROR_TYPE_ID};

/// Rewrite every `JsonError` in `ir` as the record, and declare the
/// record when the program mentions it.
pub(crate) fn represent_json_error_as_record(ir: &mut IrFile) {
    let mut mentioned = false;
    corvid_ir::for_each_compiled_type_mut(ir, &mut |ty| mentioned |= rewrite(ty));
    if mentioned && !ir.types.iter().any(|ty| ty.id == JSON_ERROR_TYPE_ID) {
        ir.types.push(json_error_record());
    }
}

/// The record declaration native code uses for `JsonError`, the one
/// the public JSON mapping defines.
pub fn json_error_record() -> IrType {
    corvid_public_json::json_error_decl()
}

/// Replace `JsonError` anywhere inside `ty`; report whether it occurred.
fn rewrite(ty: &mut Type) -> bool {
    match ty {
        Type::JsonError => {
            *ty = Type::Struct(JSON_ERROR_TYPE_ID);
            true
        }
        Type::List(inner)
        | Type::Stream(inner)
        | Type::Option(inner)
        | Type::Tainted(inner)
        | Type::Grounded(inner)
        | Type::Partial(inner)
        | Type::ResumeToken(inner)
        | Type::Upload(inner)
        | Type::Page(inner)
        | Type::Weak(inner, _) => rewrite(inner),
        Type::Map(a, b) | Type::Result(a, b) => rewrite(a) | rewrite(b),
        Type::Function { params, ret, .. } => {
            let mut found = rewrite(ret);
            for param in params {
                found |= rewrite(param);
            }
            found
        }
        Type::RouteParams(params) => {
            let mut found = false;
            for (_, param) in params {
                found |= rewrite(param);
            }
            found
        }
        Type::Int
        | Type::Float
        | Type::String
        | Type::Bool
        | Type::Nothing
        | Type::Struct(_)
        | Type::ImportedStruct(_)
        | Type::TraceId
        | Type::DbHandle
        | Type::JsonValue
        | Type::JsonBuilder
        | Type::RegexMatch
        | Type::Decimal
        | Type::Duration
        | Type::Instant
        | Type::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_error_is_rewritten_inside_every_container() {
        let mut ty = Type::Result(
            Box::new(Type::List(Box::new(Type::Int))),
            Box::new(Type::Option(Box::new(Type::JsonError))),
        );
        assert!(rewrite(&mut ty));
        assert_eq!(
            ty,
            Type::Result(
                Box::new(Type::List(Box::new(Type::Int))),
                Box::new(Type::Option(Box::new(Type::Struct(JSON_ERROR_TYPE_ID)))),
            )
        );
        let mut plain = Type::List(Box::new(Type::String));
        assert!(!rewrite(&mut plain));
    }

    #[test]
    fn the_record_has_the_checker_s_fields_in_order() {
        let record = json_error_record();
        let names: Vec<&str> = record.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["path", "expected", "found", "message"]);
        assert!(record.fields.iter().all(|f| f.ty == Type::String));
    }
}
