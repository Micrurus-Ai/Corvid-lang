//! Marshalling between Corvid `Value` and `serde_json::Value`.
//!
//! Tools and LLM adapters cross the runtime boundary as JSON. This module
//! is the only place that translation lives. The interpreter calls
//! `value_to_json` to prepare arguments for a tool/LLM call, and
//! `json_to_value` to build a `Value` from the JSON the runtime returned.
//!
//! The inbound direction (JSON → Value) needs the *expected* `Type` so
//! struct results can recover their `type_id` and `type_name`. The
//! interpreter passes the called tool's / prompt's declared return type.

use crate::value::{
    BoxedValue, ListValue, PartialFieldValue, PartialValue, ResumeTokenValue, StreamChunk,
    StructValue, Value,
};
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use std::collections::HashMap;
use std::sync::Arc;

/// Convert a `Value` to a `serde_json::Value`. Lossless for primitives;
/// structs become JSON objects (the type name is dropped — the receiving
/// tool doesn't need it).
pub fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(n) => serde_json::Value::from(*n),
        Value::Float(f) => serde_json::Value::from(*f),
        Value::String(s) => serde_json::Value::String(s.to_string()),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Nothing => serde_json::Value::Null,
        Value::Struct(s) => {
            let mut obj = serde_json::Map::new();
            s.with_fields(|fields| {
                for (k, v) in fields {
                    obj.insert(k.clone(), value_to_json(v));
                }
            });
            serde_json::Value::Object(obj)
        }
        Value::List(items) => {
            serde_json::Value::Array(items.iter_cloned().iter().map(value_to_json).collect())
        }
        Value::Enum(e) => serde_json::json!({
            "tag": "variant",
            "type": e.type_name(),
            "variant": e.variant_name(),
            "fields": e.fields_cloned().iter().map(value_to_json).collect::<Vec<_>>(),
        }),
        Value::Map(m) => {
            let entries = m.entries_cloned();
            if entries.iter().all(|(k, _)| matches!(k, Value::String(_))) {
                let mut obj = serde_json::Map::new();
                for (k, v) in &entries {
                    if let Value::String(s) = k {
                        obj.insert(s.to_string(), value_to_json(v));
                    }
                }
                serde_json::Value::Object(obj)
            } else {
                // Non-String keys can't be a JSON object; render as an
                // array of [key, value] pairs.
                serde_json::Value::Array(
                    entries
                        .iter()
                        .map(|(k, v)| {
                            serde_json::Value::Array(vec![value_to_json(k), value_to_json(v)])
                        })
                        .collect(),
                )
            }
        }
        Value::Weak(w) => match w.upgrade() {
            Some(value) => serde_json::json!({ "tag": "weak", "value": value_to_json(&value) }),
            None => serde_json::json!({ "tag": "weak", "value": serde_json::Value::Null }),
        },
        Value::ResultOk(v) => serde_json::json!({ "tag": "ok", "ok": value_to_json(&v.get()) }),
        Value::ResultErr(v) => serde_json::json!({ "tag": "err", "err": value_to_json(&v.get()) }),
        Value::OptionSome(v) => serde_json::json!({ "tag": "some", "value": value_to_json(&v.get()) }),
        Value::OptionNone => serde_json::json!({ "tag": "none" }),
        Value::Grounded(g) => {
            let inner = value_to_json(&g.inner.get());
            let sources: Vec<serde_json::Value> = g.provenance.entries.iter().map(|e| {
                serde_json::json!({
                    "kind": e.kind.label(),
                    "name": e.name,
                    "timestamp_ms": e.timestamp_ms,
                })
            }).collect();
            serde_json::json!({ "tag": "grounded", "value": inner, "sources": sources })
        }
        Value::Partial(p) => {
            let mut fields = serde_json::Map::new();
            p.with_fields(|partial_fields| {
                for (name, field) in partial_fields {
                    let value = match field {
                        PartialFieldValue::Complete(value) => {
                            serde_json::json!({ "tag": "complete", "value": value_to_json(value) })
                        }
                        PartialFieldValue::Streaming => serde_json::json!({ "tag": "streaming" }),
                    };
                    fields.insert(name.clone(), value);
                }
            });
            serde_json::json!({ "tag": "partial", "type": p.type_name(), "fields": fields })
        }
        Value::ResumeToken(token) => serde_json::json!({
            "tag": "resume_token",
            "prompt": token.prompt_name,
            "args": token.args.iter().map(value_to_json).collect::<Vec<_>>(),
            "delivered": token.delivered.iter().map(|chunk| {
                serde_json::json!({
                    "value": value_to_json(&chunk.value),
                    "cost": chunk.cost,
                    "confidence": chunk.confidence,
                    "tokens": chunk.tokens,
                })
            }).collect::<Vec<_>>(),
            "provider_session": token.provider_session,
        }),
        Value::Stream(stream) => serde_json::json!({
            "tag": "stream",
            "backpressure": stream.backpressure().label()
        }),
        // Closures (slice 45j) are opaque: JSON cannot carry code +
        // captured environment, and tools must never receive one.
        // The sentinel exists purely for trace-debug visibility;
        // `json_to_value` has no inverse for this shape.
        Value::Closure(c) => serde_json::json!({
            "tag": "closure_opaque_sentinel",
            "arity": c.arity(),
        }),
        // Phase 33S3a — `Value::DbHandle` cannot be represented as
        // JSON in a way that survives `json_to_value`'s inverse
        // round-trip: the underlying `Arc<DbHandleInner>` carries
        // pointer identity into the runtime's slotmap, and a JSON
        // value cannot carry that pointer. We emit an opaque
        // sentinel here PURELY for trace-debug visibility — it
        // lets traces show "a DbHandle was returned" with the
        // diagnostic path — but `json_to_value` rejects this shape
        // when it sees `expected: Type::DbHandle` so a marshalled
        // JSON DbHandle can never be turned back into a runtime
        // handle. The typed-Value dispatch path is the ONLY way to
        // produce a `Value::DbHandle`; that's what makes the
        // opacity guarantee structural rather than documentary.
        Value::DbHandle(inner) => serde_json::json!({
            "tag": "db_handle_opaque_sentinel",
            "handle_id": inner.handle_id,
            "path": inner.path,
        }),
        // Phase 33R5b-a — `Value::JsonValue`'s payload IS the JSON
        // shape; the round trip is lossless. Unlike DbHandle there
        // is no opacity gate because there is no underlying
        // registry the value indexes into. Cloning the inner
        // serde_json::Value is structurally necessary because the
        // returned `serde_json::Value` is owned by the caller.
        Value::JsonValue(value) => (**value).clone(),
        // Phase 33R5b-a — `Value::JsonBuilder` is a process-
        // internal mutation surface. Emit a snapshot of the
        // current state for trace-debug visibility. There is no
        // `json_to_value` round-trip for JsonBuilder (the type
        // is constructed only by `json_object_new`).
        Value::JsonBuilder(builder) => match builder.lock() {
            Ok(map) => serde_json::Value::Object((*map).clone()),
            Err(_) => serde_json::json!({
                "tag": "json_builder_poisoned",
            }),
        },
    }
}

/// Convert a `serde_json::Value` to a `Value`, guided by the `expected`
/// type. The type table `types_by_id` is consulted when the expected
/// type is a struct so the rebuilt `StructValue` carries the right
/// `type_id` and `type_name`.
pub fn json_to_value(
    json: serde_json::Value,
    expected: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, ConvError> {
    use serde_json::Value as J;
    match (expected, json) {
        (Type::Int, J::Number(n)) => n
            .as_i64()
            .map(Value::Int)
            .ok_or_else(|| ConvError::TypeMismatch {
                expected: "Int".into(),
                got: "non-integer number".into(),
            }),
        // Map<String, V> (45g) decodes from a JSON object; other key
        // types decode from an array of [key, value] pairs (the same
        // shapes value_to_json emits).
        (Type::Map(key_ty, val_ty), J::Object(obj)) if matches!(**key_ty, Type::String) => {
            let mut entries = Vec::with_capacity(obj.len());
            for (k, v) in obj {
                entries.push((
                    Value::String(std::sync::Arc::from(k.as_str())),
                    json_to_value(v.clone(), val_ty, types_by_id)?,
                ));
            }
            Ok(Value::Map(crate::value::MapValue::new(entries)))
        }
        (Type::Map(key_ty, val_ty), J::Array(pairs)) => {
            let mut entries = Vec::with_capacity(pairs.len());
            for pair in pairs {
                let J::Array(kv) = pair else {
                    return Err(ConvError::TypeMismatch {
                        expected: "a [key, value] pair".into(),
                        got: "a non-array element".into(),
                    });
                };
                if kv.len() != 2 {
                    return Err(ConvError::TypeMismatch {
                        expected: "a [key, value] pair".into(),
                        got: format!("an array of length {}", kv.len()),
                    });
                }
                entries.push((
                    json_to_value(kv[0].clone(), key_ty, types_by_id)?,
                    json_to_value(kv[1].clone(), val_ty, types_by_id)?,
                ));
            }
            Ok(Value::Map(crate::value::MapValue::new(entries)))
        }
        // Float absorbs both JSON floats and JSON integers (LLMs often
        // emit `1` where a float field is declared).
        (Type::Float, J::Number(n)) => n
            .as_f64()
            .map(Value::Float)
            .ok_or_else(|| ConvError::TypeMismatch {
                expected: "Float".into(),
                got: "non-float number".into(),
            }),
        (Type::String, J::String(s)) => Ok(Value::String(Arc::from(s))),
        (Type::Bool, J::Bool(b)) => Ok(Value::Bool(b)),
        (Type::Nothing, J::Null) => Ok(Value::Nothing),
        // Some tools/LLMs return `null` for any "absent" value. Honour it
        // for `Nothing` returns; reject elsewhere.
        (_, J::Null) => Err(ConvError::TypeMismatch {
            expected: type_label(expected),
            got: "null".into(),
        }),
        (Type::List(elem_ty), J::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(json_to_value(item, elem_ty, types_by_id)?);
            }
            Ok(Value::List(ListValue::new(out)))
        }
        (Type::Stream(_), got) => Err(ConvError::TypeMismatch {
            expected: "Stream".into(),
            got: json_kind(&got).into(),
        }),
        (Type::Partial(inner_ty), J::Object(map)) => partial_from_json(map, inner_ty, types_by_id),
        (Type::ResumeToken(inner_ty), J::Object(map)) => {
            resume_token_from_json(map, inner_ty, types_by_id)
        }
        (Type::Option(inner_ty), J::Object(map)) => match map.get("tag").and_then(|v| v.as_str()) {
            Some("some") => {
                let raw = map.get("value").cloned().ok_or_else(|| ConvError::TypeMismatch {
                    expected: "Option::Some payload".into(),
                    got: "missing `value` field".into(),
                })?;
                Ok(Value::OptionSome(BoxedValue::new(json_to_value(raw, inner_ty, types_by_id)?)))
            }
            Some("none") => Ok(Value::OptionNone),
            _ => Err(ConvError::TypeMismatch {
                expected: type_label(expected),
                got: "object".into(),
            }),
        },
        (Type::Result(ok_ty, err_ty), J::Object(map)) => {
            match map.get("tag").and_then(|v| v.as_str()) {
                Some("ok") => {
                    let raw = map.get("ok").cloned().ok_or_else(|| ConvError::TypeMismatch {
                        expected: "Result::Ok payload".into(),
                        got: "missing `ok` field".into(),
                    })?;
                    Ok(Value::ResultOk(BoxedValue::new(json_to_value(raw, ok_ty, types_by_id)?)))
                }
                Some("err") => {
                    let raw = map.get("err").cloned().ok_or_else(|| ConvError::TypeMismatch {
                        expected: "Result::Err payload".into(),
                        got: "missing `err` field".into(),
                    })?;
                    Ok(Value::ResultErr(BoxedValue::new(json_to_value(raw, err_ty, types_by_id)?)))
                }
                _ => Err(ConvError::TypeMismatch {
                    expected: type_label(expected),
                    got: "object".into(),
                }),
            }
        }
        (Type::Struct(def_id), J::Object(map)) => {
            let ir_type = types_by_id
                .get(def_id)
                .copied()
                .ok_or(ConvError::UnknownStructType(*def_id))?;
            let mut fields = HashMap::new();
            for field in &ir_type.fields {
                let raw = map
                    .get(&field.name)
                    .cloned()
                    .ok_or_else(|| ConvError::MissingField {
                        struct_name: ir_type.name.clone(),
                        field: field.name.clone(),
                    })?;
                let v = json_to_value(raw, &field.ty, types_by_id)?;
                check_field_refinement(&ir_type.name, field, &v)?;
                fields.insert(field.name.clone(), v);
            }
            Ok(Value::Struct(StructValue::new(
                ir_type.id,
                ir_type.name.clone(),
                fields,
            )))
        }
        // `Unknown` accepts any JSON, lossy. Used as a graceful fallback.
        (Type::Unknown, json) => Ok(json_to_value_loose(json)),
        // Phase 33S3a — the opacity guarantee. A JSON value cannot
        // reconstruct a `Value::DbHandle` because the underlying
        // `Arc<DbHandleInner>` carries pointer identity into the
        // runtime's slotmap. Even when the JSON shape matches the
        // `value_to_json` sentinel (which exists for trace-debug
        // visibility), this path REFUSES to mint a handle from
        // JSON — that's what makes "you cannot fabricate a SQLite
        // connection in user code" a load-bearing language property
        // rather than a documentation claim. The only path to a
        // valid `Value::DbHandle` is the typed-Value dispatch
        // surface in `Runtime::call_tool` for `db_open` (33S3b).
        (Type::DbHandle, _got) => Err(ConvError::TypeMismatch {
            expected: "DbHandle (only producible by the runtime's db_open dispatch path)".into(),
            got: "JSON payload — opaque handles cannot be reconstructed from JSON".into(),
        }),
        // Phase 33R5b-a — `Type::JsonValue`'s payload IS the JSON
        // shape, so the conversion is the natural identity:
        // wrap the input in an Arc. Unlike DbHandle there is NO
        // opacity gate here — the JsonValue type is a recoverable
        // wrapper around `serde_json::Value`, not a registry
        // index. This is what lets `json_parse` return a
        // `Result<JsonValue, String>` whose Ok payload round-
        // trips cleanly.
        (Type::JsonValue, json) => Ok(Value::JsonValue(Arc::new(json))),
        // Phase 33R5b-a — `Type::JsonBuilder` cannot be
        // reconstructed from JSON either; the type is constructed
        // ONLY by `json_object_new`'s typed-Value dispatch path.
        // Reject the conversion if anyone tries.
        (Type::JsonBuilder, _got) => Err(ConvError::TypeMismatch {
            expected: "JsonBuilder (only producible by the runtime's json_object_new dispatch path)".into(),
            got: "JSON payload — builders cannot be reconstructed from JSON".into(),
        }),
        (expected, got) => Err(ConvError::TypeMismatch {
            expected: type_label(expected),
            got: json_kind(&got).into(),
        }),
    }
}

/// Best-effort JSON → Value conversion when the expected type is unknown.
/// Used as a fallback path; never produces structs (no type_id available).
fn json_to_value_loose(json: serde_json::Value) -> Value {
    use serde_json::Value as J;
    match json {
        J::Null => Value::Nothing,
        J::Bool(b) => Value::Bool(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Nothing
            }
        }
        J::String(s) => Value::String(Arc::from(s)),
        J::Array(items) => Value::List(ListValue::new(items.into_iter().map(json_to_value_loose).collect::<Vec<_>>())),
        J::Object(_) => {
            // Without a type, we can't rebuild a Struct. Drop to Nothing
            // and let the interpreter surface a clean error if the value
            // is used.
            Value::Nothing
        }
    }
}

fn type_label(t: &Type) -> String {
    match t {
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::String => "String".into(),
        Type::Bool => "Bool".into(),
        Type::Nothing => "Nothing".into(),
        Type::Map(_, _) => "Map".into(),
        Type::Struct(_) => "struct".into(),
        Type::ImportedStruct(imported) => imported.name.clone(),
        Type::List(elem) => format!("List<{}>", type_label(elem)),
        Type::Stream(inner) => format!("Stream<{}>", type_label(inner)),
        Type::Result(ok, err) => format!("Result<{}, {}>", type_label(ok), type_label(err)),
        Type::Option(inner) => format!("Option<{}>", type_label(inner)),
        Type::Weak(inner, effects) => {
            if effects.is_any() {
                format!("Weak<{}>", type_label(inner))
            } else {
                let effect_names: Vec<&'static str> = effects
                    .effects()
                    .into_iter()
                    .map(|effect| match effect {
                        corvid_ast::WeakEffect::ToolCall => "tool_call",
                        corvid_ast::WeakEffect::Llm => "llm",
                        corvid_ast::WeakEffect::Approve => "approve",
                        corvid_ast::WeakEffect::Human => "human",
                    })
                    .collect();
                format!("Weak<{}, {{{}}}>", type_label(inner), effect_names.join(", "))
            }
        }
        Type::Grounded(inner) => format!("Grounded<{}>", type_label(inner)),
        Type::Partial(inner) => format!("Partial<{}>", type_label(inner)),
        Type::ResumeToken(inner) => format!("ResumeToken<{}>", type_label(inner)),
        Type::TraceId => "TraceId".into(),
        Type::DbHandle => "DbHandle".into(),
        Type::JsonValue => "JsonValue".into(),
        Type::JsonBuilder => "JsonBuilder".into(),
        Type::RouteParams(_) => "route path params".into(),
        Type::Function { .. } => "function".into(),
        Type::Unknown => "<unknown>".into(),
    }
}

fn resume_token_from_json(
    map: serde_json::Map<String, serde_json::Value>,
    inner_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, ConvError> {
    if map.get("tag").and_then(|v| v.as_str()) != Some("resume_token") {
        return Err(ConvError::TypeMismatch {
            expected: "resume_token".into(),
            got: json_kind(&serde_json::Value::Object(map)).into(),
        });
    }
    let prompt_name = map
        .get("prompt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ConvError::TypeMismatch {
            expected: "resume token prompt".into(),
            got: "missing `prompt` field".into(),
        })?
        .to_string();
    let args = map
        .get("args")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .cloned()
                .map(|raw| json_to_value(raw, &Type::Unknown, types_by_id))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let delivered = map
        .get("delivered")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .map(|raw| resume_chunk_from_json(raw, inner_ty, types_by_id))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let provider_session = map
        .get("provider_session")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(Value::ResumeToken(ResumeTokenValue {
        prompt_name,
        args,
        delivered,
        provider_session,
    }))
}

fn resume_chunk_from_json(
    raw: &serde_json::Value,
    inner_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<StreamChunk, ConvError> {
    let serde_json::Value::Object(map) = raw else {
        return Err(ConvError::TypeMismatch {
            expected: "resume token delivered chunk".into(),
            got: json_kind(raw).into(),
        });
    };
    let value_raw = map.get("value").cloned().ok_or_else(|| ConvError::TypeMismatch {
        expected: "resume token chunk value".into(),
        got: "missing `value` field".into(),
    })?;
    let value = json_to_value(value_raw, inner_ty, types_by_id)?;
    Ok(StreamChunk {
        value,
        cost: map.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0),
        confidence: map
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0),
        tokens: map.get("tokens").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

fn partial_from_json(
    map: serde_json::Map<String, serde_json::Value>,
    inner_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<Value, ConvError> {
    let (type_id, type_name, fields) = match inner_ty {
        Type::Struct(def_id) => {
            let ir_type = types_by_id
                .get(def_id)
                .copied()
                .ok_or(ConvError::UnknownStructType(*def_id))?;
            (ir_type.id, ir_type.name.clone(), ir_type.fields.as_slice())
        }
        other => {
            return Err(ConvError::TypeMismatch {
                expected: "Partial<struct>".into(),
                got: type_label(other),
            })
        }
    };

    let field_map = if map.get("tag").and_then(|v| v.as_str()) == Some("partial") {
        match map.get("fields") {
            Some(serde_json::Value::Object(fields)) => fields,
            _ => {
                return Err(ConvError::TypeMismatch {
                    expected: "Partial fields object".into(),
                    got: "missing `fields` field".into(),
                })
            }
        }
    } else {
        &map
    };

    let mut out = HashMap::new();
    for field in fields {
        let Some(raw) = field_map.get(&field.name) else {
            out.insert(field.name.clone(), PartialFieldValue::Streaming);
            continue;
        };
        let value = partial_field_from_json(raw.clone(), &field.ty, types_by_id)?;
        out.insert(field.name.clone(), value);
    }
    Ok(Value::Partial(PartialValue::new(type_id, type_name, out)))
}

fn partial_field_from_json(
    raw: serde_json::Value,
    field_ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
) -> Result<PartialFieldValue, ConvError> {
    match raw {
        serde_json::Value::Object(map) => match map.get("tag").and_then(|v| v.as_str()) {
            Some("streaming") => Ok(PartialFieldValue::Streaming),
            Some("complete") => {
                let value = map
                    .get("value")
                    .cloned()
                    .ok_or_else(|| ConvError::TypeMismatch {
                        expected: "Partial complete value".into(),
                        got: "missing `value` field".into(),
                    })?;
                Ok(PartialFieldValue::Complete(json_to_value(
                    value,
                    field_ty,
                    types_by_id,
                )?))
            }
            _ => Ok(PartialFieldValue::Complete(json_to_value(
                serde_json::Value::Object(map),
                field_ty,
                types_by_id,
            )?)),
        },
        other => Ok(PartialFieldValue::Complete(json_to_value(
            other,
            field_ty,
            types_by_id,
        )?)),
    }
}

fn json_kind(j: &serde_json::Value) -> &'static str {
    match j {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Slice 50j — enforce a field's declared value refinement after
/// its structural decode. Violations render actionably (field,
/// value, refinement) because this exact string is what `with
/// repair N` feeds back to the model.
fn check_field_refinement(
    struct_name: &str,
    field: &corvid_ir::IrField,
    value: &Value,
) -> Result<(), ConvError> {
    let Some(refinement) = &field.refinement else {
        return Ok(());
    };
    let violated = match (refinement, value) {
        (corvid_ast::Refinement::Between { min, max }, Value::Int(n)) => n < min || n > max,
        (corvid_ast::Refinement::LenBetween { min, max }, Value::String(s)) => {
            let len = s.chars().count() as u64;
            len < *min || len > *max
        }
        // Form/type mismatches are rejected at typecheck; anything
        // else reaching here decodes without a value check.
        _ => false,
    };
    if violated {
        return Err(ConvError::RefinementViolated {
            struct_name: struct_name.to_string(),
            field: field.name.clone(),
            refinement: refinement.describe(),
            got: value_to_json(value).to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub enum ConvError {
    TypeMismatch { expected: String, got: String },
    MissingField { struct_name: String, field: String },
    UnknownStructType(DefId),
    /// Slice 50j — a structurally valid field value violated its
    /// declared refinement. The message names the field, the value,
    /// and the refinement so the structured-output repair loop can
    /// feed the model an actionable correction.
    RefinementViolated {
        struct_name: String,
        field: String,
        refinement: String,
        got: String,
    },
}

impl std::fmt::Display for ConvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeMismatch { expected, got } => {
                write!(f, "expected `{expected}`, got `{got}`")
            }
            Self::MissingField { struct_name, field } => {
                write!(f, "field `{field}` missing on `{struct_name}`")
            }
            Self::RefinementViolated {
                struct_name,
                field,
                refinement,
                got,
            } => {
                write!(
                    f,
                    "field `{field}` on `{struct_name}`: value {got} violates the declared refinement `{refinement}`"
                )
            }
            Self::UnknownStructType(id) => {
                write!(f, "no IR type registered for DefId({})", id.0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn refinement_violation_rejects_decode_with_actionable_message() {
        use corvid_ir::{IrField, IrType};
        let ir_type = IrType {
            id: corvid_resolve::DefId(1),
            name: "Person".into(),
            fields: vec![
                IrField {
                    name: "age".into(),
                    ty: Type::Int,
                    refinement: Some(corvid_ast::Refinement::Between { min: 0, max: 150 }),
                    span: corvid_ast::Span::new(0, 0),
                },
            ],
            variants: Vec::new(),
            span: corvid_ast::Span::new(0, 0),
        };
        let mut types_by_id = std::collections::HashMap::new();
        types_by_id.insert(ir_type.id, &ir_type);

        let ok = json_to_value(
            serde_json::json!({"age": 42}),
            &Type::Struct(ir_type.id),
            &types_by_id,
        );
        assert!(ok.is_ok(), "in-range value must decode: {ok:?}");

        let err = json_to_value(
            serde_json::json!({"age": 200}),
            &Type::Struct(ir_type.id),
            &types_by_id,
        )
        .expect_err("out-of-range value must refuse decode");
        let message = err.to_string();
        assert!(
            message.contains("age") && message.contains("between(0, 150)") && message.contains("200"),
            "the violation must name field, refinement, and value (it feeds the repair loop): {message}"
        );
    }


    #[test]
    fn primitives_roundtrip() {
        let cases = [
            (Value::Int(42), json!(42)),
            (Value::Float(1.5), json!(1.5)),
            (Value::String(Arc::from("hi")), json!("hi")),
            (Value::Bool(true), json!(true)),
            (Value::Nothing, json!(null)),
        ];
        let empty: HashMap<DefId, &IrType> = HashMap::new();
        for (v, j) in cases {
            assert_eq!(value_to_json(&v), j.clone());
            let typ = match &v {
                Value::Int(_) => Type::Int,
                Value::Float(_) => Type::Float,
                Value::String(_) => Type::String,
                Value::Bool(_) => Type::Bool,
                Value::Nothing => Type::Nothing,
                _ => unreachable!(),
            };
            assert_eq!(json_to_value(j, &typ, &empty).unwrap(), v);
        }
    }

    #[test]
    fn list_roundtrips() {
        let v = Value::List(ListValue::new(vec![Value::Int(1), Value::Int(2)]));
        let j = value_to_json(&v);
        assert_eq!(j, json!([1, 2]));
        let empty: HashMap<DefId, &IrType> = HashMap::new();
        let back = json_to_value(j, &Type::List(Box::new(Type::Int)), &empty).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn struct_rebuilds_from_json() {
        // Build a fake IrType for `Decision { should_refund: Bool }`.
        let id = DefId(7);
        let ir_type = IrType {
            variants: Vec::new(),
            id,
            name: "Decision".into(),
            fields: vec![corvid_ir::IrField {
                name: "should_refund".into(),
                ty: Type::Bool,
                refinement: None,
                span: corvid_ast::Span::new(0, 0),
            }],
            span: corvid_ast::Span::new(0, 0),
        };
        let mut by_id = HashMap::new();
        by_id.insert(id, &ir_type);

        let json = json!({"should_refund": true});
        let v = json_to_value(json, &Type::Struct(id), &by_id).unwrap();
        match v {
            Value::Struct(s) => {
                assert_eq!(s.type_name(), "Decision");
                assert_eq!(s.type_id(), id);
                assert_eq!(s.get_field("should_refund").unwrap(), Value::Bool(true));
            }
            other => panic!("expected struct, got {other:?}"),
        }
    }

    #[test]
    fn missing_field_errors() {
        let id = DefId(1);
        let ir_type = IrType {
            variants: Vec::new(),
            id,
            name: "X".into(),
            fields: vec![corvid_ir::IrField {
                name: "needed".into(),
                ty: Type::Int,
                refinement: None,
                span: corvid_ast::Span::new(0, 0),
            }],
            span: corvid_ast::Span::new(0, 0),
        };
        let mut by_id = HashMap::new();
        by_id.insert(id, &ir_type);
        let err = json_to_value(json!({}), &Type::Struct(id), &by_id).unwrap_err();
        assert!(matches!(err, ConvError::MissingField { .. }));
    }
}
