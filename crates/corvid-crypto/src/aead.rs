//! Authenticated encryption for values kept at rest (encryption at rest,
//! K1): XChaCha20-Poly1305 under one 32-byte data key.
//!
//! Every value is sealed with a fresh random 24-byte nonce, so a key can
//! take any number of writes without a nonce repeating. A value is bound to
//! the place it belongs to through associated data (for a table column:
//! the table, the column and the row's key), so a sealed value copied to
//! another row or column fails to open instead of being read as that
//! other value. The sealed form is text, `corvid:v1:<key id>:<payload>`,
//! where the payload is the URL-safe base64 of the nonce followed by the
//! ciphertext and its tag, and the key id says which key sealed it, so a
//! store can hold values under an old and a new key while it rotates.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

/// The length of a data key, in bytes.
pub const DATA_KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const SEALED_PREFIX: &str = "corvid:v1:";

/// Why a data key or a sealed value was refused. The messages never hold
/// the key or the plaintext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SealError {
    /// The key is not 32 bytes of standard base64.
    BadKey(String),
    /// The value is not in the `corvid:v1:...` form.
    Malformed,
    /// The value was sealed under another key, named by its id.
    OtherKey(String),
    /// The value does not open: it was changed, or it belongs to another
    /// place (row, column or table) than the one it is read for.
    Tampered,
    /// The operating system's entropy source failed.
    Entropy(String),
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadKey(why) => write!(f, "the data key is not usable: {why}"),
            Self::Malformed => write!(f, "the stored value is not a sealed Corvid value"),
            Self::OtherKey(id) => write!(f, "the stored value was sealed under key `{id}`, which this program does not have"),
            Self::Tampered => write!(f, "the stored value does not open: it was changed, or it belongs to another row or column"),
            Self::Entropy(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for SealError {}

/// One data key, and the id derived from it. The key's bytes are wiped
/// when it drops.
pub struct DataKey {
    key: Zeroizing<[u8; DATA_KEY_BYTES]>,
    id: String,
}

impl std::fmt::Debug for DataKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DataKey({})", self.id)
    }
}

impl DataKey {
    /// A key from its standard base64 text, as `corvid keys new` prints
    /// it and the secret store holds it.
    pub fn from_base64(text: &str) -> Result<Self, SealError> {
        let bytes = Zeroizing::new(
            corvid_text::encoding::decode_base64_bytes(text.trim()).map_err(|_| SealError::BadKey("it is not base64".into()))?,
        );
        let key: [u8; DATA_KEY_BYTES] = bytes.as_slice().try_into().map_err(|_| {
            SealError::BadKey(format!("it holds {} bytes; a data key is {DATA_KEY_BYTES}", bytes.len()))
        })?;
        Ok(Self::from_bytes(key))
    }

    fn from_bytes(key: [u8; DATA_KEY_BYTES]) -> Self {
        let key = Zeroizing::new(key);
        // The id names the key without revealing it: the first 8 bytes of
        // an HMAC-SHA256 of a fixed label under the key. The same `hmac`
        // crate `crate::hmac_sha256` uses, called directly because that
        // function takes its key as `Bytes`, which would copy the data key
        // into a buffer that is not wiped.
        use hmac::Mac;
        let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(key.as_slice()).expect("HMAC takes a key of any length");
        mac.update(b"corvid data key id");
        let tag = mac.finalize().into_bytes();
        let id = corvid_hex::encode(&tag[..8]);
        Self { key, id }
    }

    /// A fresh random key's base64 text.
    pub fn generate_base64() -> Result<String, SealError> {
        let mut key = Zeroizing::new([0u8; DATA_KEY_BYTES]);
        crate::fill_random(key.as_mut_slice()).map_err(SealError::Entropy)?;
        Ok(corvid_text::encoding::encode_base64_bytes(key.as_slice()))
    }

    /// The id stored with every value this key seals.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Seal `plaintext` for the place `context` names.
    pub fn seal(&self, plaintext: &[u8], context: &[u8]) -> Result<String, SealError> {
        let mut nonce = [0u8; NONCE_BYTES];
        crate::fill_random(&mut nonce).map_err(SealError::Entropy)?;
        Ok(self.seal_with_nonce(plaintext, context, &nonce))
    }

    fn seal_with_nonce(&self, plaintext: &[u8], context: &[u8], nonce: &[u8; NONCE_BYTES]) -> String {
        let cipher = XChaCha20Poly1305::new(self.key.as_slice().into());
        let sealed = cipher
            .encrypt(XNonce::from_slice(nonce), Payload { msg: plaintext, aad: context })
            .expect("XChaCha20-Poly1305 sealing cannot fail for in-memory input");
        let mut payload = Vec::with_capacity(NONCE_BYTES + sealed.len());
        payload.extend_from_slice(nonce);
        payload.extend_from_slice(&sealed);
        format!("{SEALED_PREFIX}{}:{}", self.id, corvid_text::encoding::encode_base64_url_bytes(&payload))
    }

    /// The key id a sealed value names, without opening it: what a store
    /// rotating keys uses to pick the key, or to report one it lacks.
    pub fn sealed_key_id(sealed: &str) -> Result<&str, SealError> {
        let rest = sealed.strip_prefix(SEALED_PREFIX).ok_or(SealError::Malformed)?;
        rest.split_once(':').map(|(id, _)| id).ok_or(SealError::Malformed)
    }

    /// Open a value this key sealed for the place `context` names.
    pub fn open(&self, sealed: &str, context: &[u8]) -> Result<Zeroizing<Vec<u8>>, SealError> {
        let rest = sealed.strip_prefix(SEALED_PREFIX).ok_or(SealError::Malformed)?;
        let (id, payload) = rest.split_once(':').ok_or(SealError::Malformed)?;
        if id != self.id {
            return Err(SealError::OtherKey(id.to_string()));
        }
        let payload = corvid_text::encoding::decode_base64_url_bytes(payload).map_err(|_| SealError::Malformed)?;
        if payload.len() < NONCE_BYTES {
            return Err(SealError::Malformed);
        }
        let (nonce, ciphertext) = payload.split_at(NONCE_BYTES);
        let cipher = XChaCha20Poly1305::new(self.key.as_slice().into());
        cipher
            .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: context })
            .map(Zeroizing::new)
            .map_err(|_| SealError::Tampered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> DataKey {
        DataKey::from_bytes([byte; DATA_KEY_BYTES])
    }

    fn hex(text: &str) -> Vec<u8> {
        corvid_hex::decode(text).unwrap()
    }

    /// The XChaCha20-Poly1305 test vector of draft-irtf-cfrg-xchacha,
    /// appendix A.3.1, checked through the sealed form.
    #[test]
    fn sealing_matches_the_published_xchacha20_poly1305_vector() {
        let key_bytes: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f").try_into().unwrap();
        let nonce: [u8; 24] = hex("404142434445464748494a4b4c4d4e4f5051525354555657").try_into().unwrap();
        let aad = hex("50515253c0c1c2c3c4c5c6c7");
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let expected = hex(concat!(
            "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb",
            "731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452",
            "2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9",
            "21f9664c97637da9768812f615c68b13b52e",
            "c0875924c1c7987947deafd8780acf49",
        ));
        let key = DataKey::from_bytes(key_bytes);
        let sealed = key.seal_with_nonce(plaintext, &aad, &nonce);
        let payload = corvid_text::encoding::decode_base64_url_bytes(sealed.rsplit(':').next().unwrap()).unwrap();
        assert_eq!(&payload[..24], &nonce);
        assert_eq!(&payload[24..], expected.as_slice());
        assert_eq!(key.open(&sealed, &aad).unwrap().as_slice(), plaintext);
    }

    #[test]
    fn a_value_opens_only_for_the_place_it_was_sealed_for() {
        let key = key(7);
        let sealed = key.seal(b"sk_live_123", b"Customer.stripe_key[1]").unwrap();
        assert!(sealed.starts_with(&format!("corvid:v1:{}:", key.id())));
        assert_eq!(key.open(&sealed, b"Customer.stripe_key[1]").unwrap().as_slice(), b"sk_live_123");
        // Moved to another row, or another column: it does not open.
        assert_eq!(key.open(&sealed, b"Customer.stripe_key[2]").unwrap_err(), SealError::Tampered);
        assert_eq!(key.open(&sealed, b"Customer.note[1]").unwrap_err(), SealError::Tampered);
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce() {
        let key = key(7);
        let first = key.seal(b"same", b"ctx").unwrap();
        let second = key.seal(b"same", b"ctx").unwrap();
        assert_ne!(first, second, "the same value sealed twice must not look the same");
    }

    #[test]
    fn a_changed_value_or_another_key_is_refused_by_name() {
        let key = key(7);
        let sealed = key.seal(b"value", b"ctx").unwrap();
        let mut changed = sealed.clone();
        let last = changed.pop().unwrap();
        changed.push(if last == 'A' { 'B' } else { 'A' });
        assert!(matches!(key.open(&changed, b"ctx").unwrap_err(), SealError::Tampered | SealError::Malformed));
        let other = self::key(9);
        assert_eq!(other.open(&sealed, b"ctx").unwrap_err(), SealError::OtherKey(key.id().to_string()));
        assert_eq!(DataKey::sealed_key_id(&sealed).unwrap(), key.id());
        assert_eq!(key.open("plain text", b"ctx").unwrap_err(), SealError::Malformed);
    }

    #[test]
    fn a_key_is_read_from_32_bytes_of_base64_and_never_shown() {
        let text = DataKey::generate_base64().unwrap();
        let key = DataKey::from_base64(&text).unwrap();
        assert_eq!(key.id().len(), 16);
        assert!(!format!("{key:?}").contains(&text), "Debug names the key by its id only");
        assert!(matches!(DataKey::from_base64("c2hvcnQ=").unwrap_err(), SealError::BadKey(why) if why.contains("5 bytes")));
        assert!(matches!(DataKey::from_base64("not base64!").unwrap_err(), SealError::BadKey(_)));
    }
}
