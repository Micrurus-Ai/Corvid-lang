//! The scalars native code holds as their canonical text (B06, B04), and
//! the vocabulary their operation tables share.
//!
//! `Decimal`, `Duration`, `Instant`, `TimeZone`, `ZonedTime`,
//! `Currency`, `Locale` and `Bytes` are each a refcounted String in one
//! 8-byte slot, holding the exact text of the value's JSON form
//! (`corvid_public_json::text_scalar`); a `Bytes` value is its unpadded
//! base64url text. That reuses string reference counting, list and
//! record slots, option and result payloads, printing and JSON
//! unchanged. Every operator and method is a call into the runtime,
//! named by `decimal_methods`, `time_methods`, `currency_methods` and
//! `bytes_methods`; none compares the texts as strings, so `==` on
//! `Bytes` stays constant-time.

use corvid_types::Type;

/// Whether native code holds `ty` as its canonical text. The public
/// JSON crate's list of text-scalar types is the one list.
pub fn is_text_scalar(ty: &Type) -> bool {
    corvid_public_json::text_scalar::name(ty).is_some()
}

/// `ty` without the compile-time `Tainted` and `Grounded` wrappers,
/// which share the inner type's native representation.
pub fn scalar_of(ty: &Type) -> &Type {
    match ty {
        Type::Tainted(inner) | Type::Grounded(inner) => scalar_of(inner),
        other => other,
    }
}

/// How a binary operator's call result becomes the expression's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryReading {
    /// The call returns the result itself.
    Value,
    /// The call returns -1, 0 or 1, compared with zero by the operator.
    Order,
    /// The call returns 1 for equal and 0 otherwise.
    Equality,
}

/// The runtime call behind a binary operator on text-held scalars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarBinary<Operation> {
    pub operation: Operation,
    /// The call takes the right operand first (`n * d` calls `d * n`).
    pub swapped: bool,
    pub reading: BinaryReading,
}

/// How native code runs a builtin method of a text-held scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeScalarMethod<Operation> {
    /// One call into the runtime.
    Call(Operation),
    /// `to_string()`: the value already is its canonical text.
    CanonicalText,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_b06_types_and_bytes_are_text_scalars() {
        for ty in [Type::Decimal, Type::Duration, Type::Instant, Type::TimeZone, Type::ZonedTime, Type::Currency, Type::Locale, Type::Bytes] {
            assert!(is_text_scalar(&ty), "{}", ty.display_name());
        }
        assert!(!is_text_scalar(&Type::String));
        assert!(!is_text_scalar(&Type::Secret));
        assert_eq!(scalar_of(&Type::Grounded(Box::new(Type::Tainted(Box::new(Type::Instant))))), &Type::Instant);
    }
}
