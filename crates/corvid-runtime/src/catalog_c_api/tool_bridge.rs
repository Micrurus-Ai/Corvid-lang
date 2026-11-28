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

/// Allocate a tool-result buffer the bridge can reclaim, portably.
///
/// The [`CorvidToolFn`] contract requires the returned result pointer
/// to have been allocated by THIS library's Rust allocator (the
/// bridge reclaims it with `CString::from_raw`). On linux-gnu Rust's
/// default allocator IS libc malloc, so a host `malloc` happens to
/// satisfy the contract — but on Windows Rust allocates from the
/// process heap via HeapAlloc while a host CRT `malloc` may not, and
/// the mismatch corrupts the heap (observed as a silent hard crash
/// of the host process). This export removes the portability trap:
/// it returns a `content_len + 1`-byte buffer (space-filled,
/// NUL-terminated) allocated exactly the way `CString::from_raw`
/// expects.
///
/// Contract for hosts: overwrite ALL `content_len` bytes with
/// NUL-free content (JSON results are NUL-free) before returning the
/// pointer from the callback — a shorter write leaves an interior
/// NUL and the reclaim's length bookkeeping becomes undefined
/// behavior. To abandon an allocated buffer without returning it,
/// pass it to `corvid_free_result`.
#[no_mangle]
pub extern "C" fn corvid_alloc_result(content_len: usize) -> *mut c_char {
    let filled = vec![b' '; content_len];
    match CString::new(filled) {
        Ok(s) => s.into_raw(),
        Err(_) => ptr::null_mut(),
    }
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

/// Register a `#[tool]`-provided JSON dispatcher under `name`. Called
/// from `corvid_runtime_init` for every inventoried tool so a linked
/// `#[tool]` lib self-registers under the unified dispatch model
/// (`CorvidToolJsonFn` and `CorvidToolFn` are the same fn-pointer type).
pub(crate) fn register_inventoried_tool(name: &str, dispatch: crate::abi::CorvidToolJsonFn) {
    registry().lock().unwrap().insert(
        name.to_string(),
        ToolRegistration {
            callback: dispatch,
            user_data: 0,
        },
    );
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

/// Whether a tool is registered under `name`: a linked `#[tool]` or one
/// an embedding host registered.
pub fn host_tool_registered(name: &str) -> bool {
    registry().lock().unwrap().contains_key(name)
}

thread_local! {
    /// The failure a tool reported on this thread, read right after its
    /// entry point returns. The entry point runs on the calling thread,
    /// so a thread-local pairs each report with its call.
    static REPORTED_FAILURE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Report that the tool running on this thread failed with `message`;
/// its entry point then returns null. A `#[tool]` whose `async fn`
/// returns `Err` does this, and so may any host tool.
pub fn report_tool_failure(message: String) {
    REPORTED_FAILURE.with(|slot| *slot.borrow_mut() = Some(message));
}

/// `report_tool_failure` for a host tool written in C: call it with a
/// NUL-terminated message, then return null from the tool's callback.
///
/// # Safety
/// `message` must be null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn corvid_tool_failed(message: *const c_char) {
    let message = super::grounded_bridge::read_c_string(message).unwrap_or_else(|_| "the tool failed".to_string());
    report_tool_failure(message);
}

/// Call the host tool registered under `name` with `args`, as both the
/// runtime and `corvid serve --with-tools-cdylib` do, so the two read a
/// tool's answer and its failures the same way. A tool that is not
/// registered is `UnknownTool`; a reported failure, a missing answer and
/// an answer that is not JSON are `ToolFailed`, which is an outage.
pub fn call_registered_host_tool(name: &str, args: &[serde_json::Value]) -> Result<serde_json::Value, crate::RuntimeError> {
    let failed = |message: String| crate::RuntimeError::ToolFailed { tool: name.to_string(), message };
    if !host_tool_registered(name) {
        return Err(crate::RuntimeError::UnknownTool(name.to_string()));
    }
    let mut args_json = serde_json::to_string(args).map_err(|e| failed(format!("its arguments are not JSON: {e}")))?;
    REPORTED_FAILURE.with(|slot| slot.borrow_mut().take());
    let answer = dispatch_registered_tool(name, &args_json);
    // The arguments may hold a revealed secret; wipe this serialized copy.
    corvid_crypto::wipe_text(&mut args_json);
    if let Some(message) = REPORTED_FAILURE.with(|slot| slot.borrow_mut().take()) {
        return Err(failed(message));
    }
    let answer = answer.ok_or_else(|| failed("the implementation returned no answer".into()))?;
    serde_json::from_str(&answer).map_err(|e| failed(format!("its answer is not JSON: {e}")))
}

/// Public in-process bridge for host code that has registered tools via
/// [`corvid_register_tool`] and wants the raw JSON answer. Returns
/// `Some(result_json)` when a callback is registered for `name` and
/// produces a result; `None` when no tool is registered or the callback
/// fails. Prefer [`call_registered_host_tool`], which also reads a
/// reported failure.
pub fn dispatch_host_tool(name: &str, args_json: &str) -> Option<String> {
    dispatch_registered_tool(name, args_json)
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
    // The C copy of the arguments may hold a revealed secret: wipe it.
    corvid_crypto::wipe_bytes(&mut args_c.into_bytes());
    if result_ptr.is_null() {
        return None;
    }
    let result = unsafe { CString::from_raw(result_ptr) };
    result.into_string().ok()
}

#[cfg(test)]
mod tests {
    unsafe extern "C" fn reports_a_failure(_args: *const c_char, _len: usize, _user: *mut c_void) -> *mut c_char {
        let message = CString::new("the upstream is down").unwrap();
        unsafe { corvid_tool_failed(message.as_ptr()) };
        ptr::null_mut()
    }

    unsafe extern "C" fn answers_nothing(_args: *const c_char, _len: usize, _user: *mut c_void) -> *mut c_char {
        ptr::null_mut()
    }

    #[test]
    fn a_reported_failure_is_a_failed_call_and_a_missing_tool_is_unknown() {
        let reported = CString::new("reporting_tool").unwrap();
        let silent = CString::new("silent_tool").unwrap();
        unsafe {
            corvid_register_tool(reported.as_ptr(), Some(reports_a_failure), ptr::null_mut());
            corvid_register_tool(silent.as_ptr(), Some(answers_nothing), ptr::null_mut());
        }
        match call_registered_host_tool("reporting_tool", &[]) {
            Err(crate::RuntimeError::ToolFailed { message, .. }) => assert_eq!(message, "the upstream is down"),
            other => panic!("expected a failed call, got {other:?}"),
        }
        match call_registered_host_tool("silent_tool", &[]) {
            Err(error @ crate::RuntimeError::ToolFailed { .. }) => assert!(error.is_outage()),
            other => panic!("expected a failed call, got {other:?}"),
        }
        // The report does not leak into the next call.
        assert!(matches!(call_registered_host_tool("silent_tool", &[]), Err(crate::RuntimeError::ToolFailed { ref message, .. }) if message.contains("no answer")));
        assert!(matches!(call_registered_host_tool("never_registered", &[]), Err(crate::RuntimeError::UnknownTool(_))));
    }

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // A callback that allocates its result through the exported
    // portable allocator — the contract every non-Rust host must
    // follow (host malloc only matches Rust's allocator on
    // linux-gnu).
    unsafe extern "C" fn alloc_result_tool(
        _args_json: *const c_char,
        _args_len: usize,
        _user_data: *mut c_void,
    ) -> *mut c_char {
        let content = br#""portable""#;
        let ptr = corvid_alloc_result(content.len());
        if ptr.is_null() {
            return ptr;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(content.as_ptr(), ptr as *mut u8, content.len());
        }
        ptr
    }

    #[test]
    fn alloc_result_buffer_round_trips_through_dispatch() {
        let _guard = test_guard();
        let name = CString::new("portable_alloc_tool").unwrap();
        unsafe {
            corvid_register_tool(name.as_ptr(), Some(alloc_result_tool), ptr::null_mut());
        }
        let result = dispatch_registered_tool("portable_alloc_tool", "[]")
            .expect("dispatch reclaims the corvid_alloc_result buffer");
        assert_eq!(result, r#""portable""#);
        unsafe {
            corvid_register_tool(name.as_ptr(), None, ptr::null_mut());
        }
    }

    #[test]
    fn alloc_result_zero_len_yields_reclaimable_empty_buffer() {
        let ptr = corvid_alloc_result(0);
        assert!(!ptr.is_null());
        // Reclaim exactly the way the bridge does.
        let s = unsafe { CString::from_raw(ptr) };
        assert_eq!(s.as_bytes(), b"");
    }

    static ECHO_CALLS: AtomicUsize = AtomicUsize::new(0);

    /// The tool registry is a process-wide global; tests that
    /// register/clear/dispatch against it must not interleave.
    /// Every test below takes this guard first — the pre-existing
    /// intermittent failures in this module were exactly this race.
    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

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
        let _guard = test_guard();
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
        let _guard = test_guard();
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

    #[test]
    fn inventoried_tool_registers_and_dispatches() {
        let _guard = test_guard();
        // The init-path helper registers a `#[tool]`'s JSON dispatcher
        // (CorvidToolJsonFn) the same way a host's `corvid_register_tool`
        // would, and `corvid_invoke_tool` then dispatches to it.
        corvid_clear_tools();
        register_inventoried_tool("double", echo_tool);
        let name = CString::new("double").unwrap();
        let args = CString::new("[\"x\"]").unwrap();
        let result_ptr =
            unsafe { corvid_invoke_tool(name.as_ptr(), args.as_ptr(), args.as_bytes().len()) };
        assert!(!result_ptr.is_null(), "inventoried tool must dispatch");
        let result = unsafe { CString::from_raw(result_ptr) }
            .into_string()
            .unwrap();
        assert_eq!(result, "{\"echo\":[\"x\"]}");
        corvid_clear_tools();
    }
}
