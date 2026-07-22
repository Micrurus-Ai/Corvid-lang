//! A local mock OpenID Connect provider for tests (slice 51k).
//!
//! `MockIdp` mints Ed25519-signed ID tokens and serves a matching
//! JWKS through a [`JwksFetcher`], so the full verification path —
//! `kid` resolution, signature check, `iss`/`aud`/`exp` validation —
//! runs end-to-end without a network or a real provider. The signing
//! key is DETERMINISTIC (a fixed seed), so tests are reproducible and
//! do not touch the system RNG.
//!
//! It also exposes lower-level `mint_*` helpers used by the
//! source-bypass mutator suite: each one deliberately breaks one
//! safe-default (drop the signature, claim `alg=none`, forge the
//! `kid`, swap the issuer/audience, backdate `exp`) so a test can
//! assert the verifier refuses the tampered token. The mutators live
//! next to the honest issuer on purpose — the guarantee is that the
//! ONLY token this mock produces that verifies is the fully-correct
//! one.

use super::jwks::{JsonWebKey, JsonWebKeySet, JwksFetcher};
use super::JwtVerifyError;
use crate::auth::JwtVerificationContract;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use std::sync::Mutex;

/// Public Corvid guarantee id this module's adversarial suite
/// enforces (slice 51k): `auth.jwt_tamper_and_fuzz_resistant`.
/// Declared as a literal so the `corvid-guarantees` inverse-coverage
/// sentinel links the mutator + byte-fuzz tests to the registry row.
#[allow(dead_code)]
pub const GUARANTEE_ID_JWT_TAMPER_AND_FUZZ: &str = "auth.jwt_tamper_and_fuzz_resistant";

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// A deterministic mock OIDC identity provider.
pub struct MockIdp {
    signing_key: SigningKey,
    kid: String,
    issuer: String,
    audience: String,
    jwks_url: String,
}

impl MockIdp {
    /// A mock provider for the given issuer + audience. The Ed25519
    /// key is derived from a fixed seed so every run mints the same
    /// keypair.
    pub fn new(issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        let issuer = issuer.into();
        let audience = audience.into();
        let jwks_url = format!("{issuer}/.well-known/jwks.json");
        Self {
            // Fixed seed → deterministic keypair. Test-only; never a
            // production key.
            signing_key: SigningKey::from_bytes(&[7u8; 32]),
            kid: "mock-idp-key-1".to_string(),
            issuer,
            audience,
            jwks_url,
        }
    }

    /// The verification contract a caller passes to `JwtVerifier`.
    pub fn contract(&self) -> JwtVerificationContract {
        JwtVerificationContract {
            issuer: self.issuer.clone(),
            audience: self.audience.clone(),
            jwks_url: self.jwks_url.clone(),
            algorithm: "EdDSA".to_string(),
            required_subject_claim: "sub".to_string(),
            required_tenant_claim: "tenant".to_string(),
            clock_skew_ms: 60_000,
        }
    }

    /// The JWKS document exposing this provider's public key.
    pub fn jwks(&self) -> JsonWebKeySet {
        let public = self.signing_key.verifying_key();
        JsonWebKeySet {
            keys: vec![JsonWebKey {
                kty: "OKP".to_string(),
                alg: Some("EdDSA".to_string()),
                kid: Some(self.kid.clone()),
                n: None,
                e: None,
                crv: Some("Ed25519".to_string()),
                x: Some(b64url(public.as_bytes())),
                y: None,
                r#use: Some("sig".to_string()),
            }],
        }
    }

    /// A `JwksFetcher` returning this provider's JWKS.
    pub fn fetcher(&self) -> MockIdpFetcher {
        MockIdpFetcher {
            jwks: Mutex::new(self.jwks()),
        }
    }

    /// Mint an honest, fully-valid ID token for `sub` in `tenant`.
    /// `exp_secs` / `iat_secs` are absolute unix seconds.
    pub fn mint(&self, sub: &str, tenant: &str, iat_secs: u64, exp_secs: u64) -> String {
        let header = serde_json::json!({
            "alg": "EdDSA",
            "typ": "JWT",
            "kid": self.kid,
        });
        let payload = serde_json::json!({
            "iss": self.issuer,
            "aud": self.audience,
            "sub": sub,
            "tenant": tenant,
            "iat": iat_secs,
            "exp": exp_secs,
        });
        self.sign(&header, &payload)
    }

    /// Mint a token whose payload is overridden field-by-field — the
    /// base is a valid token for `("user-1","tenant-1")` with a far
    /// future `exp`, then `mutate` edits the JSON before signing.
    pub fn mint_with(&self, mutate: impl FnOnce(&mut serde_json::Value)) -> String {
        let header = serde_json::json!({ "alg": "EdDSA", "typ": "JWT", "kid": self.kid });
        let mut payload = serde_json::json!({
            "iss": self.issuer,
            "aud": self.audience,
            "sub": "user-1",
            "tenant": "tenant-1",
            "iat": 1_000,
            "exp": 4_000_000_000u64,
        });
        mutate(&mut payload);
        self.sign(&header, &payload)
    }

    // ---- source-bypass mutators (slice 51k) ----

    /// A token claiming `alg=none` with an empty signature segment.
    pub fn mint_alg_none(&self) -> String {
        let header = serde_json::json!({ "alg": "none", "typ": "JWT", "kid": self.kid });
        let payload = serde_json::json!({
            "iss": self.issuer, "aud": self.audience,
            "sub": "user-1", "tenant": "tenant-1", "iat": 1_000, "exp": 4_000_000_000u64,
        });
        let signing_input = format!(
            "{}.{}",
            b64url(&serde_json::to_vec(&header).unwrap()),
            b64url(&serde_json::to_vec(&payload).unwrap())
        );
        format!("{signing_input}.")
    }

    /// A valid token with its signature's last byte flipped.
    pub fn mint_tampered_signature(&self) -> String {
        let token = self.mint("user-1", "tenant-1", 1_000, 4_000_000_000);
        let (rest, sig_b64) = token.rsplit_once('.').unwrap();
        let mut sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig_b64)
            .unwrap();
        let last = sig.len() - 1;
        sig[last] ^= 0x01;
        format!("{rest}.{}", b64url(&sig))
    }

    /// A correctly-signed token whose header names a `kid` the JWKS
    /// does not contain.
    pub fn mint_forged_kid(&self) -> String {
        let header = serde_json::json!({ "alg": "EdDSA", "typ": "JWT", "kid": "attacker-kid" });
        let payload = serde_json::json!({
            "iss": self.issuer, "aud": self.audience,
            "sub": "user-1", "tenant": "tenant-1", "iat": 1_000, "exp": 4_000_000_000u64,
        });
        self.sign(&header, &payload)
    }

    fn sign(&self, header: &serde_json::Value, payload: &serde_json::Value) -> String {
        let signing_input = format!(
            "{}.{}",
            b64url(&serde_json::to_vec(header).unwrap()),
            b64url(&serde_json::to_vec(payload).unwrap())
        );
        let signature = self.signing_key.sign(signing_input.as_bytes());
        format!("{signing_input}.{}", b64url(&signature.to_bytes()))
    }
}

/// A `JwksFetcher` backed by a [`MockIdp`]'s keys.
pub struct MockIdpFetcher {
    jwks: Mutex<JsonWebKeySet>,
}

impl JwksFetcher for MockIdpFetcher {
    fn fetch(&self, _jwks_url: &str) -> Result<JsonWebKeySet, JwtVerifyError> {
        Ok(self.jwks.lock().unwrap().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt_verify::JwtVerifier;
    use std::sync::Arc;

    fn idp() -> MockIdp {
        MockIdp::new("https://issuer.test", "corvid-test")
    }

    /// The honest token the mock issues verifies end-to-end through
    /// the real `JwtVerifier` (slice 51k). Establishes that the mock
    /// IdP and the verifier agree on the Ed25519 signing path.
    #[test]
    fn mock_idp_token_verifies_end_to_end() {
        let idp = idp();
        let verifier = JwtVerifier::new(Arc::new(idp.fetcher()));
        let token = idp.mint("user-1", "tenant-1", 1_000, 4_000_000_000);
        let claims = verifier
            .verify(&token, &idp.contract(), 2_000_000)
            .expect("honest token verifies");
        assert_eq!(claims.subject, "user-1");
        assert_eq!(claims.tenant, "tenant-1");
        assert_eq!(claims.issuer, "https://issuer.test");
    }

    /// Source-bypass mutators (slice 51k): every tampered token the
    /// mock can produce is REFUSED. This is the adversarial heart of
    /// the identity block — the safe-defaults cannot be bypassed by
    /// dropping the signature, downgrading the algorithm, forging the
    /// key id, swapping issuer/audience, or backdating expiry.
    #[test]
    fn every_mutated_token_is_refused() {
        let idp = idp();
        let verifier = JwtVerifier::new(Arc::new(idp.fetcher()));
        let contract = idp.contract();
        let now = 2_000_000u64;

        let cases: Vec<(&str, String)> = vec![
            ("alg_none", idp.mint_alg_none()),
            ("tampered_signature", idp.mint_tampered_signature()),
            ("forged_kid", idp.mint_forged_kid()),
            ("wrong_issuer", idp.mint_with(|p| p["iss"] = "https://evil.test".into())),
            ("wrong_audience", idp.mint_with(|p| p["aud"] = "someone-else".into())),
            ("expired", idp.mint_with(|p| p["exp"] = serde_json::json!(1_500))),
        ];

        for (name, token) in cases {
            let result = verifier.verify(&token, &contract, now);
            assert!(
                result.is_err(),
                "mutated token `{name}` must be refused, but it verified: {result:?}"
            );
        }
    }

    /// Byte-fuzz (slice 51k): feed the verifier a deterministic stream
    /// of malformed byte sequences and assert it NEVER panics — every
    /// input yields a clean `Err`, never an abort or a success. A
    /// parser that panics on adversarial input is a denial-of-service
    /// hole; this proves the JWT front door degrades gracefully.
    #[test]
    fn byte_fuzz_never_panics_and_never_forges() {
        let idp = idp();
        let verifier = JwtVerifier::new(Arc::new(idp.fetcher()));
        let contract = idp.contract();

        // Deterministic xorshift PRNG — no system RNG, reproducible.
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        for _ in 0..2_000 {
            let len = (next() % 300) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
            // Lossily reinterpret as a token string; also try a
            // dotted three-segment shape built from random base64.
            let as_str = String::from_utf8_lossy(&bytes).into_owned();
            let _ = verifier.verify(&as_str, &contract, 2_000_000);

            let seg = |n: usize| b64url(&bytes.iter().rev().take(n).cloned().collect::<Vec<_>>());
            let dotted = format!("{}.{}.{}", seg(20), seg(40), seg(64));
            let result = verifier.verify(&dotted, &contract, 2_000_000);
            // A random dotted token must never verify.
            assert!(result.is_err(), "random token unexpectedly verified: {dotted}");
        }
    }
}
