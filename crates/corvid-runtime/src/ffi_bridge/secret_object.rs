//! A `Secret` in native code (native dispatch N3).
//!
//! A native secret is a reference-counted heap object of the C
//! allocator's `corvid_typeinfo_Secret`. Its 8-byte payload holds a
//! pointer to a boxed [`corvid_crypto::Secret`], the value the
//! interpreter holds, so comparison, HMAC keys, password checks and key
//! decoding are one implementation on both tiers. Compiled code retains,
//! releases and passes the object and never reads it; this module is the
//! only code that opens the box. The box is dropped when the allocator
//! frees the object (`corvid_secret_drop_payload`).

use std::ffi::c_void;

use corvid_crypto::Secret;

extern "C" {
    fn corvid_alloc_typed(payload_bytes: i64, typeinfo: *const c_void) -> *mut u8;
    static corvid_typeinfo_Secret: u8;
}

/// A new native secret object holding `secret`, with one reference.
pub(crate) fn new_secret_object(secret: Secret) -> *mut c_void {
    let boxed = Box::into_raw(Box::new(secret));
    let payload = unsafe { corvid_alloc_typed(8, std::ptr::addr_of!(corvid_typeinfo_Secret).cast()) };
    unsafe { payload.cast::<*mut Secret>().write(boxed) };
    payload.cast()
}

/// The secret a native secret object holds.
///
/// # Safety
/// `object` must be a live native secret object.
pub(crate) unsafe fn secret_of<'a>(object: *const c_void) -> &'a Secret {
    let boxed = unsafe { object.cast::<*const Secret>().read() };
    unsafe { &*boxed }
}

/// Drops the boxed secret of an object the allocator is freeing.
///
/// # Safety
/// Called once per object, by the allocator, with the object's payload.
#[no_mangle]
pub unsafe extern "C" fn corvid_secret_drop_payload(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let slot = payload.cast::<*mut Secret>();
    let boxed = unsafe { slot.read() };
    if !boxed.is_null() {
        unsafe { slot.write(std::ptr::null_mut()) };
        drop(unsafe { Box::from_raw(boxed) });
    }
}

/// Two slots holding native secret objects, compared as `==` compares
/// them: the structural-equality walker's entry for a `Secret` field,
/// item or payload.
///
/// # Safety
/// Both slots must hold live native secret objects.
pub(crate) unsafe fn slots_equal(left: u64, right: u64) -> bool {
    unsafe { corvid_secret_eq(left as *const c_void, right as *const c_void) == 1 }
}

/// `a == b` on two secrets: 1 when equal, 0 otherwise, compared in
/// constant time by [`Secret::constant_time_eq`], as the interpreter
/// compares them.
///
/// # Safety
/// Both must be live native secret objects.
#[no_mangle]
pub unsafe extern "C" fn corvid_secret_eq(a: *const c_void, b: *const c_void) -> i64 {
    i64::from(unsafe { secret_of(a) }.constant_time_eq(unsafe { secret_of(b) }))
}
