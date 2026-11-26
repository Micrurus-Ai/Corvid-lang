//! Native entry points for writing numbers as text, through
//! [`crate::number_text`], the forms the interpreter writes.

use super::strings::string_from_rust;
use crate::abi::CorvidString;

/// Copies `text` into the caller's buffer, truncated to `capacity`, and
/// returns how many bytes it wrote.
unsafe fn write_into(text: &str, buffer: *mut u8, capacity: usize) -> usize {
    let length = text.len().min(capacity);
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buffer, length) };
    length
}

/// A `Float` as JSON number text (`null` when not finite), into `buffer`.
///
/// # Safety
/// `buffer` must be writable for `capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn corvid_float_json_text(value: f64, buffer: *mut u8, capacity: usize) -> usize {
    unsafe { write_into(&crate::number_text::float_json(value), buffer, capacity) }
}

/// `float.to_string()`, and a `Float` result as `corvid run` prints it.
#[no_mangle]
pub extern "C" fn corvid_float_to_string(value: f64) -> CorvidString {
    string_from_rust(crate::number_text::float_to_string(value))
}

/// `int.to_string()`.
#[no_mangle]
pub extern "C" fn corvid_int_to_string(value: i64) -> CorvidString {
    string_from_rust(value.to_string())
}

/// `bool.to_string()`: `true` or `false`.
#[no_mangle]
pub extern "C" fn corvid_bool_to_string(value: i64) -> CorvidString {
    string_from_rust(if value != 0 { "true" } else { "false" }.to_string())
}
