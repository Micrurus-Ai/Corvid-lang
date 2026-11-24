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
