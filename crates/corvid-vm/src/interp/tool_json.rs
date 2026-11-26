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

/// A type holding a value the runtime keeps anywhere, such as
/// `db_open`'s `Result<DbHandle, String>`: a database connection, a JSON
/// builder, or an uploaded file, which a connector sends by its bytes and
/// records by digest. Only the runtime's own tools take these, and they
/// have no public form.
fn crosses_as_handle(ty: &Type, types: &HashMap<DefId, &IrType>) -> bool {
    corvid_ir::holds(ty, types, &|ty| matches!(ty, Type::DbHandle | Type::JsonBuilder | Type::Upload(_)))
}

/// One value as a tool call carries it, given its declared type.
pub(crate) fn tool_value_json(
    value: &Value,
    ty: &Type,
    types: &HashMap<DefId, &IrType>,
    secrets: SecretArgument,
) -> Result<J, String> {
    if crosses_as_handle(ty, types) {
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
    if crosses_as_handle(ty, types) {
        return json_to_value(json, ty, types).map_err(|e| e.to_string());
    }
    decode_public_json(&json, ty, types, UnknownFields::Ignore).map_err(|error| error.to_string())
}

/// A tool's answer as the step debugger shows it: the public form, or for
/// a type that has none (a connector `Response`, a runtime handle) the
/// interpreter's own form, decided by the type.
pub(crate) fn debugger_view(value: &Value, ty: &Type, types: &HashMap<DefId, &IrType>) -> J {
    if crosses_as_handle(ty, types) || corvid_ir::holds(ty, types, &|ty| matches!(ty, Type::Response(_))) {
        return value_to_json(value);
    }
    encode_tool_argument(value, ty, types, SecretArgument::Recorded)
        .unwrap_or_else(|problem| serde_json::json!({ "unencodable": problem }))
}

/// A value a trace records (a run's arguments and result, an `approve`'s
/// arguments): the public form by its declared type, a `Secret` as
/// `<redacted>`, as native code records it. A record is an observation and
/// never stops the run, so a type with no public form (a stream, a handle)
/// or a value the form cannot write (a malformed record, a NaN) is
/// recorded in the interpreter's own form, which writes a NaN as `null`, as
/// native code does.
pub(crate) fn recorded_value_json(value: &Value, ty: &Type, types: &HashMap<DefId, &IrType>) -> J {
    let value = without_file_contents(value);
    tool_value_json(&value, ty, types, SecretArgument::Recorded).unwrap_or_else(|_| value_to_json(&value))
}

/// `value` with each uploaded file replaced by what a trace may hold of
/// it: its name, type, size and SHA-256 digest, the form a connector
/// records an upload in, so a file never reaches a trace however deep in
/// an argument it sits.
fn without_file_contents(value: &Value) -> Value {
    use crate::value::{BoxedValue, ListValue, StructValue};
    match value {
        Value::Struct(record) if record.type_name() == "Upload" => {
            let field = |name: &str| record.get_field(name).unwrap_or(Value::Nothing);
            let digest = match record.get_field("bytes") {
                Some(Value::Bytes(bytes)) => corvid_crypto::sha256(&bytes).to_hex(),
                _ => String::new(),
            };
            Value::Struct(StructValue::new(
                record.type_id(),
                "Upload",
                vec![
                    ("filename".to_string(), field("filename")),
                    ("content_type".to_string(), field("content_type")),
                    ("size".to_string(), field("size")),
                    ("sha256".to_string(), Value::String(digest.into())),
                ],
            ))
        }
        Value::Struct(record) => {
            let fields: Vec<(String, Value)> =
                record.with_fields(|fields| fields.iter().map(|(name, field)| (name.clone(), without_file_contents(field))).collect());
            Value::Struct(StructValue::new(record.type_id(), record.type_name(), fields))
        }
        Value::List(items) => Value::List(ListValue::new(items.iter_cloned().iter().map(without_file_contents))),
        Value::OptionSome(inner) => Value::OptionSome(BoxedValue::new(without_file_contents(&inner.get()))),
        Value::ResultOk(inner) => Value::ResultOk(BoxedValue::new(without_file_contents(&inner.get()))),
        Value::ResultErr(inner) => Value::ResultErr(BoxedValue::new(without_file_contents(&inner.get()))),
        other => other.clone(),
    }
}

