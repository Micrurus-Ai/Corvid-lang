//! `Secret`, sensitive text the program can use but never reveal.
//!
//! A signing key, a password or an API token is held as a `Secret`.
//! The program can compare two secrets, key an HMAC with one and hand
//! one to the tool that needs it; it can never print one, turn it into
//! a `String`, encode it as JSON or pass it to a model. The checker
//! refuses each of those at compile time; this type is the run-time
//! half of the guarantee:
//!
//! - **It never formats.** `Debug` and `Display` write `<secret>`, so a
//!   panic message, a log line or a debugger view cannot show the text.
//! - **Equality is constant-time.** `==` compares the bytes with
//!   [`crate::constant_time_eq`], so checking a supplied token against
//!   the expected one cannot leak it byte by byte.
//! - **The text is read in one place.** [`Secret::expose_text`] is the
//!   only way to the text outside this crate. The interpreter calls it
//!   when it hands a tool its arguments, and nowhere else. HMAC reads
//!   the key's bytes inside this crate ([`crate::HmacKey`]).
//!
//! A secret read from the environment holds text. One made by
//! [`Secret::decode_base64_after`], such as a Standard Webhooks signing
//! key, holds the decoded bytes, which are a key and not text: it keys
//! an HMAC and compares with `==`, and [`Secret::expose_text`] refuses
//! it, so a tool never receives it.

use std::fmt;
use std::sync::Arc;

/// What `Debug` and `Display` write in place of a secret's text.
pub const SECRET_DISPLAY: &str = "<secret>";

/// Sensitive text, or a key decoded from it. Cloning shares the bytes.
#[derive(Clone)]
pub struct Secret {
    bytes: Arc<[u8]>,
    /// Whether the bytes are the secret's text (read from the
    /// environment) rather than a key decoded from it.
    is_text: bool,
}

impl Secret {
    /// A secret holding `text`.
    pub fn new(text: &str) -> Self {
        Secret { bytes: Arc::from(text.as_bytes()), is_text: true }
    }

    /// The secret's text, for the one caller that must hand it on: a
    /// tool implementation receiving its arguments. Never format it.
    /// `None` for a key decoded by [`Secret::decode_base64_after`],
    /// whose bytes are not text.
    pub fn expose_text(&self) -> Option<&str> {
        if !self.is_text {
            return None;
        }
        // A text secret is built only from a `&str`, so this never fails.
        std::str::from_utf8(&self.bytes).ok()
    }

    /// The bytes an HMAC is keyed with: the text's UTF-8 encoding, or
    /// the decoded key. Read only inside this crate.
    pub(crate) fn key_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// `secret.decode_base64_after(prefix)`: the key written in base64
    /// after `prefix`, as a new secret. A Standard Webhooks signing
    /// secret is `whsec_` followed by the base64 of the key's bytes.
    ///
    /// The messages never quote the secret: unlike `Bytes.from_base64`,
    /// a decoding failure names neither the character nor its position.
    /// The prefix is a literal in the program (the checker requires
    /// one), so whether the secret starts with it reveals nothing the
    /// source does not already say.
    pub fn decode_base64_after(&self, prefix: &str) -> Result<Secret, String> {
        let not_base64 = || format!("the secret is not base64 after `{prefix}`");
        let Some(encoded) = self.bytes.strip_prefix(prefix.as_bytes()) else {
            return Err(format!("the secret does not start with `{prefix}`"));
        };
        let encoded = std::str::from_utf8(encoded).map_err(|_| not_base64())?;
        let key = corvid_text::decode_base64_bytes(encoded).map_err(|_| not_base64())?;
        if key.is_empty() {
            return Err(format!("the secret holds no key after `{prefix}`"));
        }
        Ok(Secret { bytes: Arc::from(key), is_text: false })
    }

    /// Constant-time equality, the meaning of `==` on `Secret`. Two
    /// secrets are equal when their bytes are: a decoded key equals a
    /// text secret only if it keys an HMAC exactly as that text does.
    pub fn constant_time_eq(&self, other: &Secret) -> bool {
        crate::constant_time_eq(&self.bytes, &other.bytes)
    }
}

impl PartialEq for Secret {
    fn eq(&self, other: &Self) -> bool {
        self.constant_time_eq(other)
    }
}

impl Eq for Secret {}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SECRET_DISPLAY)
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(SECRET_DISPLAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_never_shows_the_text() {
        let secret = Secret::new("whsec_live_0123456789");
        assert_eq!(format!("{secret}"), "<secret>");
        assert_eq!(format!("{secret:?}"), "<secret>");
        assert_eq!(format!("{:?}", Some(secret.clone())), "Some(<secret>)");
        assert_eq!(secret.expose_text(), Some("whsec_live_0123456789"));
    }

    #[test]
    fn equality_compares_the_whole_text() {
        let a = Secret::new("correct horse");
        assert_eq!(a, Secret::new("correct horse"));
        assert_ne!(a, Secret::new("correct horsf"));
        assert_ne!(a, Secret::new("Correct horse"));
        assert_ne!(a, Secret::new("correct hors"));
    }

    #[test]
    fn a_base64_key_after_a_prefix_decodes_to_a_key_that_is_not_text() {
        // "c2VjcmV0LWtleQ==" is the base64 of "secret-key".
        let key = Secret::new("whsec_c2VjcmV0LWtleQ==").decode_base64_after("whsec_").expect("decodes");
        assert_eq!(key.key_bytes(), b"secret-key");
        assert_eq!(key.expose_text(), None, "a decoded key is never handed on as text");
        assert_eq!(format!("{key} {key:?}"), "<secret> <secret>");
        assert_eq!(Secret::new("c2VjcmV0LWtleQ").decode_base64_after(""), Ok(key));
    }

    #[test]
    fn decoding_failures_never_quote_the_secret() {
        assert_eq!(
            Secret::new("sk_c2VjcmV0").decode_base64_after("whsec_"),
            Err("the secret does not start with `whsec_`".to_string())
        );
        let problem = Secret::new("whsec_ab!cd").decode_base64_after("whsec_").unwrap_err();
        assert_eq!(problem, "the secret is not base64 after `whsec_`");
        assert_eq!(
            Secret::new("whsec_").decode_base64_after("whsec_"),
            Err("the secret holds no key after `whsec_`".to_string())
        );
    }
}
