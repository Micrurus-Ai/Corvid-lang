//! `Response<Body>` accessors that need the runtime (native dispatch N4).
//!
//! Native code holds a response as a record whose `headers` field is a
//! `List<List<String>>` of name and value pairs. `status()` and `body()`
//! are field reads in compiled code; `header(name)` is answered here by
//! [`crate::connectors::response_header`], the lookup the interpreter
//! uses, so both tiers find the same header.

use super::native_scalar::slot_string;
use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;

extern "C" {
    fn corvid_list_slots(payload: u64) -> *const u64;
}

/// The items of a native list.
///
/// # Safety
/// `list` must be a live native list.
unsafe fn list_items(list: u64) -> Vec<u64> {
    let length = unsafe { *(list as *const i64) }.max(0) as usize;
    let slots = unsafe { corvid_list_slots(list) };
    (0..length).map(|index| unsafe { *slots.add(index) }).collect()
}

/// `response.header(name)` as a native `Option<String>`: a new string
/// holding the value, or null for `None`.
///
/// # Safety
/// `headers` must be a live native `List<List<String>>` and `name` a
/// live Corvid string.
#[no_mangle]
pub unsafe extern "C" fn corvid_response_header(headers: u64, name: CorvidString) -> u64 {
    let wanted = unsafe { borrow_corvid_string(&name) };
    let pairs: Vec<(CorvidString, CorvidString)> = unsafe { list_items(headers) }
        .into_iter()
        .filter_map(|pair| match unsafe { list_items(pair) }.as_slice() {
            [name, value] => Some(unsafe { (slot_string(*name), slot_string(*value)) }),
            _ => None,
        })
        .collect();
    let texts = pairs.iter().map(|(name, value)| unsafe { (borrow_corvid_string(name), borrow_corvid_string(value)) });
    match crate::connectors::response_header(texts, wanted) {
        Some(value) => {
            let value = string_from_rust(value.to_string());
            let bits = unsafe { std::mem::transmute::<CorvidString, *const u8>(value) } as u64;
            bits
        }
        None => 0,
    }
}
