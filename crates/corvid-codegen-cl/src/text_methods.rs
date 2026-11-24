//! The B03 text methods native code calls: which runtime symbol runs
//! each one. The symbols are defined in
//! `corvid-runtime/src/ffi_bridge/text.rs`, which calls `corvid-text`,
//! the code the interpreter calls. A misspelled symbol fails to link.

use corvid_types::BuiltinMethodKind;

/// How a text method is called and what it returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeTextShape {
    /// `(text) -> String`.
    Encoder,
    /// `(text, payload) -> tag`, filling a `Result<String, String>`.
    Decoder,
}

/// The runtime symbol and call shape of a text method, or `None` when
/// the method has no native form yet.
pub fn native_text_symbol(kind: BuiltinMethodKind) -> Option<(&'static str, NativeTextShape)> {
    use BuiltinMethodKind::*;
    use NativeTextShape::*;
    Some(match kind {
        StringEncodeBase64 => ("corvid_string_encode_base64", Encoder),
        StringEncodeBase64Url => ("corvid_string_encode_base64_url", Encoder),
        StringEncodeHex => ("corvid_string_encode_hex", Encoder),
        StringEncodeUrlComponent => ("corvid_string_encode_url_component", Encoder),
        StringDecodeBase64 => ("corvid_string_decode_base64", Decoder),
        StringDecodeBase64Url => ("corvid_string_decode_base64_url", Decoder),
        StringDecodeHex => ("corvid_string_decode_hex", Decoder),
        StringDecodeUrlComponent => ("corvid_string_decode_url_component", Decoder),
        StringParseUuid => ("corvid_string_parse_uuid", Decoder),
        _ => return None,
    })
}

/// Every text method with a native form, for declaring the imports.
pub(crate) const NATIVE_TEXT_METHODS: [BuiltinMethodKind; 9] = {
    use BuiltinMethodKind::*;
    [
        StringEncodeBase64,
        StringEncodeBase64Url,
        StringEncodeHex,
        StringEncodeUrlComponent,
        StringDecodeBase64,
        StringDecodeBase64Url,
        StringDecodeHex,
        StringDecodeUrlComponent,
        StringParseUuid,
    ]
};

/// Whether native code compiles this text method.
pub fn native_text_method(kind: BuiltinMethodKind) -> bool {
    native_text_symbol(kind).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_encoder_and_decoder_has_a_native_symbol() {
        for kind in NATIVE_TEXT_METHODS {
            let (symbol, shape) = native_text_symbol(kind).expect("listed kinds have symbols");
            assert!(symbol.starts_with("corvid_string_"));
            assert_eq!(shape == NativeTextShape::Decoder, kind.is_text_decoder(), "{kind:?}");
            assert_eq!(shape == NativeTextShape::Encoder, kind.is_text_encoder(), "{kind:?}");
        }
        let listed = NATIVE_TEXT_METHODS.len();
        assert_eq!(listed, 9, "a new text method needs a native symbol and a runtime entry point");
    }
}
