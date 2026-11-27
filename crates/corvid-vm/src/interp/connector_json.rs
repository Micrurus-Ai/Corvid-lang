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
//! An operation returning `Response<Body>` gets the status and response
//! headers too: the runtime returns `{"status", "headers", "body"}` and
//! this builds the `Response` record from `RESPONSE_FIELDS`.
//!
//! The runtime still wraps a response for a `Result`-returning operation
//! as `{"tag":"ok","ok":<body>}` or a mapped status as
//! `{"tag":"err","err":<variant>}`; that envelope is Corvid's own, so it is
//! read here, and only the provider's body goes through the public
//! decoder. The recorded `ToolResult` keeps that shape, so a trace
//! replays through exactly this function.

use crate::conv::json_to_value;
use crate::public_json::{decode_public_json, encode_public_json, UnknownFields};
use crate::value::{BoxedValue, ListValue, StructValue, Value};
use std::sync::Arc;
use corvid_ir::{IrParam, IrType};
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::Value as J;
use std::collections::HashMap;

/// A connector call's arguments in the two forms the runtime takes:
/// what every record of the call holds, and what the request is built
/// from. They differ only for a file sent as the body or inside a
/// multipart body, which is recorded by its name, type, size and SHA-256
/// rather than its content: an upload of a person's document never lands
/// in a trace, and replay matches the same file by its digest.
pub(super) struct ConnectorArguments {
    pub recorded: Vec<J>,
    pub implementation: Vec<J>,
}

impl ConnectorArguments {
    /// The runtime's revealed form, when the two differ.
    pub(super) fn revealed(&self) -> Option<corvid_runtime::RevealedToolArguments> {
        (self.recorded != self.implementation)
            .then(|| corvid_runtime::RevealedToolArguments::new(&self.recorded, self.implementation.clone()))
    }
}

/// Encode a connector call's arguments, each by its declared type.
/// `payload_param` names the parameter sent as a raw or multipart body.
pub(super) fn connector_arguments(
    params: &[IrParam],
    values: &[Value],
    types_by_id: &HashMap<DefId, &IrType>,
    payload_param: Option<&str>,
) -> Result<ConnectorArguments, String> {
    let mut recorded = Vec::with_capacity(values.len());
    let mut implementation = Vec::with_capacity(values.len());
    for (param, value) in params.iter().zip(values) {
        let (record, send) = if payload_param == Some(param.name.as_str()) {
            payload_forms(value, &param.ty, types_by_id)
                .map_err(|e| format!("argument `{}` cannot be sent: {e}", param.name))?
        } else if param.ty == Type::Secret {
            // An operation's `auth: bearer(token)` credential, the only
            // `Secret` the checker lets an operation take: every record of
            // the call holds `<redacted>`, and only the request gets the text.
            let form = |secrets| {
                crate::public_json::encode_tool_argument(value, &param.ty, types_by_id, secrets)
                    .map_err(|e| format!("argument `{}` cannot be sent: {e}", param.name))
            };
            (
                form(crate::public_json::SecretArgument::Recorded)?,
                form(crate::public_json::SecretArgument::Revealed)?,
            )
        } else {
            let json = encode_public_json(value, &param.ty, types_by_id)
                .map_err(|e| format!("argument `{}` cannot be sent as JSON: {e}", param.name))?;
            (json.clone(), json)
        };
        recorded.push(record);
        implementation.push(send);
    }
    Ok(ConnectorArguments { recorded, implementation })
}

/// The recorded and sent forms of a raw or multipart body value.
fn payload_forms(value: &Value, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<(J, J), String> {
    match (ty, value) {
        (Type::Upload(_), Value::Struct(upload)) => {
            let text = |field: &str| match upload.get_field(field) {
                Some(Value::String(s)) => s.to_string(),
                _ => String::new(),
            };
            let Some(Value::Bytes(bytes)) = upload.get_field("bytes") else {
                return Err("the upload has no bytes".to_string());
            };
            Ok(file_forms(Some(text("filename")), Some(text("content_type")), bytes.as_slice()))
        }
        (Type::Bytes, Value::Bytes(bytes)) => Ok(file_forms(None, None, bytes.as_slice())),
        (Type::Struct(_), Value::Struct(record)) => {
            let decl = match ty {
                Type::Struct(id) => types_by_id.get(id).copied(),
                _ => None,
            }
            .ok_or_else(|| "the multipart record's type is missing".to_string())?;
            let mut recorded = serde_json::Map::new();
            let mut sent = serde_json::Map::new();
            for field in &decl.fields {
                let field_value = record.get_field(&field.name).unwrap_or(Value::OptionNone);
                let (r, s) = multipart_field_forms(&field_value, &field.ty, types_by_id)?;
                recorded.insert(field.name.clone(), r);
                sent.insert(field.name.clone(), s);
            }
            Ok((J::Object(recorded), J::Object(sent)))
        }
        _ => Err(format!("a `{}` is not a file or a multipart record", ty.display_name())),
    }
}

fn multipart_field_forms(value: &Value, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<(J, J), String> {
    match (ty, value) {
        (Type::Upload(_) | Type::Bytes, _) => payload_forms(value, ty, types_by_id),
        (Type::Option(_), Value::OptionNone) => Ok((J::Null, J::Null)),
        (Type::Option(inner), Value::OptionSome(item)) => multipart_field_forms(&item.get(), inner, types_by_id),
        (Type::List(inner), Value::List(items)) => {
            let mut recorded = Vec::new();
            let mut sent = Vec::new();
            for item in items.iter_cloned() {
                let (r, s) = multipart_field_forms(&item, inner, types_by_id)?;
                recorded.push(r);
                sent.push(s);
            }
            Ok((J::Array(recorded), J::Array(sent)))
        }
        _ => {
            let json = encode_public_json(value, ty, types_by_id)?;
            Ok((json.clone(), json))
        }
    }
}

/// A file's recorded form (name, type, size, digest) and its sent form
/// (name, type, base64url bytes). A `Bytes` value has no name or type.
fn file_forms(filename: Option<String>, content_type: Option<String>, bytes: &[u8]) -> (J, J) {
    use sha2::Digest;
    let digest = corvid_text::encoding::encode_hex_bytes(&sha2::Sha256::digest(bytes));
    let mut recorded = serde_json::Map::new();
    let mut sent = serde_json::Map::new();
    if let Some(filename) = filename {
        recorded.insert("filename".into(), J::String(filename.clone()));
        sent.insert("filename".into(), J::String(filename));
    }
    if let Some(content_type) = content_type {
        recorded.insert("content_type".into(), J::String(content_type.clone()));
        sent.insert("content_type".into(), J::String(content_type));
    }
    recorded.insert("size".into(), J::from(bytes.len() as u64));
    recorded.insert("sha256".into(), J::String(digest));
    sent.insert("bytes".into(), J::String(corvid_text::encoding::encode_base64_url_bytes(bytes)));
    (J::Object(recorded), J::Object(sent))
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
                    let value = decode_success(body, ok_ty, types_by_id)?;
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
        other => decode_success(json, other, types_by_id),
    }
}

/// A successful answer: a `Response` record, or the body alone.
fn decode_success(json: J, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    let Type::Response(body_ty) = ty else {
        return decode_body(&json, ty, types_by_id);
    };
    let J::Object(mut parts) = json else {
        return Err(format!("expected a `{}` with its status and headers", declared_name(ty, types_by_id)));
    };
    let status = parts
        .get("status")
        .and_then(J::as_i64)
        .ok_or_else(|| "a response without a status".to_string())?;
    let headers = match parts.remove("headers") {
        Some(J::Array(pairs)) => pairs
            .into_iter()
            .filter_map(|pair| match pair {
                J::Array(pair) => match pair.as_slice() {
                    [J::String(name), J::String(value)] => Some((name.clone(), value.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let body = decode_body(&parts.remove("body").unwrap_or(J::Null), body_ty, types_by_id)?;
    Ok(response_value(status, headers, body))
}

/// A `Response` record, its fields in `RESPONSE_FIELDS` order.
pub(super) fn response_value(status: i64, headers: Vec<(String, String)>, body: Value) -> Value {
    let pairs: Vec<Value> = headers
        .into_iter()
        .map(|(name, value)| {
            Value::List(ListValue::new(vec![
                Value::String(Arc::from(name.to_ascii_lowercase().as_str())),
                Value::String(Arc::from(value.as_str())),
            ]))
        })
        .collect();
    let fields = corvid_types::RESPONSE_FIELDS
        .iter()
        .map(|name| name.to_string())
        .zip([Value::Int(status), Value::List(ListValue::new(pairs)), body])
        .collect::<Vec<_>>();
    Value::Struct(StructValue::new(corvid_types::RESPONSE_TYPE_ID, "Response", fields))
}

/// In mock mode an operation returning `Response<Body>` declares its
/// mock as the body (inside `Ok` when the operation returns a
/// `Result`); it becomes a status-200 response with no headers. A
/// `Stream<T>` operation declares the `List<T>` of its events.
pub(super) fn mock_response(value: Value, return_ty: &Type) -> Value {
    match (return_ty, value) {
        (Type::Response(_), body) => response_value(200, Vec::new(), body),
        // A streamed operation's mock lists its events; mock mode streams
        // them in order.
        (Type::Stream(_), Value::List(events)) => {
            let (sender, stream) = crate::value::StreamValue::channel(
                super::effect_compose::default_stream_backpressure(),
            );
            let events = events.iter_cloned();
            stream.spawn_producer(async move {
                for event in events {
                    if !sender.send(Ok(event)).await {
                        break;
                    }
                }
            });
            Value::Stream(stream)
        }
        (Type::Result(ok, _), Value::ResultOk(body)) if matches!(**ok, Type::Response(_)) => {
            Value::ResultOk(BoxedValue::new(response_value(200, Vec::new(), body.get())))
        }
        (_, value) => value,
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

/// `Err(Variant)` or `Err(Variant(reason))` for an `on failure` operation
/// whose answer could not be read, built from the runtime's own failure
/// envelope so every failure path produces the same value.
pub(super) fn failure_value(
    variant: &str,
    reason: &str,
    return_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, String> {
    // A streamed operation answers a failure that happens before it
    // opens (an open circuit breaker) with a stream of one final `Err`
    // event, as the runtime answers a failure to open (decision 7).
    if let Type::Stream(item) = return_ty {
        let value = failure_value(variant, reason, item, types_by_id)?;
        let (sender, stream) = crate::value::StreamValue::channel(super::effect_compose::default_stream_backpressure());
        stream.spawn_producer(async move {
            let _ = sender.send(Ok(value)).await;
        });
        return Ok(Value::Stream(stream));
    }
    let takes_reason = match return_ty {
        Type::Result(_, err) => match err.as_ref() {
            Type::Struct(id) => types_by_id
                .get(id)
                .and_then(|t| t.variants.iter().find(|v| v.name == variant))
                .is_some_and(|v| !v.fields.is_empty()),
            _ => false,
        },
        _ => false,
    };
    let envelope = corvid_runtime::connectors::failure_result(variant, takes_reason, reason);
    connector_result(envelope, return_ty, types_by_id)
}
