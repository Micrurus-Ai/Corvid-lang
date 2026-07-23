//! Per-login cryptographic material (slice 52e): the PKCE verifier +
//! challenge, the CSRF `state`, and the OIDC `nonce`.
//!
//! Each is generated from the OS CSPRNG for every login. The verifier is
//! kept server-side (on the single-use OAuth state row) and replayed to
//! the token endpoint; the challenge, state, and nonce travel to the
//! provider. PKCE (RFC 7636, S256) binds the authorization code to the
//! client that started the flow, so an intercepted code cannot be
//! redeemed by anyone else.

use base64::Engine;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

/// A URL-safe, unpadded base64 encoding of 32 fresh random bytes — 43
/// characters, the shape RFC 7636 recommends for a PKCE verifier and a
/// good size for opaque state / nonce / session-id / token values too.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The material a single login mints.
#[derive(Debug, Clone)]
pub struct LoginMaterial {
    /// The PKCE code verifier — kept server-side, replayed at token
    /// exchange. Never sent to the browser.
    pub verifier: String,
    /// The PKCE code challenge (`S256`) — sent to the authorize endpoint.
    pub challenge: String,
    /// The opaque CSRF state — round-trips through the provider and is
    /// the single-use OAuth state key.
    pub state: String,
    /// The OIDC nonce — sent to the authorize endpoint, returned in the
    /// ID token, and fingerprinted onto the OAuth state.
    pub nonce: String,
}

impl LoginMaterial {
    /// Mint fresh material for one login.
    pub fn generate() -> Self {
        let verifier = random_token();
        let challenge = code_challenge_s256(&verifier);
        Self {
            verifier,
            challenge,
            state: random_token(),
            nonce: random_token(),
        }
    }
}

/// The RFC 7636 `S256` code challenge for a verifier:
/// `base64url(sha256(verifier))`, unpadded.
pub fn code_challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_is_fresh_each_time_and_well_formed() {
        let a = LoginMaterial::generate();
        let b = LoginMaterial::generate();
        // Overwhelmingly unlikely to collide; proves we draw fresh bytes.
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
        assert_ne!(a.nonce, b.nonce);
        // 32 bytes → 43 unpadded base64url chars.
        assert_eq!(a.verifier.len(), 43);
        assert!(!a.verifier.contains('='));
        assert!(!a.verifier.contains('+'));
        assert!(!a.verifier.contains('/'));
    }

    #[test]
    fn challenge_is_the_s256_of_the_verifier() {
        let material = LoginMaterial::generate();
        assert_eq!(material.challenge, code_challenge_s256(&material.verifier));
        // A known vector from RFC 7636 §appendix B.
        assert_eq!(
            code_challenge_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
