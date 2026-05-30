#![allow(unsafe_code)]

mod approval_bridge;
mod catalog_exports;
mod grounded_bridge;
mod invoke_matrix;
mod tool_bridge;
pub use approval_bridge::{
    corvid_approval_predicate_json, corvid_clear_approver, corvid_evaluate_approval_predicate,
    corvid_mark_preapproved_request, corvid_record_host_event, corvid_register_approver,
    corvid_register_approver_from_source, CorvidHostEventStatus,
};
pub(crate) use approval_bridge::{
    decide_registered_approval, mark_preapproved_request, request_host_approval,
    take_last_approval_detail, ApprovalRequestOutcome,
};
pub use catalog_exports::{
    corvid_agent_signature_json, corvid_call_agent, corvid_find_agents_where, corvid_free_result,
    corvid_list_agents, corvid_pre_flight,
};
pub use tool_bridge::{
    corvid_clear_tools, corvid_invoke_tool, corvid_register_tool, CorvidToolFn,
};
pub(crate) use tool_bridge::register_inventoried_tool;
pub use grounded_bridge::{
    corvid_begin_direct_observation, corvid_finish_direct_observation, corvid_grounded_attest_bool,
    corvid_grounded_attest_float, corvid_grounded_attest_int, corvid_grounded_attest_string,
    corvid_grounded_attest_struct, corvid_grounded_capture_scalar_handle,
    corvid_grounded_capture_string_handle, corvid_grounded_capture_struct_handle,
    corvid_grounded_confidence, corvid_grounded_release, corvid_grounded_sources,
    corvid_observation_cost_usd, corvid_observation_exceeded_bound, corvid_observation_latency_ms,
    corvid_observation_release, corvid_observation_tokens_in, corvid_observation_tokens_out,
};
pub(crate) use invoke_matrix::build_scalar_invoker;

use crate::catalog::{descriptor_hash, descriptor_json_ptr};
use crate::errors::RuntimeError;
use corvid_abi::{read_embedded_section_from_library, EmbeddedDescriptorSection};
use std::ffi::{c_char, c_void, CString};
use std::path::PathBuf;
use std::ptr;

/// Reads the cdylib's embedded ABI descriptor by first resolving
/// the path to the *current Corvid library file* (the cdylib
/// holding `corvid_abi_descriptor_json`, not the host process)
/// and then opening that file through
/// `read_embedded_section_from_library`. The two-step path
/// works the same on Windows and Unix; only the cdylib-path
/// lookup uses an OS-specific anchor (Windows:
/// `GetModuleHandleExA(FROM_ADDRESS, anchor)`; Unix:
/// `dladdr(anchor, &info)`). This is the correctness fix for
/// the long-standing demo-verify `bundle_verify` CI failure
/// where `libloading::os::unix::Library::this()` resolved to
/// the *main program* (e.g. `python3`) instead of the cdylib
/// the symbol actually lives in, producing
/// `undefined symbol: CORVID_ABI_DESCRIPTOR`.
pub(crate) fn load_embedded_descriptor_from_current_library(
) -> Result<EmbeddedDescriptorSection, RuntimeError> {
    let path = current_library_path()?;
    read_embedded_section_from_library(&path).map_err(|err| {
        RuntimeError::Other(format!(
            "read embedded descriptor from `{}`: {err}",
            path.display()
        ))
    })
}

#[cfg(unix)]
fn current_library_path() -> Result<PathBuf, RuntimeError> {
    use std::ffi::CStr;

    #[repr(C)]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }

    extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> std::os::raw::c_int;
    }

    unsafe {
        let mut info = std::mem::MaybeUninit::<DlInfo>::zeroed();
        // Use a function known to be defined in this crate's
        // compiled object as the anchor. On Linux `dladdr`
        // returns the path to the loaded shared object that
        // contains the address — i.e. the Corvid cdylib when
        // we're loaded into a host like `python3`, the host
        // binary itself when we're statically linked into a
        // native build, and the correct image in any other
        // configuration.
        let anchor = corvid_abi_descriptor_json as *const c_void;
        let result = dladdr(anchor, info.as_mut_ptr());
        if result == 0 {
            return Err(RuntimeError::Other(
                "dladdr failed to resolve current Corvid library path".to_string(),
            ));
        }
        let info = info.assume_init();
        if info.dli_fname.is_null() {
            return Err(RuntimeError::Other(
                "dladdr returned null library path; the host loader cannot map \
                 the cdylib's address back to a file on disk"
                    .to_string(),
            ));
        }
        let path_cstr = CStr::from_ptr(info.dli_fname);
        let path_str = path_cstr.to_str().map_err(|err| {
            RuntimeError::Other(format!("library path UTF-8 decode: {err}"))
        })?;
        Ok(PathBuf::from(path_str))
    }
}

#[cfg(windows)]
fn current_library_path() -> Result<PathBuf, RuntimeError> {
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    unsafe {
        let mut module: HMODULE = std::ptr::null_mut();
        let anchor = corvid_abi_descriptor_json as *const ();
        let ok = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor.cast(),
            &mut module,
        );
        if ok == 0 || module.is_null() {
            return Err(RuntimeError::Other(
                "resolve current Corvid module handle".to_string(),
            ));
        }

        let mut buf = vec![0u16; 260];
        loop {
            let written = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32);
            if written == 0 {
                return Err(RuntimeError::Other(
                    "resolve current Corvid module path".to_string(),
                ));
            }
            let written = written as usize;
            if written < buf.len() - 1 {
                buf.truncate(written);
                let path = String::from_utf16(&buf).map_err(|err| {
                    RuntimeError::Other(format!("module path UTF-16 decode: {err}"))
                })?;
                return Ok(PathBuf::from(path));
            }
            buf.resize(buf.len() * 2, 0);
        }
    }
}

/// Resolves a symbol exported by the current Corvid library
/// (the cdylib when hosted, or the native binary when statically
/// linked). Used by `invoke_matrix::build_scalar_invoker` to look
/// up `pub extern "C"` agent entrypoints and by other catalog
/// surfaces that need to call into the compiled-Corvid code via
/// raw addresses.
///
/// The historical Unix implementation called
/// `libloading::os::unix::Library::this()`, which on glibc
/// returns a handle to the *main program* (`dlopen(NULL)`) — not
/// the cdylib the calling code lives in. When the cdylib was
/// loaded by a host like `python3 ctypes.CDLL("classify.so")`,
/// every symbol lookup failed with
/// `undefined symbol: <agent_name>` because Python's main
/// program never exports the cdylib's symbols (default
/// `RTLD_LOCAL` on Linux). Windows did not have this bug because
/// `GetModuleHandleExA(FROM_ADDRESS, anchor)` already returned
/// the cdylib's module.
///
/// The fix uses `dladdr(anchor)` to find the path of the shared
/// object that contains the calling code, then opens that file
/// via `Library::open` (refcounted; same handle as the existing
/// dlopen). The library handle is cached in a `OnceLock` so we
/// pay the dladdr+dlopen cost once per process. A fallback to
/// `Library::this()` is preserved for native-binary configurations
/// where the agent symbols are exported via `--export-dynamic`
/// from the main program.
pub(crate) unsafe fn resolve_current_library_symbol(
    symbol: &str,
) -> Result<*const c_void, RuntimeError> {
    #[cfg(unix)]
    {
        let lib = current_library_unix();
        let export = lib
            .get::<*const c_void>(format!("{symbol}\0").as_bytes())
            .map_err(|err| RuntimeError::Other(format!("resolve symbol `{symbol}`: {err}")))?;
        return Ok(*export);
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::HMODULE;
        use windows_sys::Win32::System::LibraryLoader::{
            GetModuleHandleExA, GetProcAddress, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        };

        let mut module: HMODULE = std::ptr::null_mut();
        let anchor = corvid_register_approver as *const ();
        let ok = GetModuleHandleExA(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            anchor.cast(),
            &mut module,
        );
        if ok == 0 || module.is_null() {
            return Err(RuntimeError::Other(
                "resolve current Corvid module handle".to_string(),
            ));
        }
        let symbol_c = CString::new(symbol)
            .map_err(|err| RuntimeError::Other(format!("symbol name contained NUL: {err}")))?;
        let ptr = GetProcAddress(module, symbol_c.as_ptr().cast());
        let Some(ptr) = ptr else {
            return Err(RuntimeError::Other(format!(
                "resolve symbol `{symbol}`: not found"
            )));
        };
        return Ok(ptr as *const c_void);
    }
}

#[cfg(unix)]
fn current_library_unix() -> &'static libloading::os::unix::Library {
    use std::sync::OnceLock;
    static CURRENT_LIB: OnceLock<libloading::os::unix::Library> = OnceLock::new();

    CURRENT_LIB.get_or_init(|| {
        // Prefer opening the cdylib by the path that `dladdr`
        // resolves for an in-crate anchor. This is the path that
        // works when we're hosted in another process (e.g.
        // `python3 ctypes.CDLL("classify.so")`).
        if let Ok(path) = current_library_path() {
            // RTLD_NOW = 0x2 on every glibc + musl + macOS we
            // ship to. libloading doesn't expose the constant
            // through `os::unix`, so we pass the raw value.
            const RTLD_NOW: std::os::raw::c_int = 2;
            if let Ok(lib) =
                unsafe { libloading::os::unix::Library::open(Some(&path), RTLD_NOW) }
            {
                return lib;
            }
        }
        // Fallback: `dlopen(NULL)` — the main-program handle.
        // Works for native-binary builds where the agent symbols
        // are dynamically exported from the binary.
        libloading::os::unix::Library::this()
    })
}

#[no_mangle]
pub unsafe extern "C" fn corvid_abi_descriptor_json(out_len: *mut usize) -> *const c_char {
    match descriptor_json_ptr() {
        Ok((ptr, len)) => {
            if !out_len.is_null() {
                *out_len = len;
            }
            ptr
        }
        Err(_) => {
            if !out_len.is_null() {
                *out_len = 0;
            }
            ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn corvid_abi_descriptor_hash(out_hash: *mut u8) {
    if out_hash.is_null() {
        return;
    }
    if let Ok(hash) = descriptor_hash() {
        unsafe {
            ptr::copy_nonoverlapping(hash.as_ptr(), out_hash, hash.len());
        }
    }
}

#[no_mangle]
pub extern "C" fn corvid_abi_verify(expected: *const u8) -> i32 {
    if expected.is_null() {
        return 0;
    }
    let mut expected_hash = [0u8; 32];
    unsafe {
        ptr::copy_nonoverlapping(expected, expected_hash.as_mut_ptr(), expected_hash.len());
    }
    match crate::catalog::verify_hash(&expected_hash) {
        Ok(true) => 1,
        Ok(false) | Err(_) => 0,
    }
}
