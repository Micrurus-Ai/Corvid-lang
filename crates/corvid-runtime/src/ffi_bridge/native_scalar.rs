//! What every text-held native scalar shares (B06): reading a slot
//! back as its value, and stopping the program on a runtime error.
//!
//! Native code holds `Decimal`, `Duration`, `Instant`, `TimeZone`,
//! `ZonedTime`, `Currency` and `Locale` as a Corvid String containing
//! the value's canonical text, the text its JSON form carries. The
//! entry points in `decimal.rs`, `time.rs` and `currency.rs` read their
//! operands through [`canonical`] and stop with [`runtime_trap`] the way
//! the interpreter's `InterpErrorKind::Arithmetic` trap stops a run.

use super::strings::borrow_corvid_string;
use crate::abi::CorvidString;
use std::fmt::Display;

/// Stop the program with a runtime error carrying `message`, the native
/// form of the interpreter's arithmetic trap.
pub(super) fn runtime_trap(message: &dyn Display) -> ! {
    eprintln!("corvid: runtime error: {message}");
    std::process::exit(1)
}

/// The value a native slot of type `what` holds, read with the type's
/// own parser.
///
/// # Safety
/// `value` must be a live Corvid string written by a runtime entry
/// point or a compiler-emitted literal of that type.
pub(super) unsafe fn canonical<T, E: Display>(
    value: &CorvidString,
    what: &str,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> T {
    let text = unsafe { borrow_corvid_string(value) };
    parse(text).unwrap_or_else(|error| {
        // Every native value of these types is canonical text written by
        // the compiler or the runtime, so this is a compiler bug.
        eprintln!("corvid: internal error: a native {what} holds `{text}`, which is not {what} text: {error}");
        std::process::abort()
    })
}

/// A structural-equality slot as the String it holds.
///
/// # Safety
/// `slot` must hold a live native String descriptor pointer.
pub(super) unsafe fn slot_string(slot: u64) -> CorvidString {
    // SAFETY: `CorvidString` is `#[repr(transparent)]` over the
    // descriptor pointer a native slot holds, and has no destructor.
    unsafe { std::mem::transmute(slot as *const u8) }
}

/// Writes a parse outcome into a native `Result<T, String>` payload
/// slot and returns its tag, 0 for Ok and 1 for Err, as
/// `corvid_string_parse_int` does. `Ok` holds the value's canonical
/// text.
///
/// # Safety
/// `payload` must point to a writable native Result payload slot; the
/// written String is owned by the Result.
pub(super) unsafe fn write_parse_result(outcome: Result<String, String>, payload: *mut CorvidString) -> i64 {
    let (tag, written) = match outcome {
        Ok(text) => (0, text),
        Err(message) => (1, message),
    };
    // SAFETY: the caller passes an aligned, writable Result payload slot.
    unsafe { payload.write(super::strings::string_from_rust(written)) };
    tag
}
