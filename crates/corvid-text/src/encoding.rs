//! Base64, hex and URL-component encoding of text and of byte strings.
//!
//! Encoders never fail. Decoders accept exactly one spelling of each
//! byte string, apart from base64 padding, which may be present or
//! absent; anything else returns `Err` with the first problem and its
//! position. The text functions encode a string's UTF-8 bytes and
//! require decoded bytes to be UTF-8, because the result is a Corvid
//! `String`. The byte functions (`*_bytes`) are the same codecs over
//! raw bytes, for Corvid's `Bytes` type; the text functions are built
//! on them, so the two cannot disagree on an alphabet or a message.

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::{DecodeError, Engine};

/// RFC 4648 section 4: the standard alphabet, padded on encode.
const STANDARD: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// RFC 4648 section 5: the URL-safe alphabet, unpadded on encode, as
/// JSON Web Tokens use it.
const URL_SAFE: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// `text.encode_base64()`.
pub fn encode_base64(text: &str) -> String {
    encode_base64_bytes(text.as_bytes())
}

/// `text.decode_base64()`.
pub fn decode_base64(text: &str) -> Result<String, String> {
    utf8_text(decode_base64_bytes(text)?)
}

/// `text.encode_base64_url()`.
pub fn encode_base64_url(text: &str) -> String {
    encode_base64_url_bytes(text.as_bytes())
}

/// `text.decode_base64_url()`.
pub fn decode_base64_url(text: &str) -> Result<String, String> {
    utf8_text(decode_base64_url_bytes(text)?)
}

/// `bytes.to_base64()`: the standard alphabet, padded.
pub fn encode_base64_bytes(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// `Bytes.from_base64(text)`: padding optional.
pub fn decode_base64_bytes(text: &str) -> Result<Vec<u8>, String> {
    decode_base64_with(&STANDARD, "base64", text)
}

/// `bytes.to_base64_url()`: the URL-safe alphabet, unpadded.
pub fn encode_base64_url_bytes(bytes: &[u8]) -> String {
    URL_SAFE.encode(bytes)
}

/// `Bytes.from_base64_url(text)`: padding optional.
pub fn decode_base64_url_bytes(text: &str) -> Result<Vec<u8>, String> {
    decode_base64_with(&URL_SAFE, "URL-safe base64", text)
}

fn decode_base64_with(engine: &GeneralPurpose, name: &str, text: &str) -> Result<Vec<u8>, String> {
    engine.decode(text.as_bytes()).map_err(|error| match error {
        DecodeError::InvalidByte(offset, _) => format!(
            "not {name}: unexpected character {} at position {}",
            describe_char_at(text, offset),
            char_position(text, offset)
        ),
        DecodeError::InvalidLastSymbol(offset, _) => format!(
            "not {name}: the last character {} at position {} encodes bits past the end of the data",
            describe_char_at(text, offset),
            char_position(text, offset)
        ),
        DecodeError::InvalidLength(_) => format!(
            "not {name}: {} characters cannot encode whole bytes",
            text.chars().count()
        ),
        DecodeError::InvalidPadding => format!("not {name}: misplaced `=` padding"),
    })
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// `text.encode_hex()`: two lowercase digits per UTF-8 byte.
pub fn encode_hex(text: &str) -> String {
    encode_hex_bytes(text.as_bytes())
}

/// `text.decode_hex()`: either case; an even number of digits.
pub fn decode_hex(text: &str) -> Result<String, String> {
    utf8_text(decode_hex_bytes(text)?)
}

/// `bytes.to_hex()`: two lowercase digits per byte.
pub fn encode_hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX_DIGITS[usize::from(byte >> 4)] as char);
        out.push(HEX_DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// `Bytes.from_hex(text)`: either case; an even number of digits.
pub fn decode_hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let mut high: Option<u8> = None;
    for (position, ch) in text.chars().enumerate() {
        let Some(digit) = ch.to_digit(16) else {
            return Err(format!("not hex: unexpected character `{ch}` at position {position}"));
        };
        let digit = digit as u8;
        match high.take() {
            None => high = Some(digit),
            Some(h) => bytes.push(h << 4 | digit),
        }
    }
    if high.is_some() {
        return Err(format!(
            "not hex: {} digits cannot encode whole bytes; hex needs two digits per byte",
            text.chars().count()
        ));
    }
    Ok(bytes)
}

/// The RFC 3986 unreserved characters, which never need encoding in
/// any URL component.
fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// `text.encode_url_component()`: every byte outside the unreserved
/// set becomes `%XX` with uppercase digits (RFC 3986 section 2.1).
pub fn encode_url_component(text: &str) -> String {
    const UPPER: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(UPPER[usize::from(byte >> 4)] as char);
            out.push(UPPER[usize::from(byte & 0x0f)] as char);
        }
    }
    out
}

/// `text.decode_url_component()`: `%XX` becomes byte `XX`; every other
/// character, including `+`, stands for itself.
pub fn decode_url_component(text: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut chars = text.char_indices().enumerate().peekable();
    while let Some((position, (_, ch))) = chars.next() {
        if ch != '%' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let high = chars.next().and_then(|(_, (_, c))| c.to_digit(16));
        let low = chars.next().and_then(|(_, (_, c))| c.to_digit(16));
        match (high, low) {
            (Some(high), Some(low)) => bytes.push((high << 4 | low) as u8),
            _ => {
                return Err(format!(
                    "not URL-encoded: `%` at position {position} must be followed by two hex digits"
                ))
            }
        }
    }
    utf8_text(bytes)
}

fn utf8_text(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|error| {
        format!(
            "the decoded bytes are not UTF-8 text (byte {} starts an invalid sequence)",
            error.utf8_error().valid_up_to()
        )
    })
}

/// The character position of byte `offset` in `text`.
fn char_position(text: &str, offset: usize) -> usize {
    text.char_indices().take_while(|(index, _)| *index < offset).count()
}

/// The character that starts at byte `offset`, quoted for a message.
fn describe_char_at(text: &str, offset: usize) -> String {
    match text.get(offset..).and_then(|rest| rest.chars().next()) {
        Some(ch) => format!("`{}`", ch.escape_debug()),
        None => "(end of text)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(encode_base64(plain), encoded);
            assert_eq!(decode_base64(encoded).as_deref(), Ok(plain));
        }
    }

    #[test]
    fn base64_encodes_the_utf8_bytes_of_text() {
        assert_eq!(encode_base64("café ☕"), "Y2Fmw6kg4piV");
        assert_eq!(decode_base64("Y2Fmw6kg4piV").as_deref(), Ok("café ☕"));
    }

    #[test]
    fn base64_decoding_accepts_missing_padding_only() {
        assert_eq!(decode_base64("Zm8").as_deref(), Ok("fo"));
        assert_eq!(decode_base64("Zm8=").as_deref(), Ok("fo"));
        assert!(decode_base64("Zm8==").is_err());
        assert!(decode_base64("Zm 8=").is_err());
    }

    #[test]
    fn base64_errors_name_the_character_and_its_position() {
        assert_eq!(
            decode_base64("Zm9v!A==").unwrap_err(),
            "not base64: unexpected character `!` at position 4"
        );
        // Positions count characters, not bytes.
        assert_eq!(
            decode_base64("éZm").unwrap_err(),
            "not base64: unexpected character `é` at position 0"
        );
        assert_eq!(decode_base64("Z").unwrap_err(), "not base64: 1 characters cannot encode whole bytes");
    }

    #[test]
    fn base64_rejects_non_zero_trailing_bits() {
        // `Zh` decodes to `f` only if the unused bits are ignored.
        assert!(decode_base64("Zh==").unwrap_err().contains("encodes bits past the end"));
    }

    #[test]
    fn base64_reports_decoded_bytes_that_are_not_text() {
        assert_eq!(
            decode_base64("/w==").unwrap_err(),
            "the decoded bytes are not UTF-8 text (byte 0 starts an invalid sequence)"
        );
    }

    #[test]
    fn url_safe_base64_uses_its_own_alphabet_without_padding() {
        // These UTF-8 bytes (df bf ef bf bf) encode with `/` in the
        // standard alphabet and `_` in the URL-safe one.
        let text = "\u{7ff}\u{ffff}";
        assert_eq!(encode_base64(text), "37/vv78=");
        assert_eq!(encode_base64_url(text), "37_vv78");
        assert_eq!(decode_base64_url("37_vv78").as_deref(), Ok(text));
        assert_eq!(decode_base64_url("37_vv78=").as_deref(), Ok(text));
        assert!(decode_base64_url("37/vv78").unwrap_err().starts_with("not URL-safe base64: unexpected character `/`"));
        assert!(decode_base64("37_vv78").is_err());
    }

    #[test]
    fn hex_round_trips_in_lowercase_and_decodes_either_case() {
        assert_eq!(encode_hex("Hi ☕"), "486920e29895");
        assert_eq!(decode_hex("486920e29895").as_deref(), Ok("Hi ☕"));
        assert_eq!(decode_hex("486920E29895").as_deref(), Ok("Hi ☕"));
        assert_eq!(decode_hex("").as_deref(), Ok(""));
    }

    #[test]
    fn hex_errors_name_the_problem() {
        assert_eq!(decode_hex("4g").unwrap_err(), "not hex: unexpected character `g` at position 1");
        assert_eq!(
            decode_hex("486").unwrap_err(),
            "not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte"
        );
        assert!(decode_hex("ff").unwrap_err().starts_with("the decoded bytes are not UTF-8"));
    }

    #[test]
    fn byte_codecs_carry_bytes_that_are_not_text() {
        let bytes = [0x00u8, 0xff, 0xfe, 0x80];
        assert_eq!(encode_hex_bytes(&bytes), "00fffe80");
        assert_eq!(decode_hex_bytes("00FFfe80").as_deref(), Ok(&bytes[..]));
        assert_eq!(encode_base64_bytes(&bytes), "AP/+gA==");
        assert_eq!(decode_base64_bytes("AP/+gA").as_deref(), Ok(&bytes[..]));
        assert_eq!(encode_base64_url_bytes(&bytes), "AP_-gA");
        assert_eq!(decode_base64_url_bytes("AP_-gA==").as_deref(), Ok(&bytes[..]));
        assert_eq!(decode_hex_bytes("").as_deref(), Ok(&[][..]));
    }

    #[test]
    fn byte_codecs_report_the_same_errors_as_the_text_codecs() {
        assert_eq!(decode_hex_bytes("4g").unwrap_err(), decode_hex("4g").unwrap_err());
        assert_eq!(decode_base64_bytes("Zm9v!A==").unwrap_err(), decode_base64("Zm9v!A==").unwrap_err());
        assert_eq!(decode_base64_url_bytes("37/vv78").unwrap_err(), decode_base64_url("37/vv78").unwrap_err());
    }

    #[test]
    fn url_components_keep_only_unreserved_characters() {
        assert_eq!(encode_url_component("AZaz09-._~"), "AZaz09-._~");
        assert_eq!(encode_url_component("a b&c=d/e?f+g"), "a%20b%26c%3Dd%2Fe%3Ff%2Bg");
        assert_eq!(encode_url_component("café"), "caf%C3%A9");
    }

    #[test]
    fn url_components_decode_percent_escapes_and_leave_plus_alone() {
        assert_eq!(decode_url_component("a%20b%2Bc+d").as_deref(), Ok("a b+c+d"));
        assert_eq!(decode_url_component("caf%c3%a9").as_deref(), Ok("café"));
        assert_eq!(decode_url_component("café").as_deref(), Ok("café"));
        for text in ["a b&c=d/e?f+g", "café ☕", "100%", ""] {
            assert_eq!(decode_url_component(&encode_url_component(text)).as_deref(), Ok(text));
        }
    }

    #[test]
    fn url_component_errors_name_the_bad_escape() {
        assert_eq!(
            decode_url_component("100%").unwrap_err(),
            "not URL-encoded: `%` at position 3 must be followed by two hex digits"
        );
        assert_eq!(
            decode_url_component("é%zz").unwrap_err(),
            "not URL-encoded: `%` at position 1 must be followed by two hex digits"
        );
        assert!(decode_url_component("%ff").unwrap_err().starts_with("the decoded bytes are not UTF-8"));
    }
}
