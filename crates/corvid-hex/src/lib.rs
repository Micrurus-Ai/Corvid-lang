//! Hex: the one encoder and decoder every Corvid crate uses.
//!
//! Lowercase hex was written by hand in about forty places: content
//! hashes, signatures, cache keys, CSRF tokens, webhook signatures, trace
//! ids. A copy that drifts (in case, or in how a decoder treats an odd
//! length) makes two digests of the same bytes read differently. Every
//! crate calls these instead. This crate has no dependencies, so the
//! lowest crate in the workspace can use it.
//! `tests/no_hand_written_hex.rs` fails when a new hand-written encoder
//! appears.

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Two lowercase hex digits per byte.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// The first `digits` lowercase hex digits of `bytes`: a short id such
/// as `review-` and 16 digits of a digest. All of them when `bytes` has
/// fewer.
pub fn encode_prefix(bytes: &[u8], digits: usize) -> String {
    let mut out = encode(&bytes[..bytes.len().min(digits.div_ceil(2))]);
    out.truncate(digits);
    out
}

/// The bytes `text` spells: digits of either case, two per byte. An error
/// names the first character that is not a hex digit and its position,
/// or says the digits do not make whole bytes.
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
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

#[cfg(test)]
mod tests {
    use super::{decode, encode, encode_prefix};

    #[test]
    fn encodes_lowercase_and_decodes_either_case() {
        assert_eq!(encode(&[0x00, 0x0f, 0xab, 0xff]), "000fabff");
        assert_eq!(encode(&[]), "");
        assert_eq!(decode("000fABff").unwrap(), vec![0x00, 0x0f, 0xab, 0xff]);
        assert_eq!(decode(&encode(b"corvid")).unwrap(), b"corvid");
    }

    #[test]
    fn a_prefix_is_the_leading_digits() {
        assert_eq!(encode_prefix(&[0xab, 0xcd, 0xef], 3), "abc");
        assert_eq!(encode_prefix(&[0xab, 0xcd], 4), "abcd");
        assert_eq!(encode_prefix(&[0xab], 6), "ab");
    }

    #[test]
    fn refuses_what_is_not_whole_bytes_of_hex() {
        assert_eq!(decode("abc").unwrap_err(), "not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte");
        assert_eq!(decode("0g").unwrap_err(), "not hex: unexpected character `g` at position 1");
    }
}
