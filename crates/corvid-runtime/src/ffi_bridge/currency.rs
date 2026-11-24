//! Native entry points for `Currency`, `Locale` and
//! `Decimal.format_currency` (B06).
//!
//! Native code holds a `Currency` as a Corvid String containing its
//! ISO 4217 code and a `Locale` as one containing its canonical BCP 47
//! tag, the texts their JSON forms carry. Every method is one call
//! here into `corvid_currency`, the code and CLDR data the interpreter
//! uses. Formatting never rounds: an amount with more digits than the
//! currency shows stops the program with the interpreter's message.
//!
//! Operands are borrowed; a returned String is a fresh +1 the caller owns.

use super::native_scalar::{canonical, runtime_trap, slot_string, write_parse_result};
use super::strings::{borrow_corvid_string, string_from_rust};
use crate::abi::CorvidString;
use corvid_currency::{Currency, Locale};

unsafe fn currency_of(value: &CorvidString) -> Currency {
    unsafe { canonical(value, "Currency", Currency::parse) }
}

unsafe fn locale_of(value: &CorvidString) -> Locale {
    unsafe { canonical(value, "Locale", Locale::parse) }
}

/// `amount.format_currency(currency, locale)`.
///
/// # Safety
/// The operands must be a live native Decimal, Currency and Locale string.
#[no_mangle]
pub unsafe extern "C" fn corvid_decimal_format_currency(
    amount: CorvidString,
    currency: CorvidString,
    locale: CorvidString,
) -> CorvidString {
    let (amount, currency, locale) =
        unsafe { (super::decimal::decimal_of(&amount), currency_of(&currency), locale_of(&locale)) };
    match corvid_currency::format(&amount, currency, &locale) {
        Ok(text) => string_from_rust(text),
        Err(error) => runtime_trap(&error),
    }
}

/// `currency.minor_digits()`.
///
/// # Safety
/// The operand must be a live native Currency string.
#[no_mangle]
pub unsafe extern "C" fn corvid_currency_minor_digits(currency: CorvidString) -> i64 {
    i64::from(unsafe { currency_of(&currency) }.minor_digits())
}

/// `a == b` between two currencies: 1 or 0.
///
/// # Safety
/// Both operands must be live native Currency strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_currency_equal(left: CorvidString, right: CorvidString) -> i64 {
    i64::from(unsafe { currency_of(&left) == currency_of(&right) })
}

/// `a == b` between two locales: 1 or 0.
///
/// # Safety
/// Both operands must be live native Locale strings.
#[no_mangle]
pub unsafe extern "C" fn corvid_locale_equal(left: CorvidString, right: CorvidString) -> i64 {
    i64::from(unsafe { locale_of(&left) == locale_of(&right) })
}

/// `text.parse_currency()` into a native `Result<Currency, String>`.
///
/// # Safety
/// The receiver must remain a live Corvid string for this call, and
/// `payload` must point to a writable native Result payload slot.
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_currency(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    unsafe { write_parse_result(Currency::parse_user_text(text).map(|c| c.to_string()), payload) }
}

/// `text.parse_locale()` into a native `Result<Locale, String>`.
///
/// # Safety
/// As for [`corvid_string_parse_currency`].
#[no_mangle]
pub unsafe extern "C" fn corvid_string_parse_locale(text: CorvidString, payload: *mut CorvidString) -> i64 {
    let text = unsafe { borrow_corvid_string(&text) };
    unsafe { write_parse_result(Locale::parse_user_text(text).map(|l| l.to_string()), payload) }
}

/// Whether two native Currency slots hold the same currency, for
/// structural equality.
///
/// # Safety
/// Both slots must hold live native Currency strings.
pub(crate) unsafe fn currency_slots_equal(left: u64, right: u64) -> bool {
    unsafe { currency_of(&slot_string(left)) == currency_of(&slot_string(right)) }
}

/// Whether two native Locale slots hold the same locale.
///
/// # Safety
/// Both slots must hold live native Locale strings.
pub(crate) unsafe fn locale_slots_equal(left: u64, right: u64) -> bool {
    unsafe { locale_of(&slot_string(left)) == locale_of(&slot_string(right)) }
}
