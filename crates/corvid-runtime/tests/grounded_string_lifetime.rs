//! Exercise collection in a dedicated process because the native heap is global.
use corvid_runtime::abi::CorvidString;
use corvid_runtime::catalog_c_api::{
    corvid_grounded_attest_string, corvid_grounded_capture_string_handle,
    corvid_grounded_confidence, corvid_grounded_release,
};
use corvid_runtime::ffi_bridge::{release_string, string_from_rust};
use corvid_runtime::grounded_handles::register_handle_for_pointer;

extern "C" {
    fn corvid_gc_from_roots(roots: *mut *mut u8, count: usize);
    fn corvid_retain(value: CorvidString);
}

#[test]
fn attestation_lives_until_capture_or_descriptor_reclamation() {
    let source = string_from_rust("catalog".into());
    let text = || string_from_rust("entry".into());
    unsafe {
        let released = corvid_grounded_attest_string(text(), source, 0.8);
        // CorvidString is a transparent descriptor pointer. Query the store by
        // integer key after reclamation without dereferencing that address.
        let key = std::mem::transmute::<CorvidString, usize>(released);
        release_string(released);
        assert_eq!(register_handle_for_pointer(key), 0);

        let retained = corvid_grounded_attest_string(text(), source, 0.7);
        corvid_retain(retained);
        release_string(retained);
        let handle = corvid_grounded_capture_string_handle(retained);
        assert_ne!(handle, 0);
        release_string(retained);

        let swept = corvid_grounded_attest_string(text(), source, 0.6);
        let swept_key = std::mem::transmute::<CorvidString, usize>(swept);
        release_string(source);
        corvid_gc_from_roots(std::ptr::null_mut(), 0);
        assert_eq!(register_handle_for_pointer(swept_key), 0);
        assert_eq!(corvid_grounded_confidence(handle), 0.7);
        corvid_grounded_release(handle);
    }
}
