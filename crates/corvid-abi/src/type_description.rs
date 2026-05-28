use crate::schema::{
    AbiGroundedType, AbiListType, AbiOptionType, AbiPartialType, AbiResultType, AbiResumeTokenType,
    AbiWeakType, ScalarTypeName, TypeDescription,
};
use corvid_ast::WeakEffect;
use corvid_resolve::{DefId, Resolved};
use corvid_types::Type;
use std::collections::HashMap;

/// Authoritative `DefId -> struct name` map built from the lowered IR's
/// type table. Because `lower_with_modules` appends imported module
/// types under their remapped (cross-module) `DefId`s, this map carries
/// the names of imported structs that are *not* present in the root
/// file's symbol table — so it is consulted before the symbol table.
pub type StructNames = HashMap<DefId, String>;

pub fn emit_type_description(
    ty: &Type,
    resolved: &Resolved,
    names: &StructNames,
) -> TypeDescription {
    match ty {
        Type::Int => TypeDescription::Scalar {
            scalar: ScalarTypeName::Int,
        },
        Type::Float => TypeDescription::Scalar {
            scalar: ScalarTypeName::Float,
        },
        Type::String => TypeDescription::Scalar {
            scalar: ScalarTypeName::String,
        },
        Type::Bool => TypeDescription::Scalar {
            scalar: ScalarTypeName::Bool,
        },
        Type::Nothing => TypeDescription::Scalar {
            scalar: ScalarTypeName::Nothing,
        },
        Type::TraceId => TypeDescription::Scalar {
            scalar: ScalarTypeName::TraceId,
        },
        Type::Struct(def_id) => TypeDescription::Struct {
            name: lookup_name(resolved, names, *def_id),
        },
        Type::ImportedStruct(imported) => TypeDescription::Struct {
            name: imported.name.clone(),
        },
        Type::List(inner) | Type::Stream(inner) => TypeDescription::List {
            list: AbiListType {
                element: Box::new(emit_type_description(inner, resolved, names)),
            },
        },
        Type::Result(ok, err) => TypeDescription::Result {
            result: AbiResultType {
                ok: Box::new(emit_type_description(ok, resolved, names)),
                err: Box::new(emit_type_description(err, resolved, names)),
            },
        },
        Type::Option(inner) => TypeDescription::Option {
            option: AbiOptionType {
                inner: Box::new(emit_type_description(inner, resolved, names)),
            },
        },
        Type::Grounded(inner) => TypeDescription::Grounded {
            grounded: AbiGroundedType {
                inner: Box::new(emit_type_description(inner, resolved, names)),
            },
        },
        Type::Partial(inner) => TypeDescription::Partial {
            partial: AbiPartialType {
                inner: Box::new(emit_type_description(inner, resolved, names)),
            },
        },
        Type::ResumeToken(inner) => TypeDescription::ResumeToken {
            resume_token: AbiResumeTokenType {
                inner: Box::new(emit_type_description(inner, resolved, names)),
            },
        },
        Type::Weak(inner, effects) => TypeDescription::Weak {
            weak: AbiWeakType {
                inner: Box::new(emit_type_description(inner, resolved, names)),
                effects: effects
                    .effects()
                    .into_iter()
                    .map(|effect| match effect {
                        WeakEffect::ToolCall => "tool_call".to_string(),
                        WeakEffect::Llm => "llm_call".to_string(),
                        WeakEffect::Approve => "approve".to_string(),
                        WeakEffect::Human => "human".to_string(),
                    })
                    .collect(),
            },
        },
        Type::Function { .. } | Type::RouteParams(_) | Type::Unknown => TypeDescription::Scalar {
            scalar: ScalarTypeName::String,
        },
    }
}

/// Resolve a struct `DefId` to its declared name.
///
/// The IR-derived `names` map is authoritative: it covers both the
/// root file's own types and the imported module types that
/// `lower_with_modules` appended under remapped (cross-module)
/// `DefId`s. Those remapped ids are out of range for the root file's
/// symbol table, so consulting the table first would panic — hence the
/// map takes precedence, with the symbol table as a same-module
/// fallback and a synthetic name as the last resort.
fn lookup_name(resolved: &Resolved, names: &StructNames, def_id: DefId) -> String {
    if let Some(name) = names.get(&def_id) {
        return name.clone();
    }
    if (def_id.0 as usize) < resolved.symbols.entries().len() {
        return resolved.symbols.get(def_id).name.clone();
    }
    format!("Struct#{}", def_id.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_resolve::resolve;
    use corvid_syntax::{lex, parse_file};

    fn tiny_resolved() -> Resolved {
        let src = "agent main() -> Int:\n    return 1\n";
        let tokens = lex(src).expect("lex");
        let (file, errs) = parse_file(&tokens);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        resolve(&file)
    }

    /// Regression: an app struct field of an imported type lowers to
    /// `Type::Struct(<remapped cross-module DefId>)`, which is out of
    /// range for the root file's symbol table. The IR-derived name map
    /// must resolve it — previously this indexed the symbol table out
    /// of bounds and panicked during cdylib ABI emission.
    #[test]
    fn out_of_range_struct_resolves_from_ir_name_map() {
        let resolved = tiny_resolved();
        let out_of_range = DefId(resolved.symbols.entries().len() as u32 + 100);
        let mut names = StructNames::new();
        names.insert(out_of_range, "Actor".to_string());
        match emit_type_description(&Type::Struct(out_of_range), &resolved, &names) {
            TypeDescription::Struct { name } => assert_eq!(name, "Actor"),
            other => panic!("expected struct description, got {other:?}"),
        }
    }

    /// An out-of-range struct id with no IR name degrades to a synthetic
    /// name rather than panicking, so a missing entry never crashes
    /// descriptor emission.
    #[test]
    fn out_of_range_struct_without_name_degrades_gracefully() {
        let resolved = tiny_resolved();
        let id = DefId(resolved.symbols.entries().len() as u32 + 7);
        match emit_type_description(&Type::Struct(id), &resolved, &StructNames::new()) {
            TypeDescription::Struct { name } => assert_eq!(name, format!("Struct#{}", id.0)),
            other => panic!("expected struct description, got {other:?}"),
        }
    }
}
