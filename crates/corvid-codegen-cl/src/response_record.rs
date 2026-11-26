//! The native backend holds a `Response<Body>` as an ordinary record
//! (native dispatch N4).
//!
//! A connector operation that returns `Response<Body>` answers with its
//! status, its response headers and its decoded body. The interpreter
//! holds that as a record with the fields `corvid_types::RESPONSE_FIELDS`
//! names, in that order. Native records have fixed field types, so each
//! distinct body type gets its own record: `status: Int`,
//! `headers: List<List<String>>` (name and value pairs) and
//! `body: Body`, with an id from `corvid_types::RESPONSE_RECORD_IDS`.
//! Rewriting the type once, before ownership analysis, lets every
//! native record mechanism apply unchanged, as for `JsonError`. The
//! accessors `status()`, `body()` and `header(name)` are lowered in
//! `lowering/expr` against this layout.

use corvid_ast::Span;
use corvid_ir::{IrField, IrFile, IrType};
use corvid_resolve::DefId;
use corvid_types::{Type, RESPONSE_FIELDS, RESPONSE_RECORD_IDS};

/// Rewrite every `Response<Body>` in `ir` as its record, and declare
/// one record per distinct body type.
pub(crate) fn represent_responses_as_records(ir: &mut IrFile) {
    let mut bodies: Vec<Type> = Vec::new();
    corvid_ir::for_each_compiled_type_mut(ir, &mut |slot| {
        slot.replace_within(&mut |ty| {
            let Type::Response(body) = ty else { return false };
            let index = bodies.iter().position(|seen| seen == body.as_ref()).unwrap_or_else(|| {
                bodies.push(body.as_ref().clone());
                bodies.len() - 1
            });
            *ty = Type::Struct(record_id(index));
            true
        });
    });
    for (index, body) in bodies.into_iter().enumerate() {
        ir.types.push(response_record(record_id(index), body));
    }
}

fn record_id(index: usize) -> DefId {
    let id = RESPONSE_RECORD_IDS.start + u32::try_from(index).expect("fewer body types than the band holds");
    assert!(RESPONSE_RECORD_IDS.contains(&id), "more distinct Response body types than the reserved band holds");
    DefId(id)
}

fn response_record(id: DefId, body: Type) -> IrType {
    let span = Span::new(0, 0);
    let field = |name: &str, ty: Type| IrField { name: name.into(), ty, refinement: None, json_key: None, span };
    let types = [
        Type::Int,
        Type::List(Box::new(Type::List(Box::new(Type::String)))),
        body,
    ];
    IrType {
        id,
        name: "Response".into(),
        fields: RESPONSE_FIELDS.iter().zip(types).map(|(name, ty)| field(name, ty)).collect(),
        variants: Vec::new(),
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_body_type_gets_one_record_with_the_interpreter_s_fields() {
        let record = response_record(record_id(0), Type::String);
        let names: Vec<&str> = record.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, RESPONSE_FIELDS);
        assert_eq!(record.fields[2].ty, Type::String);
        assert!(RESPONSE_RECORD_IDS.contains(&record.id.0));
    }
}
