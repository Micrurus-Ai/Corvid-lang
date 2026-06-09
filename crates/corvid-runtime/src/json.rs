//! Phase 33R5b — runtime support for the executing JSON surface.
//!
//! The executing tools live in `std/json.cor` and dispatch
//! through `Runtime::json_parse_tool` / `json_get_*_tool` /
//! `json_object_new_tool` / `json_object_set_*_tool` /
//! `json_object_finish_tool`. The tools all return JSON
//! envelopes the interpreter converts back to typed values
//! through the standard `json_to_value` path, except:
//!
//! - `json_parse_tool` returns `Value::JsonValue(Arc<...>)`
//!   directly (typed-Value dispatch, like `db_open_tool`'s
//!   `Arc<DbHandleInner>` return) — the `Arc<serde_json::Value>`
//!   IS the payload, no marshalling needed.
//! - `json_object_new_tool` returns
//!   `Value::JsonBuilder(Arc<Mutex<...>>)` similarly.
//! - `json_object_set_*_tool` mutate the inner Mutex and return
//!   the SAME builder Arc.
//!
//! Path security: JSON has none. The surface is a pure
//! parse/access/build path with no I/O, no network egress, no
//! filesystem touch. The structural property the surface DOES
//! carry is `json.parse_safety_no_panic` — malformed input is
//! recoverable via `Result<_, String>`; the runtime never
//! panics.
//!
//! Replay quarantine: JSON parse/build are process-internal,
//! deterministic, and side-effect-free. The replay-substitution
//! upper gate doesn't apply (there's no I/O to record); the
//! tools run identically during live and replay execution.

use crate::errors::RuntimeError;
use serde_json::{Map, Value as JsonValue};
use std::sync::{Arc, Mutex};

/// Phase 33R5b-a — anchor for the parse-safety guarantee.
/// `json_parse_tool` returns `Result<JsonValue, String>` rather
/// than panicking on malformed input. The error message is the
/// serde_json parse-error description so users can route a
/// reasonable diagnostic up to their callers. This is the
/// structural property `json.parse_safety_no_panic`: a Corvid
/// program calling `json_parse(text)` cannot crash the runtime
/// regardless of what bytes are in `text`.
pub const GUARANTEE_ID_JSON_PARSE_SAFETY_NO_PANIC: &str =
    "json.parse_safety_no_panic";

/// Phase 33R5b-a — anchor for the field-type-safety guarantee.
/// `json_get_int(json, field)` against a string-valued field
/// returns `Err("field 'x' is not an Int")` rather than
/// coercing or panicking. Each typed accessor checks the value's
/// JSON kind and returns a structured error when the kind doesn't
/// match the requested type. This is the structural property
/// `json.field_type_safety_at_access_boundary`: typed JSON
/// access is type-safe at the runtime level, even though the
/// JSON itself is dynamically-typed.
pub const GUARANTEE_ID_JSON_FIELD_TYPE_SAFETY: &str =
    "json.field_type_safety_at_access_boundary";

/// Parse a JSON text string into an `Arc<serde_json::Value>`.
/// Returns `Err(message)` on malformed input — never panics.
/// The caller (the runtime's `json_parse_tool` dispatch method)
/// wraps the result in `Result<JsonValue, String>` for the
/// Corvid program.
pub fn parse(text: &str) -> Result<Arc<JsonValue>, String> {
    serde_json::from_str::<JsonValue>(text)
        .map(Arc::new)
        .map_err(|err| format!("malformed JSON: {err}"))
}

/// Look up a top-level field on a JSON object and decode it as
/// an `i64`. Returns `Err` if the JSON value is not an object,
/// the field is missing, or the field's JSON kind isn't `Number`
/// in the integer range. The error messages name the property
/// the caller violated so user code can route diagnostics.
pub fn get_int(value: &JsonValue, field: &str) -> Result<i64, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    raw.as_i64()
        .ok_or_else(|| format!("field `{field}` is not an Int (got {})", json_kind(raw)))
}

/// Same shape as `get_int` for `f64`. Accepts both JSON
/// integers and JSON floats (the typechecker enforces the
/// caller's expectation).
pub fn get_float(value: &JsonValue, field: &str) -> Result<f64, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    raw.as_f64()
        .ok_or_else(|| format!("field `{field}` is not a Float (got {})", json_kind(raw)))
}

/// Same shape for `String`.
pub fn get_string(value: &JsonValue, field: &str) -> Result<String, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    raw.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("field `{field}` is not a String (got {})", json_kind(raw)))
}

/// Same shape for `bool`.
pub fn get_bool(value: &JsonValue, field: &str) -> Result<bool, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    raw.as_bool()
        .ok_or_else(|| format!("field `{field}` is not a Bool (got {})", json_kind(raw)))
}

/// Look up a nested object field. Returns an Arc-cloned subtree
/// for further typed access. Errors when the field is missing or
/// isn't itself a JSON object.
pub fn get_object(value: &JsonValue, field: &str) -> Result<Arc<JsonValue>, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    if !raw.is_object() {
        return Err(format!(
            "field `{field}` is not a JsonValue object (got {})",
            json_kind(raw)
        ));
    }
    Ok(Arc::new(raw.clone()))
}

/// Look up an array field. Returns a `Vec<Arc<JsonValue>>` so
/// each element can be passed back into another typed accessor.
pub fn get_array(value: &JsonValue, field: &str) -> Result<Vec<Arc<JsonValue>>, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("JSON value is not an object, cannot access field `{field}`"))?;
    let raw = obj
        .get(field)
        .ok_or_else(|| format!("field `{field}` is missing"))?;
    let arr = raw
        .as_array()
        .ok_or_else(|| format!("field `{field}` is not an Array (got {})", json_kind(raw)))?;
    Ok(arr.iter().map(|element| Arc::new(element.clone())).collect())
}

/// Construct an empty JSON object builder.
pub fn object_new() -> Arc<Mutex<Map<String, JsonValue>>> {
    Arc::new(Mutex::new(Map::new()))
}

/// Mutate the supplied builder by setting `key` to the supplied
/// integer value. Returns the SAME Arc (clone for the caller's
/// fluent chain) — mutations on either reference are visible to
/// the other.
pub fn object_set_int(
    builder: Arc<Mutex<Map<String, JsonValue>>>,
    key: &str,
    value: i64,
) -> Result<Arc<Mutex<Map<String, JsonValue>>>, RuntimeError> {
    builder
        .lock()
        .map_err(|err| RuntimeError::Other(format!("std.json builder mutex poisoned: {err}")))?
        .insert(key.to_string(), JsonValue::from(value));
    Ok(builder)
}

/// Same shape for floats.
pub fn object_set_float(
    builder: Arc<Mutex<Map<String, JsonValue>>>,
    key: &str,
    value: f64,
) -> Result<Arc<Mutex<Map<String, JsonValue>>>, RuntimeError> {
    builder
        .lock()
        .map_err(|err| RuntimeError::Other(format!("std.json builder mutex poisoned: {err}")))?
        .insert(key.to_string(), JsonValue::from(value));
    Ok(builder)
}

/// Same shape for strings.
pub fn object_set_string(
    builder: Arc<Mutex<Map<String, JsonValue>>>,
    key: &str,
    value: &str,
) -> Result<Arc<Mutex<Map<String, JsonValue>>>, RuntimeError> {
    builder
        .lock()
        .map_err(|err| RuntimeError::Other(format!("std.json builder mutex poisoned: {err}")))?
        .insert(key.to_string(), JsonValue::String(value.to_string()));
    Ok(builder)
}

/// Same shape for bools.
pub fn object_set_bool(
    builder: Arc<Mutex<Map<String, JsonValue>>>,
    key: &str,
    value: bool,
) -> Result<Arc<Mutex<Map<String, JsonValue>>>, RuntimeError> {
    builder
        .lock()
        .map_err(|err| RuntimeError::Other(format!("std.json builder mutex poisoned: {err}")))?
        .insert(key.to_string(), JsonValue::Bool(value));
    Ok(builder)
}

/// Snapshot the builder's current state and serialise to a
/// `String`. The builder remains usable for further set+finish
/// cycles. Errors only if the mutex is poisoned (a panic in
/// another thread holding the lock).
pub fn object_finish(
    builder: &Arc<Mutex<Map<String, JsonValue>>>,
) -> Result<String, RuntimeError> {
    let snapshot = builder
        .lock()
        .map_err(|err| RuntimeError::Other(format!("std.json builder mutex poisoned: {err}")))?
        .clone();
    serde_json::to_string(&JsonValue::Object(snapshot))
        .map_err(|err| RuntimeError::Other(format!("std.json failed to serialise builder: {err}")))
}

/// Render a JSON value's kind name for diagnostic messages.
fn json_kind(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "Null",
        JsonValue::Bool(_) => "Bool",
        JsonValue::Number(_) => "Number",
        JsonValue::String(_) => "String",
        JsonValue::Array(_) => "Array",
        JsonValue::Object(_) => "Object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------- Phase 33R5b-a plumbing tests --------

    /// 33R5b-a — round-trip parse works for a typical object.
    #[test]
    fn parse_round_trips_a_typical_object() {
        let value = parse(r#"{"id": 42, "email": "alice@example.com", "admin": true}"#).expect("parse");
        assert_eq!(get_int(&value, "id"), Ok(42));
        assert_eq!(
            get_string(&value, "email"),
            Ok("alice@example.com".to_string())
        );
        assert_eq!(get_bool(&value, "admin"), Ok(true));
    }

    /// 33R5b-a — **the load-bearing parse-safety property**.
    /// Malformed input returns an error, never panics.
    #[test]
    fn malformed_json_returns_recoverable_error_never_panics() {
        let err = parse(r#"{this is not json"#).expect_err("must reject malformed JSON");
        assert!(
            err.contains("malformed JSON"),
            "diagnostic must name the parse failure; got: {err}"
        );
    }

    /// 33R5b-a — **the load-bearing field-type-safety property**.
    /// Accessing a String field as Int returns Err, NOT a coerced
    /// or panicking result.
    #[test]
    fn typed_accessor_mismatch_returns_recoverable_error() {
        let value = parse(r#"{"email": "alice"}"#).expect("parse");
        let err = get_int(&value, "email").expect_err("must reject String as Int");
        assert!(
            err.contains("not an Int") && err.contains("String"),
            "diagnostic must name the type mismatch; got: {err}"
        );
    }

    /// 33R5b-a — missing field returns Err with the field name.
    #[test]
    fn missing_field_returns_recoverable_error_naming_the_field() {
        let value = parse(r#"{"id": 1}"#).expect("parse");
        let err = get_string(&value, "missing_field").expect_err("must reject missing field");
        assert!(
            err.contains("missing_field") && err.contains("missing"),
            "diagnostic must name the missing field; got: {err}"
        );
    }

    /// 33R5b-a — nested object access returns an Arc-cloned
    /// subtree the caller can pass back into typed accessors.
    #[test]
    fn get_object_returns_subtree_for_further_typed_access() {
        let value =
            parse(r#"{"user": {"id": 7, "email": "x@y"}}"#).expect("parse");
        let user = get_object(&value, "user").expect("get_object");
        assert_eq!(get_int(&user, "id"), Ok(7));
        assert_eq!(get_string(&user, "email"), Ok("x@y".to_string()));
    }

    /// 33R5b-a — builder set+finish round-trip preserves field
    /// values and order.
    #[test]
    fn builder_set_and_finish_preserves_field_values() {
        let builder = object_new();
        let builder = object_set_int(builder, "id", 42).expect("set_int");
        let builder = object_set_string(builder, "email", "alice@example.com").expect("set_string");
        let json = object_finish(&builder).expect("finish");
        let reparsed: JsonValue = serde_json::from_str(&json).expect("reparse");
        assert_eq!(reparsed["id"], 42);
        assert_eq!(reparsed["email"], "alice@example.com");
    }

    /// 33R5b-a — finish leaves the builder usable for further
    /// set+finish cycles (the snapshot semantics — not the
    /// consumed-builder semantics that some JSON libraries use).
    #[test]
    fn builder_finish_is_a_snapshot_not_a_consumer() {
        let builder = object_new();
        let builder = object_set_int(builder, "version", 1).expect("set_int");
        let first = object_finish(&builder).expect("first finish");
        let builder = object_set_int(builder, "version", 2).expect("second set_int");
        let second = object_finish(&builder).expect("second finish");
        assert!(first.contains("\"version\":1"));
        assert!(second.contains("\"version\":2"));
    }
}
