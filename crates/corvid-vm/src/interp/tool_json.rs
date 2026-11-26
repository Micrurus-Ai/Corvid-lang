//! The JSON a tool call exchanges (native scalar boundaries B2a).
//!
//! A tool's arguments are encoded by their parameters' declared types in
//! the public form, the one `to_json` writes, HTTP bodies carry, a model
//! is shown, and native runtime calls send. They are what the trace
//! records, what an approver is shown, and what a Python, host or Rust
//! tool receives. The result is decoded by the declared return type with
//! the public decoder, so a tool that answers with the wrong shape is
//! refused with the field path. Undeclared keys in a result are ignored,
//! as `T.from_json` ignores them.
//!
//! A value whose type has no public form, the runtime's own handles,
//! crosses as the handle the runtime's tools accept. Which types those
//! are is decided by the declared type, never by an encoding failure.

use std::collections::HashMap;

use corvid_ir::{IrTool, IrType};
use corvid_public_json::UnknownFields;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::Value as J;

use crate::conv::{json_to_value, value_to_json};
use crate::public_json::{decode_public_json, encode_tool_argument, SecretArgument};
use crate::value::Value;

/// A type holding a database connection or a JSON builder anywhere, such
/// as `db_open`'s `Result<DbHandle, String>`: a handle into the runtime,
/// which only the runtime's own tools take and which has no public form.
fn crosses_as_handle(ty: &Type) -> bool {
    ty.mentions(&|ty| matches!(ty, Type::DbHandle | Type::JsonBuilder))
}

/// One value as a tool call carries it, given its declared type.
pub(crate) fn tool_value_json(
    value: &Value,
    ty: &Type,
    types: &HashMap<DefId, &IrType>,
    secrets: SecretArgument,
) -> Result<J, String> {
    if crosses_as_handle(ty) {
        return Ok(value_to_json(value));
    }
    // A grounded value given where its plain type is declared is that
    // value: its provenance stays with the runtime, as for a prompt.
    if let (Value::Grounded(grounded), false) = (value, matches!(ty, Type::Grounded(_))) {
        return tool_value_json(&grounded.inner.get(), ty, types, secrets);
    }
    encode_tool_argument(value, ty, types, secrets)
}

/// A call's arguments, each by its parameter's declared type.
pub(crate) fn tool_arguments(
    tool: &IrTool,
    values: &[Value],
    types: &HashMap<DefId, &IrType>,
    secrets: SecretArgument,
) -> Result<Vec<J>, String> {
    tool.params
        .iter()
        .zip(values)
        .map(|(param, value)| {
            tool_value_json(value, &param.ty, types, secrets)
                .map_err(|problem| format!("tool `{}` argument `{}`: {problem}", tool.name, param.name))
        })
        .collect()
}

/// A tool's answer read as its declared return type.
pub(crate) fn tool_result(json: J, ty: &Type, types: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    if crosses_as_handle(ty) {
        return json_to_value(json, ty, types).map_err(|e| e.to_string());
    }
    decode_public_json(&json, ty, types, UnknownFields::Ignore).map_err(|error| error.to_string())
}
