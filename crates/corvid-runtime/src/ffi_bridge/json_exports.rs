//! Generic JSON parse and build primitives exposed at the C ABI.
//!
//! These functions are deliberately **type-agnostic**. The runtime
//! does not know about `Type::Struct`, `STRUCT_FIELD_SLOT_BYTES`, or
//! any other language-level concern. Codegen-emitted decoders and
//! encoders pull this surface to extract or assemble JSON objects,
//! and the language-aware logic (which field has which type, where
//! it sits in memory) lives in the codegen crate.
//!
//! Two stores live behind a single global `Mutex`:
//!
//! - `parsed` holds `Arc<serde_json::Value>` keyed by `u64`. Read-only
//!   from the FFI surface — every getter returns a fresh decoded
//!   value or a sentinel.
//! - `builders` holds `serde_json::Map<String, Value>` keyed by `u64`.
//!   Mutable through `corvid_json_object_set_*`. `corvid_json_object_finish`
//!   serialises the map to a `CorvidString` and removes it from the
//!   store.
//!
//! Handles are monotonically increasing `u64` counters starting at 1.
//! `0` is the null-handle sentinel — `corvid_json_release(0)` is a
//! safe no-op, and `corvid_json_parse` returns `0` on malformed JSON
//! so callers have one canonical "decode failed" signal.
//!
//! All field-getter sentinels follow the same rule: present-but-wrong-
//! type returns the type's zero value (`0` / `false` / `0.0` / empty
//! string). Codegen-emitted decoders are expected to call
//! `corvid_json_field_present` before each `_get_field_*` and bail to
//! the bridge's retry loop if either signal indicates failure.

#![allow(unsafe_code)]

use crate::abi::CorvidString;
use crate::ffi_bridge::strings::{borrow_corvid_string, read_corvid_string, string_from_rust};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const NULL_HANDLE: i64 = 0;

/// Builder side stores fields as a `Vec<(String, Value)>` rather than
/// `serde_json::Map`. By default `serde_json::Map` is backed by
/// `BTreeMap`, which sorts fields alphabetically on serialisation.
/// Codegen-emitted struct encoders set fields in source-order
/// matching `IrType.fields`; using a Vec preserves that order without
/// requiring the workspace-wide `preserve_order` serde_json feature
/// flag (which would alter serialisation order for every existing
/// JSON path in the runtime).
type Builder = Vec<(String, Value)>;

struct JsonStore {
    parsed: HashMap<u64, Value>,
    builders: HashMap<u64, Builder>,
    next_handle: u64,
}

impl JsonStore {
    fn new() -> Self {
        Self {
            parsed: HashMap::new(),
            builders: HashMap::new(),
            next_handle: 1,
        }
    }

    fn fresh(&mut self) -> u64 {
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        h
    }
}

fn store() -> &'static Mutex<JsonStore> {
    static STORE: OnceLock<Mutex<JsonStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(JsonStore::new()))
}

// ----------------------------------------------------------------
// Parse side
// ----------------------------------------------------------------

/// Parse a JSON string into the parsed-handle store.
///
/// Returns a fresh handle (`u64` cast to `i64`) on success. Returns
/// `0` on malformed JSON — codegen-emitted decoders short-circuit
/// to "decoder returned 0" which the bridge's retry loop interprets
/// as parse failure and retries with a stronger system prompt.
///
/// The parsed Value lives until `corvid_json_release` is called.
///
/// # Safety
///
/// `text` must be a valid `CorvidString` per the Corvid ABI.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_parse(text: CorvidString) -> i64 {
    let s = unsafe { borrow_corvid_string(&text) };
    let value: Value = match serde_json::from_str(s) {
        Ok(v) => v,
        Err(_) => return NULL_HANDLE,
    };
    let mut s = store().lock().expect("json store mutex poisoned");
    let handle = s.fresh();
    s.parsed.insert(handle, value);
    handle as i64
}

/// Release a parsed JSON handle. Safe to call on `0` (no-op).
#[no_mangle]
pub unsafe extern "C" fn corvid_json_release(handle: i64) {
    if handle == NULL_HANDLE {
        return;
    }
    let mut s = store().lock().expect("json store mutex poisoned");
    s.parsed.remove(&(handle as u64));
}

/// Returns `1` if the parsed JSON value is an object that contains a
/// member named `name`, `0` otherwise.
///
/// Codegen-emitted decoders call this before `corvid_json_get_field_*`
/// to disambiguate "field absent" from "field present with type
/// mismatch" — both of which would otherwise produce the same zero
/// sentinel from the getter.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`. `handle` may be any value
/// (invalid handles short-circuit to `0`).
#[no_mangle]
pub unsafe extern "C" fn corvid_json_field_present(handle: i64, name: CorvidString) -> i32 {
    if handle == NULL_HANDLE {
        return 0;
    }
    let s = store().lock().expect("json store mutex poisoned");
    let Some(value) = s.parsed.get(&(handle as u64)) else {
        return 0;
    };
    let Some(obj) = value.as_object() else {
        return 0;
    };
    let key = unsafe { borrow_corvid_string(&name) };
    if obj.contains_key(key) { 1 } else { 0 }
}

/// Read a named field as an `i64`. Sentinel: `0` if the handle is
/// invalid, the value isn't an object, the field is absent, or the
/// field's value is not an integer.
///
/// Floats round to integers via `as i64` to handle the common case
/// where an LLM emits `42.0` for an integer field; non-numeric values
/// hit the zero sentinel.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_get_field_int(handle: i64, name: CorvidString) -> i64 {
    if handle == NULL_HANDLE {
        return 0;
    }
    let s = store().lock().expect("json store mutex poisoned");
    let Some(value) = s.parsed.get(&(handle as u64)) else {
        return 0;
    };
    let Some(obj) = value.as_object() else { return 0 };
    let key = unsafe { borrow_corvid_string(&name) };
    let Some(field) = obj.get(key) else { return 0 };
    match field {
        Value::Number(n) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        _ => 0,
    }
}

/// Read a named field as a `bool` represented as `i32` (`0` / `1`).
/// Sentinel: `0` if absent / wrong-type.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_get_field_bool(handle: i64, name: CorvidString) -> i32 {
    if handle == NULL_HANDLE {
        return 0;
    }
    let s = store().lock().expect("json store mutex poisoned");
    let Some(value) = s.parsed.get(&(handle as u64)) else {
        return 0;
    };
    let Some(obj) = value.as_object() else { return 0 };
    let key = unsafe { borrow_corvid_string(&name) };
    match obj.get(key).and_then(|v| v.as_bool()) {
        Some(true) => 1,
        _ => 0,
    }
}

/// Read a named field as `f64`. Sentinel: `0.0` if absent / wrong-
/// type. Integers widen to floats so LLM responses with `42` for a
/// float field decode cleanly.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_get_field_float(handle: i64, name: CorvidString) -> f64 {
    if handle == NULL_HANDLE {
        return 0.0;
    }
    let s = store().lock().expect("json store mutex poisoned");
    let Some(value) = s.parsed.get(&(handle as u64)) else {
        return 0.0;
    };
    let Some(obj) = value.as_object() else { return 0.0 };
    let key = unsafe { borrow_corvid_string(&name) };
    obj.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// Read a named field as a fresh `CorvidString`. Returns an empty
/// string sentinel on absent / wrong-type. Caller owns the returned
/// `CorvidString` (refcount 1) and must release it.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_get_field_str(
    handle: i64,
    name: CorvidString,
) -> CorvidString {
    if handle == NULL_HANDLE {
        return string_from_rust(String::new());
    }
    let s = store().lock().expect("json store mutex poisoned");
    let text = (|| -> Option<String> {
        let value = s.parsed.get(&(handle as u64))?;
        let obj = value.as_object()?;
        let key = unsafe { borrow_corvid_string(&name) };
        let field = obj.get(key)?;
        Some(field.as_str()?.to_owned())
    })()
    .unwrap_or_default();
    drop(s);
    string_from_rust(text)
}

// ----------------------------------------------------------------
// Build side
// ----------------------------------------------------------------

/// Mint a fresh JSON object builder. Returns a builder handle.
///
/// The builder lives until `corvid_json_object_finish` is called,
/// which serialises the accumulated fields and removes the builder
/// from the store. Builders cannot be reused after `_finish`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_new() -> i64 {
    let mut s = store().lock().expect("json store mutex poisoned");
    let handle = s.fresh();
    s.builders.insert(handle, Builder::new());
    handle as i64
}

fn builder_push(builder: &mut Builder, key: String, value: Value) {
    // Insertion-order semantics: the same field set twice keeps the
    // first slot's position with the second value. Codegen never
    // double-sets a field, so this branch is purely defensive.
    if let Some(slot) = builder.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = value;
    } else {
        builder.push((key, value));
    }
}

/// Set a named integer field on a builder. Silently no-ops if the
/// handle is invalid (caller's bug; codegen never produces invalid
/// builder handles).
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_set_int(
    handle: i64,
    name: CorvidString,
    value: i64,
) {
    let mut s = store().lock().expect("json store mutex poisoned");
    let key = unsafe { read_corvid_string(name) };
    if let Some(map) = s.builders.get_mut(&(handle as u64)) {
        builder_push(map, key, Value::from(value));
    }
}

/// Set a named bool field. `value != 0` is `true`, otherwise `false`.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_set_bool(
    handle: i64,
    name: CorvidString,
    value: i32,
) {
    let mut s = store().lock().expect("json store mutex poisoned");
    let key = unsafe { read_corvid_string(name) };
    if let Some(map) = s.builders.get_mut(&(handle as u64)) {
        builder_push(map, key, Value::from(value != 0));
    }
}

/// Set a named float field.
///
/// Non-finite floats (NaN, +Inf, -Inf) cannot be represented in JSON
/// (RFC 8259 forbids them); they are coerced to `0.0` rather than
/// failing the build. Codegen guards on the high-confidence path
/// where this never happens; the coercion is a defensive backstop.
///
/// # Safety
///
/// `name` must be a valid `CorvidString`.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_set_float(
    handle: i64,
    name: CorvidString,
    value: f64,
) {
    let mut s = store().lock().expect("json store mutex poisoned");
    let key = unsafe { read_corvid_string(name) };
    if let Some(map) = s.builders.get_mut(&(handle as u64)) {
        let json_num = serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::from(0.0));
        builder_push(map, key, json_num);
    }
}

/// Set a named string field. Reads the string's bytes into the
/// builder's owned storage, then releases the input refcount.
///
/// # Safety
///
/// `name` and `value` must both be valid `CorvidString`s.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_set_str(
    handle: i64,
    name: CorvidString,
    value: CorvidString,
) {
    let mut s = store().lock().expect("json store mutex poisoned");
    let key = unsafe { read_corvid_string(name) };
    let text = unsafe { read_corvid_string(value) };
    if let Some(map) = s.builders.get_mut(&(handle as u64)) {
        builder_push(map, key, Value::String(text));
    }
}

/// Serialise a builder to a `CorvidString` (refcount 1) and remove
/// the builder from the store. The caller takes ownership of the
/// returned string.
///
/// Field order is iteration order of the underlying `serde_json::Map`,
/// which preserves insertion order. Codegen-emitted encoders insert
/// fields in source-order matching `IrType.fields`, so the JSON
/// output matches the user's struct declaration order.
///
/// Returns an empty string if the handle is invalid or has already
/// been finished.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_object_finish(handle: i64) -> CorvidString {
    let mut s = store().lock().expect("json store mutex poisoned");
    let fields = match s.builders.remove(&(handle as u64)) {
        Some(f) => f,
        None => return string_from_rust(String::new()),
    };
    drop(s);
    string_from_rust(serialize_object_in_insertion_order(&fields))
}

/// Serialise a builder's fields as a JSON object in insertion order.
///
/// `serde_json::to_string` on a `Map<String, Value>` would alphabetise
/// the keys (Map is BTreeMap-backed). Hand-rolling the outer object
/// while delegating each value to `serde_json::to_string` preserves
/// insertion order, lets each value (which may itself be a nested
/// object, array, etc.) follow standard JSON rules, and avoids
/// pulling in `indexmap` or flipping the workspace-wide
/// `preserve_order` feature flag.
fn serialize_object_in_insertion_order(fields: &[(String, Value)]) -> String {
    let mut out = String::with_capacity(fields.len() * 16);
    out.push('{');
    for (i, (key, value)) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        // serde_json escapes the key correctly (handling embedded
        // quotes, control chars, non-ASCII). We then write the
        // serialised value next to the colon — same delegation
        // pattern serde_json uses internally.
        let key_json = serde_json::to_string(key).unwrap_or_else(|_| String::from("\"\""));
        out.push_str(&key_json);
        out.push(':');
        let value_json = serde_json::to_string(value).unwrap_or_else(|_| String::from("null"));
        out.push_str(&value_json);
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi_bridge::strings::release_string;

    fn cs(s: &str) -> CorvidString {
        string_from_rust(s.to_owned())
    }

    fn cs_to_string(s: CorvidString) -> String {
        let owned = unsafe { read_corvid_string(s) };
        unsafe { release_string(s) };
        owned
    }

    #[test]
    fn parse_round_trip_reads_each_primitive_field() {
        let h = unsafe { corvid_json_parse(cs(r#"{"a": 42, "b": true, "c": 3.5, "d": "hi"}"#)) };
        assert_ne!(h, 0);

        assert_eq!(unsafe { corvid_json_field_present(h, cs("a")) }, 1);
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("a")) }, 42);
        assert_eq!(unsafe { corvid_json_get_field_bool(h, cs("b")) }, 1);
        assert!((unsafe { corvid_json_get_field_float(h, cs("c")) } - 3.5).abs() < 1e-9);
        let d = unsafe { corvid_json_get_field_str(h, cs("d")) };
        assert_eq!(cs_to_string(d), "hi");

        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn missing_fields_return_zero_sentinels_and_present_returns_zero() {
        let h = unsafe { corvid_json_parse(cs(r#"{"a": 1}"#)) };
        assert_ne!(h, 0);

        assert_eq!(unsafe { corvid_json_field_present(h, cs("missing")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("missing")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_bool(h, cs("missing")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_float(h, cs("missing")) }, 0.0);
        let s = unsafe { corvid_json_get_field_str(h, cs("missing")) };
        assert_eq!(cs_to_string(s), "");

        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn type_mismatch_falls_through_to_zero_without_panic() {
        // "a" is a string; reading it as int / bool / float must hit
        // the zero sentinel rather than panic. This is the case the
        // bridge's retry loop relies on: decoder gets zero from a
        // mistyped field, returns 0 overall, bridge retries.
        let h = unsafe { corvid_json_parse(cs(r#"{"a": "not a number"}"#)) };
        assert_ne!(h, 0);

        assert_eq!(unsafe { corvid_json_field_present(h, cs("a")) }, 1);
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("a")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_bool(h, cs("a")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_float(h, cs("a")) }, 0.0);

        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn malformed_input_returns_null_handle() {
        let h = unsafe { corvid_json_parse(cs("not json {{")) };
        assert_eq!(h, 0);

        // All getters on a 0 handle return zero sentinels.
        assert_eq!(unsafe { corvid_json_field_present(0, cs("any")) }, 0);
        assert_eq!(unsafe { corvid_json_get_field_int(0, cs("any")) }, 0);

        // Releasing a 0 handle is a safe no-op.
        unsafe { corvid_json_release(0) };
    }

    #[test]
    fn integer_field_decodes_from_lenient_float_repr() {
        // LLMs sometimes emit `42.0` for an integer field; decoder
        // should accept it via the `as i64` cast.
        let h = unsafe { corvid_json_parse(cs(r#"{"n": 42.0}"#)) };
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("n")) }, 42);
        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn float_field_decodes_from_integer_repr() {
        let h = unsafe { corvid_json_parse(cs(r#"{"x": 7}"#)) };
        assert_eq!(unsafe { corvid_json_get_field_float(h, cs("x")) }, 7.0);
        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn build_round_trip_preserves_field_order() {
        let b = unsafe { corvid_json_object_new() };
        unsafe {
            corvid_json_object_set_str(b, cs("name"), cs("alice"));
            corvid_json_object_set_int(b, cs("age"), 30);
            corvid_json_object_set_bool(b, cs("admin"), 0);
        }
        let json = unsafe { corvid_json_object_finish(b) };
        let text = cs_to_string(json);
        // serde_json preserves insertion order in the Map; the
        // generated JSON should reflect the source-order setters.
        assert_eq!(text, r#"{"name":"alice","age":30,"admin":false}"#);
    }

    #[test]
    fn build_finish_releases_handle() {
        let b = unsafe { corvid_json_object_new() };
        let _ = cs_to_string(unsafe { corvid_json_object_finish(b) });

        // Setting after finish silently no-ops; finishing again
        // returns the empty-string sentinel.
        unsafe {
            corvid_json_object_set_int(b, cs("x"), 1);
        }
        let again = unsafe { corvid_json_object_finish(b) };
        assert_eq!(cs_to_string(again), "");
    }

    #[test]
    fn build_then_parse_round_trip() {
        let b = unsafe { corvid_json_object_new() };
        unsafe {
            corvid_json_object_set_str(b, cs("title"), cs("hello"));
            corvid_json_object_set_int(b, cs("count"), 3);
        }
        let built = unsafe { corvid_json_object_finish(b) };
        let h = unsafe { corvid_json_parse(built) };
        assert_ne!(h, 0);

        let title = unsafe { corvid_json_get_field_str(h, cs("title")) };
        assert_eq!(cs_to_string(title), "hello");
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("count")) }, 3);

        unsafe { corvid_json_release(h) };
    }

    #[test]
    fn handles_are_isolated_across_concurrent_parsers() {
        let h1 = unsafe { corvid_json_parse(cs(r#"{"k": 1}"#)) };
        let h2 = unsafe { corvid_json_parse(cs(r#"{"k": 2}"#)) };
        assert_ne!(h1, h2);
        assert_eq!(unsafe { corvid_json_get_field_int(h1, cs("k")) }, 1);
        assert_eq!(unsafe { corvid_json_get_field_int(h2, cs("k")) }, 2);
        unsafe {
            corvid_json_release(h1);
            corvid_json_release(h2);
        }
    }

    #[test]
    fn released_handle_returns_zero() {
        let h = unsafe { corvid_json_parse(cs(r#"{"k": 9}"#)) };
        unsafe { corvid_json_release(h) };
        assert_eq!(unsafe { corvid_json_get_field_int(h, cs("k")) }, 0);
        assert_eq!(unsafe { corvid_json_field_present(h, cs("k")) }, 0);
    }
}
