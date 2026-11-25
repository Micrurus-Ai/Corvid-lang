//! Tool calls that carry or produce a `Secret`.
//!
//! A tool whose declared parameters hold a `Secret` receives the text,
//! and every record of the call holds `<redacted>` instead: the
//! interpreter builds the recorded arguments with
//! [`crate::conv::value_to_json`] and the implementation's with
//! [`crate::conv::tool_argument_json`], and the runtime keeps them
//! apart (`corvid_runtime::secret_arguments`). Whether a call needs the
//! revealed form is decided by the tool's declared parameter types, not
//! by looking at the values.
//!
//! `secret(name)` from std/secrets and `random_token(bytes)` from
//! std/crypto are the tools whose result is a `Secret`. JSON cannot mint
//! one (`json_to_value` refuses it), so their results are decoded here.

use crate::conv::{secret_tool_result_to_value, tool_argument_json};
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::Value;
use corvid_ast::Span;
use corvid_ir::{IrTool, IrType};
use corvid_resolve::DefId;
use corvid_runtime::{RevealedToolArguments, Runtime};
use corvid_types::Type;
use std::collections::HashMap;

/// The revealed arguments for a call to `tool`, when one of its
/// declared parameters holds a `Secret`; `None` otherwise. An error when
/// an argument holds a key made by `decode_base64_after`, which has no
/// text to reveal.
pub(super) fn revealed_arguments(
    tool: &IrTool,
    recorded: &[serde_json::Value],
    arg_values: &[Value],
    types_by_id: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Option<RevealedToolArguments>, InterpError> {
    let takes_secret = tool.params.iter().any(|param| holds_secret(&param.ty, types_by_id, &mut Vec::new()));
    if !takes_secret {
        return Ok(None);
    }
    let revealed = arg_values
        .iter()
        .map(tool_argument_json)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|problem| InterpError::new(InterpErrorKind::Marshal(format!("tool `{}`: {problem}", tool.name)), span))?;
    Ok(Some(RevealedToolArguments::new(recorded, revealed)))
}

/// Whether a value of type `ty` can hold a `Secret`, through type
/// arguments and the fields of declared records and variants.
fn holds_secret(ty: &Type, types_by_id: &HashMap<DefId, &IrType>, seen: &mut Vec<DefId>) -> bool {
    match ty {
        Type::Struct(id) => {
            if seen.contains(id) {
                return false;
            }
            let Some(decl) = types_by_id.get(id) else { return false };
            seen.push(*id);
            let holds = decl
                .fields
                .iter()
                .chain(decl.variants.iter().flat_map(|variant| &variant.fields))
                .any(|field| holds_secret(&field.ty, types_by_id, seen));
            seen.pop();
            holds
        }
        // IR lowering gives imported records local ids; one that kept
        // its module identity is treated as holding a secret, which
        // only means its arguments are scanned for one.
        Type::ImportedStruct(_) => true,
        Type::List(inner)
        | Type::Option(inner)
        | Type::Tainted(inner)
        | Type::Grounded(inner)
        | Type::Weak(inner, _) => holds_secret(inner, types_by_id, seen),
        Type::Map(a, b) | Type::Result(a, b) => holds_secret(a, types_by_id, seen) || holds_secret(b, types_by_id, seen),
        other => other.mentions_secret(),
    }
}

/// `secret(name)` and `random_token(bytes)`: the runtime reads the
/// environment or draws the token and records the call with its result
/// redacted; the text becomes a `Value::Secret`.
pub(super) async fn dispatch_secret_value_tool(
    runtime: &Runtime,
    callee_name: &str,
    json_args: Vec<serde_json::Value>,
    return_ty: &Type,
    span: Span,
) -> Result<Value, InterpError> {
    let result = runtime
        .call_tool(callee_name, json_args)
        .await
        .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
    secret_tool_result_to_value(result, return_ty)
        .map_err(|e| InterpError::new(InterpErrorKind::Marshal(format!("tool `{callee_name}`: {e}")), span))
}
