//! Host tool registration bridge and C tool exports.
//!
//! Mirrors [`approval_bridge`](super::approval_bridge): a host — or a
//! self-registering `#[tool]` library — registers a tool implementation
//! *by name* at load time, and native tool-call codegen dispatches
//! through [`corvid_invoke_tool`] instead of a link-time
//! `__corvid_tool_<name>` symbol. That lets a signed cdylib link and
//! ship without the host's tool implementations baked in, the same way
//! approvers are already provided at runtime via
//! [`corvid_register_approver`](super::corvid_register_approver).
//!
//! Args and results are JSON, matching the `corvid_call_agent`
//! dispatch convention: a tool callback receives the call arguments as
//! a JSON array string and returns the result as a JSON string.

use crate::abi::{CorvidString, IntoCorvidAbi};
use crate::ffi_bridge::{borrow_corvid_string, read_corvid_string};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CString};
use std::ptr;
use std::sync::{Mutex, OnceLock};

/// Host tool callback.
///
/// `args_json` / `args_len` carry the call arguments as a UTF-8 JSON
/// array (`[arg0, arg1, ...]`) — borrowed for the duration of the call.
/// The callback returns the result as a freshly-allocated JSON C string
/// that the *caller* owns and frees (via `corvid_free_result`), or null
/// to signal failure. `user_data` is the opaque pointer supplied at
/// registration.
pub type CorvidToolFn = unsafe extern "C" fn(
    args_json: *const c_char,
    args_len: usize,
    user_data: *mut c_void,
) -> *mut c_char;

struct ToolRegistration {
    callback: CorvidToolFn,
    user_data: usize,
}

fn registry() -> &'static Mutex<HashMap<String, ToolRegistration>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, ToolRegistration>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register (or, with `fn_ptr == None`, unregister) a tool by name.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn corvid_register_tool(
    name: *const c_char,
    fn_ptr: Option<CorvidToolFn>,
    user_data: *mut c_void,
) {
    let Ok(name) = super::grounded_bridge::read_c_string(name) else {
        return;
    };
    let mut registry = registry().lock().unwrap();
    match fn_ptr {
        Some(callback) => {
            registry.insert(
                name,
                ToolRegistration {
                    callback,
                    user_data: user_data as usize,
                },
            );
        }
        None => {
            registry.remove(&name);
        }
    }
}

/// Drop every registered tool. Mirrors `corvid_clear_approver`.
#[no_mangle]
pub extern "C" fn corvid_clear_tools() {
    registry().lock().unwrap().clear();
}

/// Invoke the tool registered under `name`, forwarding the JSON args to
/// its callback and returning the callback's owned JSON result. Returns
/// null when no tool is registered under `name` (the caller surfaces a
/// "tool not registered" runtime error) or when `name` is invalid.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated C string; `args_json` /
/// `args_len` must describe a valid byte range for the call.
#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool(
    name: *const c_char,
    args_json: *const c_char,
    args_len: usize,
) -> *mut c_char {
    let Ok(name) = super::grounded_bridge::read_c_string(name) else {
        return ptr::null_mut();
    };
    // Copy the callback + user_data out under the lock, then release the
    // lock before invoking so a tool that itself calls back into the
    // registry (e.g. registers another tool) can't deadlock.
    let registration = {
        let registry = registry().lock().unwrap();
        match registry.get(&name) {
            Some(entry) => (entry.callback, entry.user_data),
            None => return ptr::null_mut(),
        }
    };
    let (callback, user_data) = registration;
    callback(args_json, args_len, user_data as *mut c_void)
}

/// Shared dispatch core for the typed `corvid_invoke_tool_*` family.
/// Looks up the registered callback for `name`, forwards `args_json`,
/// and reclaims the callback's result string. Returns `None` when no
/// tool is registered or the callback fails. The result pointer is
/// reclaimed with `CString::from_raw` per the [`CorvidToolFn`] contract
/// (the callback returns a `CString::into_raw`-allocated string).
fn dispatch_registered_tool(name: &str, args_json: &str) -> Option<String> {
    let (callback, user_data) = {
        let registry = registry().lock().unwrap();
        let entry = registry.get(name)?;
        (entry.callback, entry.user_data)
    };
    let args_c = CString::new(args_json).ok()?;
    let result_ptr = unsafe {
        callback(
            args_c.as_ptr(),
            args_c.as_bytes().len(),
            user_data as *mut c_void,
        )
    };
    if result_ptr.is_null() {
        return None;
    }
    let result = unsafe { CString::from_raw(result_ptr) };
    result.into_string().ok()
}

/// Decode the codegen-supplied trace-buffer args, dispatch to the
/// registered host tool, and return `(tool_name, result_json)`. Panics
/// with a clear message when the tool is not registered — a live native
/// call to an unregistered tool is a host wiring error, mirroring how
/// the replay family panics on a missing recorded result.
///
/// # Safety
///
/// `tool` / `arg_types` must be valid `CorvidString`s and
/// `argc` / `args_ptr` a valid trace-value buffer (as produced by the
/// tool-call codegen, identical to the replay path).
unsafe fn invoke_decoded_json(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> (String, String) {
    let tool_name = read_corvid_string(tool);
    let arg_tags = borrow_corvid_string(&arg_types);
    let args = crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr);
    let args_json = serde_json::to_string(&args).unwrap_or_else(|_| "[]".to_string());
    let result_json = dispatch_registered_tool(&tool_name, &args_json).unwrap_or_else(|| {
        panic!(
            "corvid tool `{tool_name}` is not registered (or its callback returned null); a host must register it via corvid_register_tool before the agent invokes it"
        )
    });
    (tool_name, result_json)
}

fn parse_tool_result(tool_name: &str, result_json: &str) -> serde_json::Value {
    serde_json::from_str(result_json).unwrap_or_else(|err| {
        panic!("corvid tool `{tool_name}` returned invalid JSON (`{result_json}`): {err}")
    })
}

#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_int(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> i64 {
    let (name, json) = invoke_decoded_json(tool, arg_types, argc, args_ptr);
    parse_tool_result(&name, &json)
        .as_i64()
        .unwrap_or_else(|| panic!("corvid tool `{name}` returned non-int result `{json}`"))
}

#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_bool(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> bool {
    let (name, json) = invoke_decoded_json(tool, arg_types, argc, args_ptr);
    parse_tool_result(&name, &json)
        .as_bool()
        .unwrap_or_else(|| panic!("corvid tool `{name}` returned non-bool result `{json}`"))
}

#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_float(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> f64 {
    let (name, json) = invoke_decoded_json(tool, arg_types, argc, args_ptr);
    parse_tool_result(&name, &json)
        .as_f64()
        .unwrap_or_else(|| panic!("corvid tool `{name}` returned non-float result `{json}`"))
}

#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_string(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> CorvidString {
    let (name, json) = invoke_decoded_json(tool, arg_types, argc, args_ptr);
    parse_tool_result(&name, &json)
        .as_str()
        .unwrap_or_else(|| panic!("corvid tool `{name}` returned non-string result `{json}`"))
        .to_owned()
        .into_corvid_abi()
}

#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_nothing(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) {
    let _ = invoke_decoded_json(tool, arg_types, argc, args_ptr);
}

/// Struct-returning tools hand the result JSON object straight back to
/// the caller (the native tool-call codegen decodes it into a struct
/// handle via the per-struct decoder, the same way prompt struct
/// returns are decoded).
#[no_mangle]
pub unsafe extern "C" fn corvid_invoke_tool_struct(
    tool: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> CorvidString {
    let (_, json) = invoke_decoded_json(tool, arg_types, argc, args_ptr);
    json.into_corvid_abi()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static ECHO_CALLS: AtomicUsize = AtomicUsize::new(0);

    // A registered tool that echoes its JSON args back inside a result
    // object, and counts invocations so the test can prove dispatch.
    unsafe extern "C" fn echo_tool(
        args_json: *const c_char,
        args_len: usize,
        _user_data: *mut c_void,
    ) -> *mut c_char {
        ECHO_CALLS.fetch_add(1, Ordering::SeqCst);
        let bytes = std::slice::from_raw_parts(args_json as *const u8, args_len);
        let args = String::from_utf8_lossy(bytes).into_owned();
        CString::new(format!("{{\"echo\":{args}}}"))
            .unwrap()
            .into_raw()
    }

    #[test]
    fn register_invoke_and_clear_roundtrip() {
        corvid_clear_tools();
        ECHO_CALLS.store(0, Ordering::SeqCst);

        let name = CString::new("do_thing").unwrap();
        let args = CString::new("[\"hello\"]").unwrap();

        // Unregistered → null.
        let before = unsafe {
            corvid_invoke_tool(name.as_ptr(), args.as_ptr(), args.as_bytes().len())
        };
        assert!(before.is_null(), "unregistered tool must return null");

        // Register, then invoke: the host callback runs and its JSON
        // result comes back to the caller.
        unsafe { corvid_register_tool(name.as_ptr(), Some(echo_tool), ptr::null_mut()) };
        let result_ptr = unsafe {
            corvid_invoke_tool(name.as_ptr(), args.as_ptr(), args.as_bytes().len())
        };
        assert!(!result_ptr.is_null(), "registered tool must dispatch");
        let result = unsafe { CString::from_raw(result_ptr) }
            .into_string()
            .unwrap();
        assert_eq!(result, "{\"echo\":[\"hello\"]}");
        assert_eq!(ECHO_CALLS.load(Ordering::SeqCst), 1);

        // Clearing removes it again.
        corvid_clear_tools();
        let after = unsafe {
            corvid_invoke_tool(name.as_ptr(), args.as_ptr(), args.as_bytes().len())
        };
        assert!(after.is_null(), "cleared tool must return null");
    }

    // A tool returning a JSON object (a "receipt"), exercising the
    // dispatch core the typed `corvid_invoke_tool_*` family delegates to.
    unsafe extern "C" fn receipt_tool(
        _args_json: *const c_char,
        _args_len: usize,
        _user_data: *mut c_void,
    ) -> *mut c_char {
        CString::new(r#"{"id":"r-1","status":"sent"}"#)
            .unwrap()
            .into_raw()
    }

    #[test]
    fn dispatch_registered_tool_reclaims_and_parses_result() {
        corvid_clear_tools();
        let name = CString::new("share").unwrap();
        unsafe { corvid_register_tool(name.as_ptr(), Some(receipt_tool), ptr::null_mut()) };

        // Registered: dispatch returns the callback's JSON result.
        let got = dispatch_registered_tool("share", "[\"hi\"]");
        assert_eq!(got.as_deref(), Some(r#"{"id":"r-1","status":"sent"}"#));

        // Unregistered: None (the typed family turns this into a clear
        // "tool not registered" panic at the live call site).
        assert_eq!(dispatch_registered_tool("missing", "[]"), None);
        corvid_clear_tools();
    }
}
