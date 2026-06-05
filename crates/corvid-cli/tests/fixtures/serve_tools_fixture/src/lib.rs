//! Tiny cdylib fixture for the `serve --with-tools-cdylib` integration
//! test. Exposes a single tool, `echo_string`, that matches the
//! `CorvidToolFn` ABI from
//! `crates/corvid-runtime/src/catalog_c_api/tool_bridge.rs`.
//!
//! The semantics: receive a JSON args array `["<some-string>"]`,
//! return JSON `"<some-string>"`. The integration test calls the tool
//! via the interpreter's approval-gated path and asserts the string
//! round-trips through the cdylib unchanged — proving the
//! dlopen + `corvid_register_tool` + `dispatch_host_tool` bridge in
//! `crates/corvid-cli/src/serve_cmd.rs::register_cdylib_tool_handlers`
//! actually delivers the tool call to the host implementation.

use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::slice;

/// `CorvidToolFn` signature:
/// `unsafe extern "C" fn(*const c_char, usize, *mut c_void) -> *mut c_char`.
///
/// The CLI dlsyms this symbol from the fixture cdylib and registers it
/// via `corvid_register_tool("echo_string", fn_ptr, null)`. When the
/// interpreter calls `echo_string("hello")`, the result is a JSON
/// string `"hello"` allocated as a `CString::into_raw` that the
/// runtime reclaims with `corvid_free_result`.
///
/// # Safety
///
/// `args_json` must be a valid pointer to `args_len` bytes of UTF-8
/// JSON. The CLI satisfies this contract — it serialises the
/// interpreter's `Vec<serde_json::Value>` via `serde_json::to_string`
/// and passes the resulting bytes + length unchanged.
#[no_mangle]
pub unsafe extern "C" fn __corvid_tool_echo_string(
    args_json: *const c_char,
    args_len: usize,
    _user_data: *mut c_void,
) -> *mut c_char {
    if args_json.is_null() {
        return ptr::null_mut();
    }
    let bytes = slice::from_raw_parts(args_json as *const u8, args_len);
    let Ok(args_str) = std::str::from_utf8(bytes) else {
        return ptr::null_mut();
    };
    let Ok(parsed): Result<Vec<serde_json::Value>, _> = serde_json::from_str(args_str) else {
        return ptr::null_mut();
    };
    let Some(first) = parsed.into_iter().next() else {
        return ptr::null_mut();
    };
    // `first` should be a JSON string per the tool's declared signature
    // `echo_string(value: String) -> String`. Re-emit it as JSON so the
    // runtime's result decoder parses it back to a string.
    let Ok(result_json) = serde_json::to_string(&first) else {
        return ptr::null_mut();
    };
    match CString::new(result_json) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}
