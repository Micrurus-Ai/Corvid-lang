//! B03 text methods: signatures of the encoders, decoders and
//! `parse_uuid`.

use super::check;
use crate::errors::TypeErrorKind;

#[test]
fn encoders_return_string_and_decoders_return_result() {
    let checked = check(
        "agent main(text: String) -> Result<String, String>:
    encoded = text.encode_base64() + text.encode_base64_url() + text.encode_hex() + text.encode_url_component()
    plain = encoded.decode_base64()?
    also = plain.decode_base64_url()?
    hex = also.decode_hex()?
    component = hex.decode_url_component()?
    return component.parse_uuid()
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn a_decoder_result_must_be_handled_before_use_as_text() {
    let checked = check(
        "agent main(text: String) -> String:
    return text.decode_hex()
",
    );
    assert!(
        checked.errors.iter().any(|e| matches!(&e.kind,
            TypeErrorKind::TypeMismatch { got, .. } | TypeErrorKind::ReturnTypeMismatch { got, .. }
                if got == "Result<String, String>")),
        "got {:?}",
        checked.errors
    );
}

#[test]
fn text_methods_take_no_arguments_and_need_a_string_receiver() {
    let arity = check(
        "agent main(text: String) -> String:
    return text.encode_hex(2)
",
    );
    assert!(
        arity.errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::ArityMismatch { callee, expected: 0, got: 1 } if callee == "encode_hex")),
        "got {:?}",
        arity.errors
    );
    let receiver = check(
        "agent main(n: Int) -> String:
    return n.encode_hex()
",
    );
    assert!(!receiver.errors.is_empty(), "an Int has no encode_hex");
}
