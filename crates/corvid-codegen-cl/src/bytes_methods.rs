//! The `Bytes` operations native code calls (B04): which runtime symbol
//! runs each method and `==`, and how many words it takes.
//!
//! Native code holds a `Bytes` value as a String containing its
//! unpadded base64url text, the text its JSON form carries. Every
//! operation is one call into `corvid-runtime/src/ffi_bridge/bytes.rs`,
//! which decodes the operands, computes with `corvid-crypto` (the code
//! the interpreter calls) and returns canonical text, an `Int`, a
//! `Bool` as one word, or a Result tag. `==` and `!=` are the runtime's
//! constant-time comparison, never string equality. The import
//! declarations, the lowering, the tier picker and native match arms all
//! read this table, so an operation is native everywhere or nowhere. A
//! misspelled symbol fails to link.
//!
//! The `Secret` methods (`decode_base64_after`, `verify_password`) and
//! an HMAC keyed by a `Secret` are not here: native code has no
//! `Secret` (see the driver's `NotNativeReason::SecretNotNative`).

use crate::text_scalar::{scalar_of, BinaryReading, NativeScalarMethod, ScalarBinary};
use corvid_ast::BinaryOp;
use corvid_types::{BuiltinMethodKind, Type};

/// One runtime entry point for `Bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BytesOperation {
    /// `text.utf8()`, whose receiver is the `String`.
    FromUtf8Text,
    ToUtf8,
    Length,
    ToHex,
    ToBase64,
    /// `Bytes.from_hex(text)`, whose receiver is the `String`.
    FromHex,
    FromBase64,
    FromBase64Url,
    Sha256,
    Sha384,
    Sha512,
    HmacSha256,
    HmacSha512,
    /// 1 when the signature verifies.
    VerifyEd25519,
    /// 1 when two byte strings are equal, compared in constant time.
    Equal,
    /// `Bytes(<hex>)`, the interpreter's printed form, for an entry
    /// agent that returns a byte string.
    Display,
}

impl BytesOperation {
    /// Every operation, for declaring the imports.
    pub fn all() -> Vec<BytesOperation> {
        use BytesOperation::*;
        vec![
            FromUtf8Text,
            ToUtf8,
            Length,
            ToHex,
            ToBase64,
            FromHex,
            FromBase64,
            FromBase64Url,
            Sha256,
            Sha384,
            Sha512,
            HmacSha256,
            HmacSha512,
            VerifyEd25519,
            Equal,
            Display,
        ]
    }

    pub fn symbol(self) -> &'static str {
        use BytesOperation::*;
        match self {
            FromUtf8Text => "corvid_string_utf8",
            ToUtf8 => "corvid_bytes_to_utf8",
            Length => "corvid_bytes_length",
            ToHex => "corvid_bytes_to_hex",
            ToBase64 => "corvid_bytes_to_base64",
            FromHex => "corvid_bytes_from_hex",
            FromBase64 => "corvid_bytes_from_base64",
            FromBase64Url => "corvid_bytes_from_base64_url",
            Sha256 => "corvid_bytes_sha256",
            Sha384 => "corvid_bytes_sha384",
            Sha512 => "corvid_bytes_sha512",
            HmacSha256 => "corvid_bytes_hmac_sha256",
            HmacSha512 => "corvid_bytes_hmac_sha512",
            VerifyEd25519 => "corvid_bytes_verify_ed25519",
            Equal => "corvid_bytes_equal",
            Display => "corvid_bytes_display",
        }
    }

    /// How many one-word parameters the entry point takes; every entry
    /// point returns one word.
    pub fn arity(self) -> usize {
        use BytesOperation::*;
        match self {
            FromUtf8Text | Length | ToHex | ToBase64 | Sha256 | Sha384 | Sha512 | Display => 1,
            ToUtf8 | FromHex | FromBase64 | FromBase64Url | HmacSha256 | HmacSha512 | Equal => 2,
            VerifyEd25519 => 3,
        }
    }

    /// Whether this fills a `Result<T, String>` payload slot and returns its tag.
    pub fn is_parse(self) -> bool {
        matches!(
            self,
            BytesOperation::ToUtf8 | BytesOperation::FromHex | BytesOperation::FromBase64 | BytesOperation::FromBase64Url
        )
    }
}

/// `==` and `!=` between two byte strings.
pub fn bytes_binary(op: BinaryOp, left: &Type, right: &Type) -> Option<ScalarBinary<BytesOperation>> {
    match (op, scalar_of(left), scalar_of(right)) {
        (BinaryOp::Eq | BinaryOp::NotEq, Type::Bytes, Type::Bytes) => {
            Some(ScalarBinary { operation: BytesOperation::Equal, swapped: false, reading: BinaryReading::Equality })
        }
        _ => None,
    }
}

/// Whether either operand is a `Bytes`.
pub fn has_bytes_operand(left: &Type, right: &Type) -> bool {
    [left, right].into_iter().any(|ty| matches!(scalar_of(ty), Type::Bytes))
}

/// The native form of a `Bytes` method, or `None` for the `Secret`
/// methods, which have none.
pub fn native_bytes_method(kind: BuiltinMethodKind) -> Option<NativeScalarMethod<BytesOperation>> {
    use BuiltinMethodKind::*;
    use BytesOperation as Op;
    use NativeScalarMethod::*;
    Some(match kind {
        StringUtf8 => Call(Op::FromUtf8Text),
        BytesToUtf8 => Call(Op::ToUtf8),
        BytesLength => Call(Op::Length),
        BytesToHex => Call(Op::ToHex),
        BytesToBase64 => Call(Op::ToBase64),
        // The value already is its unpadded base64url text.
        BytesToBase64Url => CanonicalText,
        BytesFromHex => Call(Op::FromHex),
        BytesFromBase64 => Call(Op::FromBase64),
        BytesFromBase64Url => Call(Op::FromBase64Url),
        BytesSha256 => Call(Op::Sha256),
        BytesSha384 => Call(Op::Sha384),
        BytesSha512 => Call(Op::Sha512),
        BytesHmacSha256 => Call(Op::HmacSha256),
        BytesHmacSha512 => Call(Op::HmacSha512),
        BytesVerifyEd25519 => Call(Op::VerifyEd25519),
        _ => return None,
    })
}

/// Whether native code compiles this `Bytes` method.
pub fn is_native_bytes_method(kind: BuiltinMethodKind) -> bool {
    native_bytes_method(kind).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_crypto_method_but_the_secret_ones_has_a_native_form() {
        use BuiltinMethodKind::*;
        let kinds = [
            StringUtf8,
            BytesToUtf8,
            BytesLength,
            BytesToHex,
            BytesToBase64,
            BytesToBase64Url,
            BytesFromHex,
            BytesFromBase64,
            BytesFromBase64Url,
            BytesSha256,
            BytesSha384,
            BytesSha512,
            BytesHmacSha256,
            BytesHmacSha512,
            BytesVerifyEd25519,
        ];
        for kind in kinds {
            assert!(kind.is_crypto(), "{kind:?}");
            assert!(is_native_bytes_method(kind), "{kind:?} needs a native form");
        }
        for kind in [SecretDecodeBase64After, SecretVerifyPassword] {
            assert!(kind.is_crypto(), "{kind:?}");
            assert!(!is_native_bytes_method(kind), "native code has no `Secret`");
        }
        assert!(!is_native_bytes_method(StringLength));
    }

    #[test]
    fn symbols_are_distinct_and_parsers_take_a_payload_slot() {
        let all = BytesOperation::all();
        let symbols: std::collections::HashSet<&str> = all.iter().map(|op| op.symbol()).collect();
        assert_eq!(symbols.len(), all.len());
        for op in all {
            if op.is_parse() {
                assert_eq!(op.arity(), 2, "{op:?} takes its operand and the payload slot");
            }
        }
    }

    #[test]
    fn only_byte_strings_compare_here_and_only_for_equality() {
        assert!(bytes_binary(BinaryOp::Eq, &Type::Bytes, &Type::Bytes).is_some());
        assert_eq!(
            bytes_binary(BinaryOp::NotEq, &Type::Tainted(Box::new(Type::Bytes)), &Type::Bytes).map(|b| b.reading),
            Some(BinaryReading::Equality)
        );
        assert!(bytes_binary(BinaryOp::Lt, &Type::Bytes, &Type::Bytes).is_none());
        assert!(bytes_binary(BinaryOp::Eq, &Type::String, &Type::String).is_none());
        assert!(has_bytes_operand(&Type::String, &Type::Bytes));
        assert!(!has_bytes_operand(&Type::String, &Type::String));
    }
}
