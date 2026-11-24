//! The B03 text builtin methods on the interpreter tier: encoders,
//! decoders and UUID parsing. Every rule is `corvid-text`'s, the same
//! code the native runtime calls, so this file only moves values in and
//! out of `Value`.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, Value};
use corvid_ast::Span;
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

/// Evaluate a text encoder or decoder on the receiver `text`.
pub(super) fn eval_text_method(
    kind: BuiltinMethodKind,
    text: &str,
    span: Span,
) -> Result<Value, InterpError> {
    use BuiltinMethodKind::*;
    let string = |s: String| Value::String(Arc::from(s));
    let result = |r: Result<String, String>| match r {
        Ok(text) => Value::ResultOk(BoxedValue::new(string(text))),
        Err(message) => Value::ResultErr(BoxedValue::new(string(message))),
    };
    Ok(match kind {
        StringEncodeBase64 => string(corvid_text::encode_base64(text)),
        StringEncodeBase64Url => string(corvid_text::encode_base64_url(text)),
        StringEncodeHex => string(corvid_text::encode_hex(text)),
        StringEncodeUrlComponent => string(corvid_text::encode_url_component(text)),
        StringDecodeBase64 => result(corvid_text::decode_base64(text)),
        StringDecodeBase64Url => result(corvid_text::decode_base64_url(text)),
        StringDecodeHex => result(corvid_text::decode_hex(text)),
        StringDecodeUrlComponent => result(corvid_text::decode_url_component(text)),
        StringParseUuid => result(corvid_text::parse_uuid(text)),
        other => {
            return Err(InterpError::new(
                InterpErrorKind::Other(format!("`{other:?}` is not a text encoding method")),
                span,
            ))
        }
    })
}
