//! Native entry points for the B03 text methods: encoders, decoders
//! and UUID parsing. Each one borrows its receiver and calls
//! `corvid-text`, the code the interpreter calls, so both tiers return
//! the same text and the same error messages.
//!
//! Encoders return a fresh `String`. Decoders follow
//! `corvid_string_parse_int`: they write the Ok or Err `String` into a
//! native `Result` payload slot and return its tag, 0 for Ok and 1 for
//! Err.

use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;

fn write_result(result: Result<String, String>, payload: *mut CorvidString) -> i64 {
    let (tag, text) = match result {
        Ok(text) => (0, text),
        Err(message) => (1, message),
    };
    // SAFETY: the caller passes an aligned, writable Result payload slot.
    unsafe { payload.write(string_from_rust(text)) };
    tag
}

macro_rules! text_encoder {
    ($symbol:ident, $rule:path) => {
        /// # Safety
        /// The receiver must remain a live Corvid string for this call.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(value: CorvidString) -> CorvidString {
            string_from_rust($rule(unsafe { borrow_corvid_string(&value) }))
        }
    };
}

macro_rules! text_decoder {
    ($symbol:ident, $rule:path) => {
        /// # Safety
        /// The receiver must remain a live Corvid string for this call,
        /// and `payload` must point to a writable native Result payload
        /// slot; the written String is owned by the Result.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(value: CorvidString, payload: *mut CorvidString) -> i64 {
            write_result($rule(unsafe { borrow_corvid_string(&value) }), payload)
        }
    };
}

text_encoder!(corvid_string_encode_base64, corvid_text::encode_base64);
text_encoder!(corvid_string_encode_base64_url, corvid_text::encode_base64_url);
text_encoder!(corvid_string_encode_hex, corvid_text::encode_hex);
text_encoder!(corvid_string_encode_url_component, corvid_text::encode_url_component);
text_decoder!(corvid_string_decode_base64, corvid_text::decode_base64);
text_decoder!(corvid_string_decode_base64_url, corvid_text::decode_base64_url);
text_decoder!(corvid_string_decode_hex, corvid_text::decode_hex);
text_decoder!(corvid_string_decode_url_component, corvid_text::decode_url_component);
text_decoder!(corvid_string_parse_uuid, corvid_text::parse_uuid);

/// The checker validated every pattern with the same parser, so an
/// `Err` here is a compiler bug, not bad input: stop loudly.
fn checked<T>(result: Result<T, String>) -> T {
    result.unwrap_or_else(|message| {
        eprintln!("corvid: {message}");
        std::process::abort()
    })
}

/// # Safety
/// Each input descriptor must remain live for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_contains_regex(value: CorvidString, pattern: CorvidString) -> u8 {
    let (text, pattern) = unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern)) };
    checked(corvid_text::regex::contains(text, pattern)) as u8
}

/// A `RegexMatch` as JSON, its fields named by
/// `corvid_types::regex_match_fields`, the list the interpreter builds its
/// record from.
fn regex_match_json(found: corvid_text::regex::RegexMatch) -> serde_json::Value {
    let corvid_text::regex::RegexMatch { text, start, end, groups } = found;
    let mut text = Some(text);
    let mut groups = Some(groups);
    let fields = corvid_types::regex_match_fields().map(|(name, _)| {
        let value = match name {
            "text" => serde_json::Value::from(text.take().unwrap_or_default()),
            "start" => serde_json::Value::from(start),
            "end" => serde_json::Value::from(end),
            "groups" => serde_json::Value::from(groups.take().unwrap_or_default()),
            other => unreachable!("RegexMatch field `{other}` has no runtime value"),
        };
        (name.to_string(), value)
    });
    serde_json::Value::Object(fields.into_iter().collect())
}

/// `text.find_regex(pattern) -> Option<RegexMatch>`: the first match,
/// built as the native value of the plan's root type.
///
/// # Safety
/// Each input descriptor must remain live for the duration of the call;
/// `plan`, `plan_len` and `typeinfos` describe `Option<RegexMatch>`.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_find_regex(
    value: CorvidString,
    pattern: CorvidString,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const std::ffi::c_void,
) -> i64 {
    let (text, pattern) = unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern)) };
    let found = checked(corvid_text::regex::find(text, pattern)).map(regex_match_json);
    let json = found.unwrap_or(serde_json::Value::Null);
    unsafe { super::typed_json::native_value_from_json(&json, plan, plan_len, typeinfos) }
}

/// `text.find_all_regex(pattern) -> List<RegexMatch>`.
///
/// # Safety
/// As for [`corvid_string_find_regex`], with a plan for `List<RegexMatch>`.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_find_all_regex(
    value: CorvidString,
    pattern: CorvidString,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const std::ffi::c_void,
) -> i64 {
    let (text, pattern) = unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern)) };
    let found: Vec<serde_json::Value> =
        checked(corvid_text::regex::find_all(text, pattern)).into_iter().map(regex_match_json).collect();
    unsafe { super::typed_json::native_value_from_json(&serde_json::Value::Array(found), plan, plan_len, typeinfos) }
}

/// `replace_regex` with a literal template the checker validated.
///
/// # Safety
/// Each input descriptor must remain live for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_replace_regex(
    value: CorvidString,
    pattern: CorvidString,
    template: CorvidString,
) -> CorvidString {
    let (text, pattern, template) =
        unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern), borrow_corvid_string(&template)) };
    string_from_rust(checked(corvid_text::regex::replace(text, pattern, template)))
}

/// `replace_regex` with a replacement computed at run time, inserted
/// as written.
///
/// # Safety
/// Each input descriptor must remain live for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_replace_regex_verbatim(
    value: CorvidString,
    pattern: CorvidString,
    replacement: CorvidString,
) -> CorvidString {
    let (text, pattern, replacement) =
        unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern), borrow_corvid_string(&replacement)) };
    string_from_rust(checked(corvid_text::regex::replace_verbatim(text, pattern, replacement)))
}

/// `split_regex`, appending each piece to the fresh native list of
/// strings the caller allocated, as `corvid_string_split` does.
///
/// # Safety
/// The inputs must remain live. `list` must be a fresh native list of
/// strings, which is returned.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_split_regex(
    value: CorvidString,
    pattern: CorvidString,
    list: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    extern "C" {
        fn corvid_list_append(list: *mut std::ffi::c_void, slot: i64);
    }
    let (text, pattern) = unsafe { (borrow_corvid_string(&value), borrow_corvid_string(&pattern)) };
    for piece in checked(corvid_text::regex::split(text, pattern)) {
        let descriptor = string_from_rust(piece);
        // SAFETY: the list owns the appended String's reference.
        let raw: *const u8 = unsafe { std::mem::transmute(descriptor) };
        unsafe { corvid_list_append(list, raw as isize as i64) };
    }
    list
}
