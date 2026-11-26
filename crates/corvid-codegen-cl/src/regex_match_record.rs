//! The native backend holds a `RegexMatch` as an ordinary record.
//!
//! `find_regex` and `find_all_regex` answer with the compiler-known
//! `RegexMatch`, which the interpreter holds as a record with the fields
//! `corvid_types::regex_match_fields` declares, in that order, under the
//! reserved `REGEX_MATCH_TYPE_ID`. Rewriting the type into that record
//! once, before ownership analysis, as `JsonError` is, lets every native
//! record mechanism apply unchanged: field reads, reference counting, list
//! and option slots, and the decode plan the runtime builds a match from.

use corvid_ast::Span;
use corvid_ir::{IrField, IrFile, IrType};
use corvid_types::{Type, REGEX_MATCH_TYPE_ID};

/// Rewrite every `RegexMatch` in `ir` as its record, and declare the
/// record when the program mentions one.
pub(crate) fn represent_regex_match_as_record(ir: &mut IrFile) {
    let mut mentioned = false;
    corvid_ir::for_each_compiled_type_mut(ir, &mut |slot| {
        mentioned |= slot.replace_within(&mut |ty| {
            if matches!(ty, Type::RegexMatch) {
                *ty = Type::Struct(REGEX_MATCH_TYPE_ID);
                true
            } else {
                false
            }
        });
    });
    if mentioned && !ir.types.iter().any(|ty| ty.id == REGEX_MATCH_TYPE_ID) {
        ir.types.push(regex_match_record());
    }
}

/// The record declaration native code uses for `RegexMatch`.
pub fn regex_match_record() -> IrType {
    let span = Span::new(0, 0);
    IrType {
        id: REGEX_MATCH_TYPE_ID,
        name: "RegexMatch".into(),
        fields: corvid_types::regex_match_fields()
            .into_iter()
            .map(|(name, ty)| IrField { name: name.into(), ty, refinement: None, json_key: None, span })
            .collect(),
        variants: Vec::new(),
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_record_has_the_interpreter_s_fields_in_order() {
        let names: Vec<String> = regex_match_record().fields.iter().map(|f| f.name.clone()).collect();
        let expected: Vec<String> = corvid_types::regex_match_fields().iter().map(|(name, _)| name.to_string()).collect();
        assert_eq!(names, expected);
    }
}
