//! Grounded and observation C bridge exports.

use crate::abi::CorvidString;
use crate::catalog::CorvidCallStatus;
use crate::ffi_bridge::read_corvid_string;
use crate::grounded_handles;
use crate::observation_handles;
use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

thread_local! {
    static TRANSIENT_STRINGS: RefCell<Vec<CString>> = RefCell::new(Vec::new());
}

pub(super) fn reset_transients() {
    TRANSIENT_STRINGS.with(|arena| arena.borrow_mut().clear());
}

pub(super) fn stash_transient(text: &str) -> *const c_char {
    if text.is_empty() {
        return ptr::null();
    }
    TRANSIENT_STRINGS.with(|arena| {
        let mut arena = arena.borrow_mut();
        let c = CString::new(text).expect("transient text contained NUL");
        let ptr = c.as_ptr();
        arena.push(c);
        ptr
    })
}

pub(super) unsafe fn read_c_string(ptr: *const c_char) -> Result<String, CorvidCallStatus> {
    if ptr.is_null() {
        return Err(CorvidCallStatus::BadArgs);
    }
    CStr::from_ptr(ptr)
        .to_str()
        .map(|value| value.to_owned())
        .map_err(|_| CorvidCallStatus::BadArgs)
}

fn grounded_source_pointers(handle: u64) -> Option<Vec<*const c_char>> {
    let sources = grounded_handles::sources_for_handle(handle)?;
    reset_transients();
    Some(
        sources
            .into_iter()
            .map(|source| stash_transient(&source))
            .collect(),
    )
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_sources(
    handle: u64,
    out: *mut *const c_char,
    capacity: usize,
) -> i32 {
    let Some(sources) = grounded_source_pointers(handle) else {
        return -1;
    };
    if !out.is_null() {
        let count = sources.len().min(capacity);
        ptr::copy_nonoverlapping(sources.as_ptr(), out, count);
    }
    sources.len() as i32
}

#[no_mangle]
pub extern "C" fn corvid_grounded_confidence(handle: u64) -> f64 {
    grounded_handles::confidence_for_handle(handle).unwrap_or(f64::NAN)
}

#[no_mangle]
pub extern "C" fn corvid_grounded_release(handle: u64) {
    let released = grounded_handles::release_handle(handle);
    if cfg!(debug_assertions) && handle != grounded_handles::NULL_GROUNDED_HANDLE && !released {
        eprintln!("warning: grounded handle {handle} was already released or never existed");
    }
}

#[no_mangle]
pub extern "C" fn corvid_observation_cost_usd(handle: u64) -> f64 {
    observation_handles::cost_usd_for_handle(handle).unwrap_or(f64::NAN)
}

#[no_mangle]
pub extern "C" fn corvid_observation_latency_ms(handle: u64) -> u64 {
    observation_handles::latency_ms_for_handle(handle).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn corvid_observation_tokens_in(handle: u64) -> u64 {
    observation_handles::tokens_in_for_handle(handle).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn corvid_observation_tokens_out(handle: u64) -> u64 {
    observation_handles::tokens_out_for_handle(handle).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn corvid_observation_exceeded_bound(handle: u64) -> bool {
    observation_handles::exceeded_bound_for_handle(handle).unwrap_or(false)
}

#[no_mangle]
pub extern "C" fn corvid_observation_release(handle: u64) {
    let released = observation_handles::release_handle(handle);
    if cfg!(debug_assertions) && handle != observation_handles::NULL_OBSERVATION_HANDLE && !released
    {
        eprintln!("warning: observation handle {handle} was already released or never existed");
    }
}

#[no_mangle]
pub extern "C" fn corvid_begin_direct_observation(declared_bound_usd: f64) {
    let declared_bound = if declared_bound_usd.is_finite() {
        Some(declared_bound_usd)
    } else {
        None
    };
    observation_handles::begin_direct_observation(declared_bound);
}

#[no_mangle]
pub unsafe extern "C" fn corvid_finish_direct_observation(out_handle: *mut u64) {
    if out_handle.is_null() {
        let _ = observation_handles::finish_direct_observation();
        return;
    }
    *out_handle = observation_handles::finish_direct_observation();
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_attest_int(
    value: i64,
    source_name: CorvidString,
    confidence: f64,
) -> i64 {
    let source = read_corvid_string(source_name);
    let chain =
        crate::provenance::ProvenanceChain::with_retrieval(&source, crate::tracing::now_ms());
    grounded_handles::set_last_scalar_attestation(grounded_handles::make_attestation(
        chain, confidence,
    ));
    value
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_attest_float(
    value: f64,
    source_name: CorvidString,
    confidence: f64,
) -> f64 {
    let source = read_corvid_string(source_name);
    let chain =
        crate::provenance::ProvenanceChain::with_retrieval(&source, crate::tracing::now_ms());
    grounded_handles::set_last_scalar_attestation(grounded_handles::make_attestation(
        chain, confidence,
    ));
    value
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_attest_bool(
    value: bool,
    source_name: CorvidString,
    confidence: f64,
) -> bool {
    let source = read_corvid_string(source_name);
    let chain =
        crate::provenance::ProvenanceChain::with_retrieval(&source, crate::tracing::now_ms());
    grounded_handles::set_last_scalar_attestation(grounded_handles::make_attestation(
        chain, confidence,
    ));
    value
}

extern "C" {
    fn corvid_init_grounded_string_cleanup(forget: extern "C" fn(*mut std::ffi::c_void));
    fn corvid_string_concat(left: CorvidString, right: CorvidString) -> CorvidString;
    fn corvid_track_grounded_string(value: CorvidString);
}

extern "C" fn forget_grounded_pointer(payload: *mut std::ffi::c_void) {
    grounded_handles::forget_pointer_attestation(payload as usize);
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_attest_string(
    value: CorvidString,
    source_name: CorvidString,
    confidence: f64,
) -> CorvidString {
    let source = read_corvid_string(source_name);
    let chain =
        crate::provenance::ProvenanceChain::with_retrieval(&source, crate::tracing::now_ms());
    attach_string_attestation(value, grounded_handles::make_attestation(chain, confidence));
    value
}

unsafe fn attach_string_attestation(
    value: CorvidString,
    attestation: std::sync::Arc<grounded_handles::GroundedAttestation>,
) {
    static INIT_CLEANUP: std::sync::Once = std::sync::Once::new();
    INIT_CLEANUP.call_once(|| corvid_init_grounded_string_cleanup(forget_grounded_pointer));
    corvid_track_grounded_string(value);
    grounded_handles::attach_pointer_attestation(value.descriptor_key(), attestation);
}

/// Join two borrowed strings. Bit 0 identifies a grounded left operand and
/// bit 1 a grounded right operand. Success returns an owned descriptor.
/// Invalid flags or a missing required attestation return a null descriptor.
///
/// # Safety
/// Both arguments must be live native string descriptors.
#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_string_concat(
    left: CorvidString,
    right: CorvidString,
    grounded_mask: u8,
) -> CorvidString {
    if !(1..=3).contains(&grounded_mask) {
        return CorvidString::null();
    }
    let Some(attestation) = grounded_handles::derive_string_add(
        (grounded_mask & 1 != 0).then_some(left.descriptor_key()),
        (grounded_mask & 2 != 0).then_some(right.descriptor_key()),
    ) else {
        return CorvidString::null();
    };
    let value = corvid_string_concat(left, right);
    attach_string_attestation(value, attestation);
    value
}

/// Attach a `Grounded<Struct>` attestation by the struct's heap
/// pointer. Mirrors `corvid_grounded_attest_string` — a struct
/// allocation has a stable heap pointer that serves as the key in
/// the same `pointer_attestations` map used for string descriptors.
///
/// The codegen-cl prompt-bridge call site (in
/// `crate::lowering::prompt::emit_grounded_prompt_attestation`)
/// invokes this after `corvid_prompt_call_struct` returns the
/// decoded struct pointer, so the attestation is in place before
/// the value flows into the agent body. The agent's later
/// `unwrap g` reads the underlying pointer; any cdylib export
/// path that wants to surface the handle to a non-Corvid host
/// reaches it via `corvid_grounded_capture_struct_handle` (added
/// alongside).
///
/// `value` is `i64` rather than a typed struct ABI because the
/// Cranelift codegen treats every refcounted heap type as a raw
/// pointer at the C-ABI boundary.
#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_attest_struct(
    value: i64,
    source_name: CorvidString,
    confidence: f64,
) -> i64 {
    let source = read_corvid_string(source_name);
    let chain =
        crate::provenance::ProvenanceChain::with_retrieval(&source, crate::tracing::now_ms());
    grounded_handles::attach_pointer_attestation(
        value as usize,
        grounded_handles::make_attestation(chain, confidence),
    );
    value
}

#[no_mangle]
pub extern "C" fn corvid_grounded_capture_scalar_handle() -> u64 {
    grounded_handles::register_handle_for_last_scalar()
}

#[no_mangle]
pub unsafe extern "C" fn corvid_grounded_capture_string_handle(value: CorvidString) -> u64 {
    grounded_handles::register_handle_for_pointer(value.descriptor_key())
}

/// Promote a struct's pointer-keyed attestation into the numeric
/// handle store. Mirrors `corvid_grounded_capture_string_handle`
/// — same map, same lookup, just keyed on the raw struct pointer
/// rather than a string descriptor pointer.
#[no_mangle]
pub extern "C" fn corvid_grounded_capture_struct_handle(value: i64) -> u64 {
    grounded_handles::register_handle_for_pointer(value as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grounded_handles::{
        make_attestation, register_handle_for_last_scalar, release_handle,
        set_last_scalar_attestation,
    };
    use crate::provenance::ProvenanceChain;

    #[test]
    fn derived_handle_reports_nested_retrievals_in_operand_order() {
        let first = ProvenanceChain::with_retrieval("catalog", 1);
        let mut second = ProvenanceChain::with_retrieval("inventory", 2);
        second.add_prompt_transform("summarize", 3);
        let nested = ProvenanceChain::derived("add", vec![first.clone(), second], 0);
        let chain = ProvenanceChain::derived("add", vec![ProvenanceChain::new(), nested, first], 0);
        assert!(chain.has_retrieval());
        assert!(chain.has_source("inventory"));
        assert!(!chain.has_source("absent"));
        set_last_scalar_attestation(make_attestation(chain, 0.75));
        let handle = register_handle_for_last_scalar();
        let mut names = [std::ptr::null(); 2];
        // SAFETY: the output array has room for both pointers, which the bridge
        // keeps valid until the next transient-string call on this thread.
        unsafe {
            assert_eq!(
                corvid_grounded_sources(handle, names.as_mut_ptr(), names.len()),
                2
            );
            assert_eq!(
                names.map(|name| CStr::from_ptr(name).to_bytes()),
                [b"catalog".as_slice(), b"inventory"]
            );
        }
        assert!(release_handle(handle));
    }
}
