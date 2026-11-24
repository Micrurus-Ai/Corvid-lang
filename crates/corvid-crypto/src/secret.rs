//! `Secret`, sensitive text the program can use but never reveal.
//!
//! A signing key, a password or an API token is held as a `Secret`.
//! The program can compare two secrets and hand one to the tool that
//! needs it; it can never print one, turn it into a `String`, encode
//! it as JSON or pass it to a model. The checker refuses each of those
//! at compile time; this type is the run-time half of the guarantee:
//!
//! - **It never formats.** `Debug` and `Display` write `<secret>`, so a
//!   panic message, a log line or a debugger view cannot show the text.
//! - **Equality is constant-time.** `==` compares the UTF-8 bytes with
//!   [`crate::constant_time_eq`], so checking a supplied token against
//!   the expected one cannot leak it byte by byte.
//! - **The text is read in one place.** [`Secret::expose_text`] is the
//!   only way to the text. The interpreter calls it when it hands a
//!   tool its arguments, and nowhere else.

use std::fmt;
use std::sync::Arc;

/// What `Debug` and `Display` write in place of a secret's text.
pub const SECRET_DISPLAY: &str = "<secret>";

/// Sensitive text. Cloning shares the text.
#[derive(Clone)]
pub struct Secret(Arc<str>);

impl Secret {
    /// A secret holding `text`.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Secret(text.into())
    }

    /// The secret's text, for the one caller that must hand it on: a
    /// tool implementation receiving its arguments. Never format it.
    pub fn expose_text(&self) -> &str {
        &self.0
    }

    /// Constant-time equality, the meaning of `==` on `Secret`.
    pub fn constant_time_eq(&self, other: &Secret) -> bool {
        crate::constant_time_eq(self.0.as_bytes(), other.0.as_bytes())
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
        assert_eq!(secret.expose_text(), "whsec_live_0123456789");
    }

    #[test]
    fn equality_compares_the_whole_text() {
        let a = Secret::new("correct horse");
        assert_eq!(a, Secret::new("correct horse"));
        assert_ne!(a, Secret::new("correct horsf"));
        assert_ne!(a, Secret::new("Correct horse"));
        assert_ne!(a, Secret::new("correct hors"));
    }
}
