//! Byte strings and hashing on the interpreter tier: the `Bytes`
//! methods and `String.utf8()`.
//!
//! Every rule is `corvid_crypto`'s, the code the native runtime will
//! call, so this file only moves values in and out of `Value`. Every
//! method is pure, so none is traced and all are allowed in
//! `@deterministic` code. `==` on `Bytes` is not here: `Value`'s
//! equality calls `corvid_crypto::Bytes::constant_time_eq`, so it is
//! constant-time inside records and lists too.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, Value};
use corvid_ast::Span;
use corvid_crypto::Bytes;
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

pub(super) fn eval_crypto_method(kind: BuiltinMethodKind, receiver: &Value, span: Span) -> Result<Value, InterpError> {
    use BuiltinMethodKind::*;
    let text = |value: String| Value::String(Arc::from(value));
    let decoded = |result: Result<Value, String>| match result {
        Ok(value) => Value::ResultOk(BoxedValue::new(value)),
        Err(message) => Value::ResultErr(BoxedValue::new(text(message))),
    };
    let length = |bytes: &Bytes| {
        i64::try_from(bytes.len()).map(Value::Int).map_err(|_| {
            InterpError::new(InterpErrorKind::Arithmetic("a byte string's length exceeds the Int range".into()), span)
        })
    };
    match (kind, receiver) {
        (StringUtf8, Value::String(s)) => Ok(Value::Bytes(Bytes::from_utf8_text(s))),
        (BytesToUtf8, Value::Bytes(b)) => Ok(decoded(b.to_utf8().map(text))),
        (BytesLength, Value::Bytes(b)) => length(b),
        (BytesToHex, Value::Bytes(b)) => Ok(text(b.to_hex())),
        (BytesToBase64, Value::Bytes(b)) => Ok(text(b.to_base64())),
        (BytesToBase64Url, Value::Bytes(b)) => Ok(text(b.to_base64_url())),
        (BytesFromHex, Value::String(s)) => Ok(decoded(Bytes::from_hex(s).map(Value::Bytes))),
        (BytesFromBase64, Value::String(s)) => Ok(decoded(Bytes::from_base64(s).map(Value::Bytes))),
        (BytesFromBase64Url, Value::String(s)) => Ok(decoded(Bytes::from_base64_url(s).map(Value::Bytes))),
        (BytesSha256, Value::Bytes(b)) => Ok(Value::Bytes(corvid_crypto::sha256(b))),
        (BytesSha384, Value::Bytes(b)) => Ok(Value::Bytes(corvid_crypto::sha384(b))),
        (BytesSha512, Value::Bytes(b)) => Ok(Value::Bytes(corvid_crypto::sha512(b))),
        (other, value) => Err(InterpError::new(
            InterpErrorKind::Other(format!("internal: `{other:?}` cannot run on {}", value.type_name())),
            span,
        )),
    }
}
