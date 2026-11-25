//! The wire JSON of a connector call.
//!
//! A connector talks to a real API, so its arguments are sent and its
//! responses read through B01's public JSON mapping, the one
//! `T.to_json()`, `T.from_json(text)`, routes and the Application
//! Contract use: an `Option` is the plain value or `null`, never Corvid's
//! internal `{"tag": ...}` envelope. A response that does not match the
//! declared type fails with the field path, the expected kind and what
//! was found. Undeclared fields in a response are ignored, because
//! providers add fields without notice.
//!
//! The runtime still wraps a response for a `Result`-returning operation
//! as `{"tag":"ok","ok":<body>}` or a mapped status as
//! `{"tag":"err","err":<variant>}`; that envelope is Corvid's own, so it is
//! read here, and only the provider's body goes through the public
//! decoder. The recorded `ToolResult` keeps that shape, so a trace
//! replays through exactly this function.

use crate::conv::json_to_value;
use crate::public_json::{decode_public_json, encode_public_json, UnknownFields};
use crate::value::{BoxedValue, Value};
use corvid_ir::{IrParam, IrType};
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::Value as J;
use std::collections::HashMap;

/// Encode a connector call's arguments, each by its declared type.
pub(super) fn connector_arguments(
    params: &[IrParam],
    values: &[Value],
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Vec<J>, String> {
    params
        .iter()
        .zip(values)
        .map(|(param, value)| {
            encode_public_json(value, &param.ty, types_by_id)
                .map_err(|e| format!("argument `{}` cannot be sent as JSON: {e}", param.name))
        })
        .collect()
}

/// Read a connector call's result, as the runtime returned or replay
/// substituted it, into a value of the operation's return type.
pub(super) fn connector_result(
    json: J,
    return_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, String> {
    match return_ty {
        Type::Result(ok_ty, err_ty) => {
            let J::Object(mut envelope) = json else {
                return Err(format!("expected a `{}` result envelope", declared_name(return_ty, types_by_id)));
            };
            match envelope.get("tag").and_then(J::as_str) {
                Some("ok") => {
                    let body = envelope.remove("ok").unwrap_or(J::Null);
                    let value = decode_body(&body, ok_ty, types_by_id)?;
                    Ok(Value::ResultOk(BoxedValue::new(value)))
                }
                Some("err") => {
                    let err = envelope.remove("err").unwrap_or(J::Null);
                    let value = json_to_value(err, err_ty, types_by_id).map_err(|e| e.to_string())?;
                    Ok(Value::ResultErr(BoxedValue::new(value)))
                }
                _ => Err(format!("expected a `{}` result envelope", declared_name(return_ty, types_by_id))),
            }
        }
        other => decode_body(&json, other, types_by_id),
    }
}

fn decode_body(body: &J, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    decode_public_json(body, ty, types_by_id, UnknownFields::Ignore).map_err(|e| {
        format!("the response does not match `{}` {}", declared_name(ty, types_by_id), e.message())
    })
}

/// A type as the source writes it, with records and enums by name.
fn declared_name(ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> String {
    match ty {
        Type::Struct(id) => types_by_id
            .get(id)
            .map(|decl| decl.name.clone())
            .unwrap_or_else(|| ty.display_name()),
        Type::List(inner) => format!("List<{}>", declared_name(inner, types_by_id)),
        Type::Option(inner) => format!("Option<{}>", declared_name(inner, types_by_id)),
        Type::Result(ok, err) => format!(
            "Result<{}, {}>",
            declared_name(ok, types_by_id),
            declared_name(err, types_by_id)
        ),
        other => other.display_name(),
    }
}
