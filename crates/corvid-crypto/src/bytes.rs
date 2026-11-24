//! `Bytes`, Corvid's immutable byte string.
//!
//! Equality is constant-time: comparing two byte strings of the same
//! length takes the same time whichever bytes differ, so comparing a
//! computed signature with a received one cannot leak the signature
//! byte by byte. Comparing byte strings of different lengths returns
//! `false` at once, which reveals only the lengths; a digest or MAC
//! tag's length is public. There is deliberately no ordering, because
//! an ordering would have to stop at the first differing byte.

use std::fmt;
use std::sync::Arc;

use subtle::ConstantTimeEq;

/// An immutable byte string. Cloning shares the bytes.
#[derive(Clone)]
pub struct Bytes(Arc<[u8]>);

impl Bytes {
    /// Bytes holding a copy of `bytes`.
    pub fn new(bytes: &[u8]) -> Self {
        Bytes(Arc::from(bytes))
    }

    /// `text.utf8()`: the UTF-8 encoding of a string.
    pub fn from_utf8_text(text: &str) -> Self {
        Bytes::new(text.as_bytes())
    }

    /// `Bytes.from_hex(text)`: either case, two digits per byte.
    pub fn from_hex(text: &str) -> Result<Self, String> {
        corvid_text::decode_hex_bytes(text).map(Bytes::from)
    }

    /// `Bytes.from_base64(text)`: the standard alphabet, padding optional.
    pub fn from_base64(text: &str) -> Result<Self, String> {
        corvid_text::decode_base64_bytes(text).map(Bytes::from)
    }

    /// `Bytes.from_base64_url(text)`: the URL-safe alphabet, padding
    /// optional. This is also the JSON form.
    pub fn from_base64_url(text: &str) -> Result<Self, String> {
        corvid_text::decode_base64_url_bytes(text).map(Bytes::from)
    }

    /// The raw bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// `bytes.length()`: the number of bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `bytes.to_utf8()`: the bytes read as UTF-8 text, or a message
    /// naming the first byte that starts an invalid sequence.
    pub fn to_utf8(&self) -> Result<String, String> {
        std::str::from_utf8(&self.0).map(str::to_owned).map_err(|error| {
            format!(
                "the bytes are not UTF-8 text (byte {} starts an invalid sequence)",
                error.valid_up_to()
            )
        })
    }

    /// `bytes.to_hex()`: two lowercase digits per byte.
    pub fn to_hex(&self) -> String {
        corvid_text::encode_hex_bytes(&self.0)
    }

    /// `bytes.to_base64()`: the standard alphabet, padded.
    pub fn to_base64(&self) -> String {
        corvid_text::encode_base64_bytes(&self.0)
    }

    /// `bytes.to_base64_url()`: the URL-safe alphabet, unpadded. This is
    /// also the JSON form.
    pub fn to_base64_url(&self) -> String {
        corvid_text::encode_base64_url_bytes(&self.0)
    }

    /// Constant-time equality, the meaning of `==` on `Bytes`.
    pub fn constant_time_eq(&self, other: &Bytes) -> bool {
        bool::from(self.0.as_ref().ct_eq(other.0.as_ref()))
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Bytes(Arc::from(bytes))
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.constant_time_eq(other)
    }
}

impl Eq for Bytes {}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({})", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equality_compares_every_byte() {
        let a = Bytes::new(&[1, 2, 3, 4]);
        assert_eq!(a, Bytes::new(&[1, 2, 3, 4]));
        assert_ne!(a, Bytes::new(&[0, 2, 3, 4]));
        assert_ne!(a, Bytes::new(&[1, 2, 3, 5]));
        assert_eq!(Bytes::new(&[]), Bytes::new(&[]));
    }

    #[test]
    fn byte_strings_of_different_lengths_are_unequal() {
        let a = Bytes::new(&[1, 2, 3]);
        assert_ne!(a, Bytes::new(&[1, 2]));
        assert_ne!(a, Bytes::new(&[1, 2, 3, 0]));
        assert_ne!(Bytes::new(&[]), Bytes::new(&[0]));
    }

    #[test]
    fn hex_and_base64_round_trip_arbitrary_bytes() {
        let all: Vec<u8> = (0..=255).collect();
        let bytes = Bytes::from(all);
        assert_eq!(Bytes::from_hex(&bytes.to_hex()), Ok(bytes.clone()));
        assert_eq!(Bytes::from_base64(&bytes.to_base64()), Ok(bytes.clone()));
        assert_eq!(Bytes::from_base64_url(&bytes.to_base64_url()), Ok(bytes.clone()));
        assert!(bytes.to_hex().starts_with("000102"));
        assert!(!bytes.to_base64_url().contains(['+', '/', '=']));
    }

    #[test]
    fn decoders_return_the_text_codec_messages() {
        assert_eq!(
            Bytes::from_hex("abc").unwrap_err(),
            "not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte"
        );
        assert_eq!(
            Bytes::from_base64_url("a+b=").unwrap_err(),
            "not URL-safe base64: unexpected character `+` at position 1"
        );
    }

    #[test]
    fn utf8_text_round_trips_and_reports_invalid_bytes() {
        let bytes = Bytes::from_utf8_text("café");
        assert_eq!(bytes.len(), 5);
        assert_eq!(bytes.to_utf8().as_deref(), Ok("café"));
        assert_eq!(
            Bytes::new(&[b'a', 0xff]).to_utf8().unwrap_err(),
            "the bytes are not UTF-8 text (byte 1 starts an invalid sequence)"
        );
    }
}
