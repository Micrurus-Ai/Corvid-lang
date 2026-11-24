//! Native entry points for `Decimal` (B06).
//!
//! Native code holds a `Decimal` as a Corvid String containing its
//! exact canonical text, the text `Decimal::to_string` writes and the
//! JSON form carries (`"2.50"`). Every operator and method is one call
//! here: the entry point parses its operands with `corvid_decimal`,
//! computes with `corvid_decimal`, the code the interpreter calls, and
//! returns the canonical text of the result. Both tiers therefore share
//! every digit rule and every error message.
//!
//! Operands are borrowed; a returned String is a fresh +1 the caller
//! owns. A `DecimalError` stops the program the way the interpreter's
//! arithmetic error does, with the same message.

use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;
use corvid_decimal::{Decimal, DecimalError, RoundingMode};

/// Stop the program with a decimal arithmetic error, the native form of
/// the interpreter's `InterpErrorKind::Arithmetic` trap.
fn trap(error: DecimalError) -> ! {
    eprintln!("corvid: runtime error: {error}");
    std::process::exit(1)
}

fn checked<T>(result: Result<T, DecimalError>) -> T {
    result.unwrap_or_else(|error| trap(error))
}

/// The value a native Decimal slot holds.
///
/// # Safety
/// `value` must be a live Corvid string written by this module or by a
/// compiler-emitted Decimal literal.
pub(crate) unsafe fn decimal_of(value: &CorvidString) -> Decimal {
    let text = unsafe { borrow_corvid_string(value) };
    Decimal::parse(text).unwrap_or_else(|error| {
        // Every native Decimal is written as canonical text by the
        // compiler or this module, so this is a compiler bug.
        eprintln!("corvid: internal error: a native Decimal holds `{text}`, which is not decimal text: {error}");
        std::process::abort()
    })
}

fn text_of(value: Decimal) -> CorvidString {
    string_from_rust(value.to_string())
}

macro_rules! decimal_binary {
    ($symbol:ident, $method:ident) => {
        /// # Safety
        /// Both operands must be live native Decimal strings.
        #[no_mangle]
        pub unsafe extern "C" fn $symbol(left: CorvidString, right: CorvidString) -> CorvidString {
            let (a, b) = unsafe { (decimal_of(&left), decimal_of(&right)) };
            text_of(checked(a.$method(&b)))
        }
    };
}

decimal_binary!(corvid_decimal_add, checked_add);
decimal_binary!(corvid_decimal_sub, checked_sub);
decimal_binary!(corvid_decimal_mul, checked_mul);
decimal_binary!(corvid_decimal_rem, checked_rem);

/// `-d`.
///
/// # Safety
/// The operand must be a live native Decimal string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_negate(value: CorvidString) -> CorvidString {
    text_of(unsafe { decimal_of(&value) }.negate())
}

/// `d.abs()`.
///
/// # Safety
/// The operand must be a live native Decimal string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_abs(value: CorvidString) -> CorvidString {
    text_of(unsafe { decimal_of(&value) }.abs())
}

/// Numeric order: -1, 0 or 1. Equal values with different scales
/// (`1.0`, `1.00`) compare as 0.
///
/// # Safety
/// Both operands must be live native Decimal strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_compare(left: CorvidString, right: CorvidString) -> i64 {
    let (a, b) = unsafe { (decimal_of(&left), decimal_of(&right)) };
    a.cmp(&b) as i64
}

/// `n.to_decimal()`, and the exact widening of an `Int` operand.
#[no_mangle]
pub extern "C" fn corvid_decimal_from_int(value: i64) -> CorvidString {
    text_of(Decimal::from_int(value))
}

/// `x.to_decimal()` on a `Float`; traps on NaN and infinity.
#[no_mangle]
pub extern "C" fn corvid_decimal_from_float(value: f64) -> CorvidString {
    text_of(checked(Decimal::from_float(value)))
}

/// `d.to_float()`.
///
/// # Safety
/// The operand must be a live native Decimal string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_to_float(value: CorvidString) -> f64 {
    unsafe { decimal_of(&value) }.to_float()
}

/// `d.to_int_truncated()`; traps outside the `Int` range.
///
/// # Safety
/// The operand must be a live native Decimal string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_to_int_truncated(value: CorvidString) -> i64 {
    checked(unsafe { decimal_of(&value) }.to_int_truncated())
}

/// `d.scale()`.
///
/// # Safety
/// The operand must be a live native Decimal string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_scale(value: CorvidString) -> i64 {
    i64::from(unsafe { decimal_of(&value) }.scale())
}

macro_rules! decimal_rounding {
    ($round:ident, $divide:ident, $mode:expr) => {
        /// `d.round_<mode>(places)`.
        ///
        /// # Safety
        /// The operand must be a live native Decimal string.
        #[no_mangle]
        pub unsafe extern "C" fn $round(value: CorvidString, places: i64) -> CorvidString {
            text_of(checked(unsafe { decimal_of(&value) }.round(places, $mode)))
        }

        /// `(a / b).round_<mode>(places)`, rounded once.
        ///
        /// # Safety
        /// Both operands must be live native Decimal strings.
        #[no_mangle]
        pub unsafe extern "C" fn $divide(left: CorvidString, right: CorvidString, places: i64) -> CorvidString {
            let (a, b) = unsafe { (decimal_of(&left), decimal_of(&right)) };
            text_of(checked(a.divide(&b, places, $mode)))
        }
    };
}

decimal_rounding!(corvid_decimal_round_half_even, corvid_decimal_divide_round_half_even, RoundingMode::HalfEven);
decimal_rounding!(corvid_decimal_round_half_up, corvid_decimal_divide_round_half_up, RoundingMode::HalfUp);
decimal_rounding!(corvid_decimal_round_toward_zero, corvid_decimal_divide_round_toward_zero, RoundingMode::TowardZero);
decimal_rounding!(
    corvid_decimal_round_away_from_zero,
    corvid_decimal_divide_round_away_from_zero,
    RoundingMode::AwayFromZero
);
decimal_rounding!(corvid_decimal_round_floor, corvid_decimal_divide_round_floor, RoundingMode::Floor);
decimal_rounding!(corvid_decimal_round_ceiling, corvid_decimal_divide_round_ceiling, RoundingMode::Ceiling);

/// `text.parse_decimal()`: writes the Decimal or the error message into
/// a native `Result<Decimal, String>` payload slot and returns its tag,
/// 0 for Ok and 1 for Err, as `corvid_string_parse_int` does.
/// `Decimal::parse_user_text` writes the message both tiers return.
///
/// # Safety
/// The receiver must remain a live Corvid string for this call, and
/// `payload` must point to a writable native Result payload slot; the
/// written String is owned by the Result.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_decimal(value: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&value) };
    let (tag, written) = match Decimal::parse_user_text(text) {
        Ok(decimal) => (0, decimal.to_string()),
        Err(message) => (1, message),
    };
    // SAFETY: the caller passes an aligned, writable Result payload slot.
    unsafe { payload.write(string_from_rust(written)) };
    tag
}

/// Whether two native Decimal slots hold the same number, for
/// structural equality of records, lists and options: `1.0` equals
/// `1.00`, as `Decimal`'s own equality says.
///
/// # Safety
/// Both slots must hold live native Decimal strings.
pub(crate) unsafe fn slots_equal(left: u64, right: u64) -> bool {
    // SAFETY: `CorvidString` is `#[repr(transparent)]` over the
    // descriptor pointer a native slot holds, and has no destructor.
    let (left, right): (CorvidString, CorvidString) =
        unsafe { (std::mem::transmute(left as *const u8), std::mem::transmute(right as *const u8)) };
    unsafe { decimal_of(&left) == decimal_of(&right) }
}
