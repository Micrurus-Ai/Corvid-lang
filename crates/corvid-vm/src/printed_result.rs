//! How `corvid run` prints what the entry agent returned, the form both
//! tiers print ([proposal](../../../docs/proposals/printed-results.md)):
//! a `String` as its text, another scalar as its `to_string()` text, and
//! a structured value (record, variant, list, option, result, map) as its
//! public JSON form by declared type, the JSON a route returns.

use crate::public_json::SecretArgument;
use crate::value::Value;
use corvid_ir::IrFile;
use corvid_types::Type;
use std::collections::HashMap;

/// The text `corvid run` prints for `value`, the result of `agent`.
pub fn printed_result(ir: &IrFile, agent: &corvid_ir::IrAgent, value: &Value) -> String {
    let declared = plain(&agent.return_ty);
    match value {
        Value::String(text) => text.to_string(),
        _ if is_structured(declared) => {
            let types: HashMap<_, _> = ir.types.iter().map(|ty| (ty.id, ty)).collect();
            crate::interp::tool_json::tool_value_json(value, declared, &types, SecretArgument::Recorded)
                .map(|json| json.to_string())
                // A value the public form cannot write (a NaN inside a
                // record) still prints, in the interpreter's own form.
                .unwrap_or_else(|_| value.to_string())
        }
        _ => value.to_string(),
    }
}

/// The type a result is printed by: provenance and taint are not part of
/// what it prints.
fn plain(ty: &Type) -> &Type {
    match ty {
        Type::Grounded(inner) | Type::Tainted(inner) => plain(inner),
        other => other,
    }
}

fn is_structured(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Struct(_) | Type::ImportedStruct(_) | Type::List(_) | Type::Option(_) | Type::Result(_, _) | Type::Map(_, _)
    )
}
