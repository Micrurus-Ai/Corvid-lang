//! Native entry points for `Bytes`, hashing, HMAC and signatures (B04).
//!
//! Native code holds a `Bytes` value as a Corvid String containing the
//! unpadded base64url text `corvid_crypto::Bytes::to_base64_url`
//! writes, the text its JSON form carries. Every method is one call
//! here: the entry point decodes its operands with
//! `corvid_crypto::Bytes::from_base64_url`, computes with
//! `corvid_crypto`, the code the interpreter calls, and returns the
//! canonical text of a byte result. Both tiers therefore share every
//! digest, tag, signature check and message.
//!
//! `==` never decodes and never compares the texts bytewise: it is
//! [`corvid_crypto::Bytes::canonical_texts_equal`], a constant-time
//! comparison of the two canonical texts, which are one to one with
//! the bytes. Structural equality of records, lists and options holding
//! byte strings goes through [`slots_equal`], the same comparison.
//!
//! Every entry point is pure and touches no runtime state, so nothing
//! here is traced or replayed, as on the interpreter.
//!
//! Operands are borrowed; a returned String is a fresh +1 the caller owns.

use super::native_scalar::{canonical, runtime_trap, slot_string, write_parse_result};
use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;
use corvid_crypto::{Bytes, HmacKey};

/// The byte string a native `Bytes` slot holds.
///
/// # Safety
/// `value` must be a live Corvid string written by this module or by a
/// native JSON decode.
pub(super) unsafe fn bytes_of(value: &CorvidString) -> Bytes {
    unsafe { canonical(value, "Bytes", Bytes::from_base64_url) }
}

/// The native slot for `bytes`: its canonical text.
pub(super) fn text_of(bytes: &Bytes) -> CorvidString {
    string_from_rust(bytes.to_base64_url())
}

/// Writes a decoded byte string, or the decoder's message, into a
/// native `Result<Bytes, String>` payload slot and returns its tag.
unsafe fn write_decoded(outcome: Result<Bytes, String>, payload: *mut CorvidString) -> i64 {
    unsafe { write_parse_result(outcome.map(|bytes| bytes.to_base64_url()), payload) }
}

/// `text.utf8()`.
///
/// # Safety
/// The receiver must be a live Corvid string.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_utf8(text: CorvidString) -> CorvidString {
    text_of(&Bytes::from_utf8_text(unsafe { borrow_corvid_string(&text) }))
}

/// `bytes.to_utf8()` into a native `Result<String, String>`.
///
/// # Safety
/// The receiver must be a live native Bytes string, and `payload` must
/// point to a writable native Result payload slot.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_to_utf8(bytes: CorvidString, payload: *mut CorvidString) -> i64 {
    unsafe { write_parse_result(bytes_of(&bytes).to_utf8(), payload) }
}

/// `bytes.length()`. A length past the `Int` range stops the program
/// with the interpreter's message.
///
/// # Safety
/// The receiver must be a live native Bytes string.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_length(bytes: CorvidString) -> i64 {
    i64::try_from(unsafe { bytes_of(&bytes) }.len())
        .unwrap_or_else(|_| runtime_trap(&"a byte string's length exceeds the Int range"))
}

/// `bytes.to_hex()`.
///
/// # Safety
/// The receiver must be a live native Bytes string.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_to_hex(bytes: CorvidString) -> CorvidString {
    string_from_rust(unsafe { bytes_of(&bytes) }.to_hex())
}

/// `bytes.to_base64()`.
///
/// # Safety
/// The receiver must be a live native Bytes string.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_to_base64(bytes: CorvidString) -> CorvidString {
    string_from_rust(unsafe { bytes_of(&bytes) }.to_base64())
}

macro_rules! bytes_decoder {
    ($symbol:ident, $decode:ident, $call:literal) => {
        #[doc = concat!("`", $call, "` into a native `Result<Bytes, String>`.")]
        ///
        /// # Safety
        /// The text must be a live Corvid string, and `payload` must
        /// point to a writable native Result payload slot.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(text: CorvidString, payload: *mut CorvidString) -> i64 {
            let text = unsafe { borrow_corvid_string(&text) };
            unsafe { write_decoded(Bytes::$decode(text), payload) }
        }
    };
}

bytes_decoder!(corvid_bytes_from_hex, from_hex, "Bytes.from_hex(text)");
bytes_decoder!(corvid_bytes_from_base64, from_base64, "Bytes.from_base64(text)");
bytes_decoder!(corvid_bytes_from_base64_url, from_base64_url, "Bytes.from_base64_url(text)");

macro_rules! bytes_digest {
    ($symbol:ident, $digest:ident) => {
        #[doc = concat!("`bytes.", stringify!($digest), "()`.")]
        ///
        /// # Safety
        /// The receiver must be a live native Bytes string.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(bytes: CorvidString) -> CorvidString {
            text_of(&corvid_crypto::$digest(&unsafe { bytes_of(&bytes) }))
        }
    };
}

bytes_digest!(corvid_bytes_sha256, sha256);
bytes_digest!(corvid_bytes_sha384, sha384);
bytes_digest!(corvid_bytes_sha512, sha512);

macro_rules! bytes_hmac {
    ($symbol:ident, $mac:ident) => {
        #[doc = concat!("`message.", stringify!($mac), "(key)` with a `Bytes` key; a `Secret`")]
        /// key goes to `secret_object`'s entry point.
        ///
        /// # Safety
        /// Both operands must be live native Bytes strings.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(message: CorvidString, key: CorvidString) -> CorvidString {
            let (message, key) = unsafe { (bytes_of(&message), bytes_of(&key)) };
            text_of(&corvid_crypto::$mac(&message, HmacKey::Bytes(&key)))
        }
    };
}

bytes_hmac!(corvid_bytes_hmac_sha256, hmac_sha256);
bytes_hmac!(corvid_bytes_hmac_sha512, hmac_sha512);

/// `message.verify_ed25519(signature, public_key)`: 1 or 0, by the
/// strict check the interpreter runs.
///
/// # Safety
/// All three operands must be live native Bytes strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_verify_ed25519(
    message: CorvidString,
    signature: CorvidString,
    public_key: CorvidString,
) -> i64 {
    let (message, signature, public_key) =
        unsafe { (bytes_of(&message), bytes_of(&signature), bytes_of(&public_key)) };
    i64::from(corvid_crypto::verify_ed25519(&message, &signature, &public_key))
}

/// `a == b` between two byte strings: 1 or 0, in constant time.
///
/// # Safety
/// Both operands must be live native Bytes strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_equal(left: CorvidString, right: CorvidString) -> i64 {
    let (left, right) = unsafe { (borrow_corvid_string(&left), borrow_corvid_string(&right)) };
    i64::from(Bytes::canonical_texts_equal(left, right))
}

/// `a + b` between two byte strings: `a`'s bytes then `b`'s, by
/// `corvid_crypto::Bytes::concat`, the interpreter's join.
///
/// # Safety
/// Both operands must be live native Bytes strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_concat(left: CorvidString, right: CorvidString) -> CorvidString {
    let (left, right) = unsafe { (bytes_of(&left), bytes_of(&right)) };
    text_of(&left.concat(&right))
}

/// The text the interpreter prints for a `Bytes` result,
/// `Bytes(<hex>)`, written by the same `Debug` form.
///
/// # Safety
/// The operand must be a live native Bytes string.
#[no_mangle]
pub unsafe extern "C" fn corvid_bytes_display(bytes: CorvidString) -> CorvidString {
    string_from_rust(format!("{:?}", unsafe { bytes_of(&bytes) }))
}

/// Whether two native Bytes slots hold the same bytes, for structural
/// equality: the constant-time comparison `==` uses.
///
/// # Safety
/// Both slots must hold live native Bytes strings.
pub(crate) unsafe fn slots_equal(left: u64, right: u64) -> bool {
    let (left, right) = unsafe { (slot_string(left), slot_string(right)) };
    let (left, right) = unsafe { (borrow_corvid_string(&left), borrow_corvid_string(&right)) };
    Bytes::canonical_texts_equal(left, right)
}
