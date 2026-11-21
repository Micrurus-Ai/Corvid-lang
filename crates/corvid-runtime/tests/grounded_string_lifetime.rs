//! Exercise collection in a dedicated process because the native heap is global.
use corvid_runtime::abi::CorvidString;
use corvid_runtime::catalog_c_api::{
    corvid_grounded_attest_string, corvid_grounded_capture_string_handle,
    corvid_grounded_confidence, corvid_grounded_release, corvid_grounded_string_concat,
};
use corvid_runtime::ffi_bridge::{release_string, string_from_rust};
use corvid_runtime::grounded_handles::register_handle_for_pointer;

extern "C" {
    fn corvid_gc_from_roots(roots: *mut *mut u8, count: usize);
    fn corvid_retain(value: CorvidString);
    fn corvid_live_object_count() -> i64;
}

// The collector and reference-counted heap are process-global.
static HEAP: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn attestation_lives_until_capture_or_descriptor_reclamation() {
    let _heap = HEAP.lock().unwrap();
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

#[test]
fn grounded_concatenation_preserves_operand_sources_and_confidence() {
    use corvid_runtime::abi::FromCorvidAbi;
    use corvid_runtime::grounded_handles::sources_for_handle;

    unsafe {
        let _heap = HEAP.lock().unwrap();
        let baseline = corvid_live_object_count();
        let catalog = string_from_rust("catalog".into());
        let inventory = string_from_rust("inventory".into());
        let left = corvid_grounded_attest_string(string_from_rust("left".into()), catalog, 0.8);
        let right = corvid_grounded_attest_string(string_from_rust("?".into()), inventory, 0.6);
        let space = string_from_rust(" ".into());

        // Explicitly unwrapped operands may still alias an attested descriptor.
        let only_left = corvid_grounded_string_concat(left, right, 1);
        let only_right = corvid_grounded_string_concat(left, right, 2);
        let joined = corvid_grounded_string_concat(only_left, only_right, 3);
        let spaced = corvid_grounded_string_concat(space, joined, 2);
        release_string(left);
        release_string(right);
        assert_eq!(String::from_corvid_abi(spaced), " left?left?");

        for (value, sources, confidence) in [
            (only_left, vec!["catalog"], 0.8),
            (only_right, vec!["inventory"], 0.6),
            (joined, vec!["catalog", "inventory"], 0.6),
            (spaced, vec!["catalog", "inventory"], 0.6),
        ] {
            let handle = corvid_grounded_capture_string_handle(value);
            release_string(value);
            assert_eq!(sources_for_handle(handle).unwrap(), sources);
            assert_eq!(corvid_grounded_confidence(handle), confidence);
            corvid_grounded_release(handle);
        }

        for mask in [0, 1, 2, 3, 4] {
            let missing = corvid_grounded_string_concat(space, space, mask);
            assert_eq!(std::mem::transmute::<CorvidString, usize>(missing), 0);
        }
        release_string(space);
        release_string(catalog);
        release_string(inventory);
        assert_eq!(corvid_live_object_count(), baseline);
    }
}
