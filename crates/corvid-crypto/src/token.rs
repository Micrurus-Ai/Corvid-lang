//! Random tokens: `random_token(bytes)` in std/crypto.
//!
//! A token is a number of random bytes from the operating system,
//! written in unpadded URL-safe base64 (RFC 4648 section 5) so it can
//! travel in a URL, a header or a cookie unchanged. It is a [`Secret`]:
//! the program can compare it, hash it with `hash_password` and hand it
//! to the tool that delivers it, and it never reaches a trace.

use crate::Secret;

/// The fewest random bytes a token may have: 16 bytes, 128 bits, the
/// floor below which a token can be guessed by an attacker with enough
/// attempts (OWASP's session-identifier guidance asks for at least 64
/// bits of entropy; 128 leaves no doubt).
pub const MIN_TOKEN_BYTES: i64 = 16;

/// The most random bytes a token may have: 1024. Nothing needs more,
/// and a larger request is a mistake in the program, such as a length
/// in bits or characters written where bytes are meant.
pub const MAX_TOKEN_BYTES: i64 = 1024;

/// `random_token(bytes)`: `bytes` random bytes as a [`Secret`] holding
/// their unpadded base64url text, which is `ceil(bytes * 4 / 3)`
/// characters long. An error when `bytes` is outside
/// [`MIN_TOKEN_BYTES`]`..=`[`MAX_TOKEN_BYTES`] or the operating system
/// cannot supply entropy.
pub fn random_token(bytes: i64) -> Result<Secret, String> {
    if bytes < MIN_TOKEN_BYTES {
        return Err(format!(
            "random_token({bytes}) is too short to be safe: a token needs at least {MIN_TOKEN_BYTES} random bytes (128 bits); use random_token(32) for a session or API key"
        ));
    }
    if bytes > MAX_TOKEN_BYTES {
        return Err(format!(
            "random_token({bytes}) asks for more than {MAX_TOKEN_BYTES} random bytes; the argument is a number of bytes, and 32 is enough for any key"
        ));
    }
    let mut random = vec![0u8; usize::try_from(bytes).expect("bounded above")];
    crate::fill_random(&mut random)?;
    Ok(Secret::new(&crate::Bytes::new(&random).to_base64_url()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(token: &Secret) -> String {
        token.expose_text().expect("a token is text").to_string()
    }

    #[test]
    fn a_token_is_base64url_text_of_the_requested_bytes() {
        for (bytes, characters) in [(16, 22), (32, 43), (33, 44), (64, 86), (1024, 1366)] {
            let token = text(&random_token(bytes).unwrap());
            assert_eq!(token.len(), characters, "{bytes} bytes");
            assert!(token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{token}");
            let decoded = crate::Bytes::from_base64_url(&token).unwrap();
            assert_eq!(decoded.len() as i64, bytes);
        }
    }

    #[test]
    fn two_tokens_differ() {
        assert_ne!(random_token(32).unwrap(), random_token(32).unwrap());
    }

    #[test]
    fn a_token_never_formats_its_text() {
        let token = random_token(32).unwrap();
        assert_eq!(format!("{token} {token:?}"), "<secret> <secret>");
    }

    #[test]
    fn sizes_outside_the_bounds_are_refused() {
        for bytes in [i64::MIN, -1, 0, 1, 15] {
            let problem = random_token(bytes).unwrap_err();
            assert!(problem.contains("at least 16 random bytes"), "{problem}");
        }
        for bytes in [1025, 8192, i64::MAX] {
            let problem = random_token(bytes).unwrap_err();
            assert!(problem.contains("more than 1024 random bytes"), "{problem}");
        }
    }
}
