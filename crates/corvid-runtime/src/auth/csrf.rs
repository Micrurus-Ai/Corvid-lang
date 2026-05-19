//! Double-submit CSRF protection with HMAC-SHA256 binding.
//!
//! Stateless verification: a CSRF token is the string
//! `<binding>.<hex_hmac>` where `binding` is an opaque
//! caller-chosen id (typically the session id) and `hex_hmac` is
//! `HMAC-SHA256(server_secret, "corvid-csrf-v1:" || binding)`.
//! The token is set in a cookie at session-issue time AND
//! returned to the client so it can be echoed back as a header
//! on every state-changing request.
//!
//! The verifier enforces three independent checks on
//! state-changing methods (POST / PUT / PATCH / DELETE):
//!
//!   1. Both the header token and the cookie token are present.
//!   2. They are equal (constant-time comparison) — the
//!      double-submit invariant. A cross-site request cannot
//!      read the cookie, so the attacker cannot supply a
//!      matching header.
//!   3. The HMAC component verifies against the server secret —
//!      so a token forged without the secret is rejected even
//!      if the attacker has somehow planted matching values in
//!      both cookie and header.
//!
//! Safe methods (GET / HEAD / OPTIONS) are passed through
//! without a token check — they are not state-changing.
//!
//! Catches the `CSRF-bypass-on-PUT/PATCH/DELETE` threat from
//! the Phase 39 adversarial corpus: a state-changing request
//! that lacks the CSRF header — or carries a forged one — is
//! refused before reaching the handler.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

const CSRF_BINDING_DOMAIN: &[u8] = b"corvid-csrf-v1:";

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-coverage
/// sentinel. Names the registry id whose runtime enforcement
/// lives in `verify_csrf_double_submit` below.
#[allow(dead_code)]
pub const GUARANTEE_ID_CSRF_DOUBLE_SUBMIT: &str = "auth.csrf_double_submit";

/// HTTP method classification for CSRF purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsrfRequestMethod {
    /// `GET`, `HEAD`, `OPTIONS` — read-only, no token required.
    Safe,
    /// `POST`, `PUT`, `PATCH`, `DELETE` — state-changing, token
    /// required.
    StateChanging,
}

impl CsrfRequestMethod {
    /// Classify an uppercase HTTP method string. Unknown methods
    /// are treated as state-changing — fail closed.
    pub fn classify(method: &str) -> Self {
        match method {
            "GET" | "HEAD" | "OPTIONS" => Self::Safe,
            _ => Self::StateChanging,
        }
    }
}

/// Reasons a CSRF verification can fail. Every variant produces
/// a distinct error message so the operator can attribute the
/// rejection in the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CsrfError {
    /// State-changing request without the `X-Corvid-CSRF` header.
    MissingHeader,
    /// State-changing request without the `corvid_csrf` cookie.
    MissingCookie,
    /// Header and cookie tokens did not match (the double-submit
    /// invariant). The most common shape for a cross-site
    /// request: the attacker controls the header but cannot read
    /// the cookie.
    HeaderCookieMismatch,
    /// Token is not in the `<binding>.<hex_hmac>` shape.
    Malformed,
    /// HMAC component does not verify against the server secret.
    /// A forged token (no knowledge of the secret) fails here.
    InvalidHmac,
    /// Server CSRF secret is empty — refuse to verify rather
    /// than silently accept (fail closed).
    EmptySecret,
}

impl std::fmt::Display for CsrfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            Self::MissingHeader => "CSRF header missing",
            Self::MissingCookie => "CSRF cookie missing",
            Self::HeaderCookieMismatch => "CSRF header and cookie do not match",
            Self::Malformed => "CSRF token is malformed",
            Self::InvalidHmac => "CSRF token failed HMAC verification",
            Self::EmptySecret => "server CSRF secret is empty",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for CsrfError {}

/// Mint a fresh CSRF token bound to `binding_id`. The token is
/// safe to set in a `Set-Cookie` header and to return to the
/// client for echoing back as `X-Corvid-CSRF` on state-changing
/// requests.
pub fn mint_csrf_token(binding_id: &str, server_secret: &[u8]) -> Result<String, CsrfError> {
    if server_secret.is_empty() {
        return Err(CsrfError::EmptySecret);
    }
    let mac = compute_hmac(binding_id, server_secret)?;
    Ok(format!("{binding_id}.{}", hex_encode(&mac)))
}

/// Verify a double-submit CSRF token on the supplied method.
/// Safe methods always succeed; state-changing methods require
/// the three independent checks listed in the module doc.
pub fn verify_csrf_double_submit(
    method: CsrfRequestMethod,
    header_token: Option<&str>,
    cookie_token: Option<&str>,
    server_secret: &[u8],
) -> Result<(), CsrfError> {
    if method == CsrfRequestMethod::Safe {
        return Ok(());
    }
    if server_secret.is_empty() {
        return Err(CsrfError::EmptySecret);
    }
    let header = header_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(CsrfError::MissingHeader)?;
    let cookie = cookie_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(CsrfError::MissingCookie)?;
    if header.as_bytes().ct_eq(cookie.as_bytes()).unwrap_u8() == 0 {
        return Err(CsrfError::HeaderCookieMismatch);
    }
    let (binding, supplied_hex) = header.split_once('.').ok_or(CsrfError::Malformed)?;
    if binding.is_empty() || supplied_hex.is_empty() {
        return Err(CsrfError::Malformed);
    }
    let supplied = hex_decode(supplied_hex).ok_or(CsrfError::Malformed)?;
    let expected = compute_hmac(binding, server_secret)?;
    if supplied.ct_eq(&expected).unwrap_u8() == 0 {
        return Err(CsrfError::InvalidHmac);
    }
    Ok(())
}

fn compute_hmac(binding_id: &str, server_secret: &[u8]) -> Result<Vec<u8>, CsrfError> {
    if server_secret.is_empty() {
        return Err(CsrfError::EmptySecret);
    }
    let mut mac =
        HmacSha256::new_from_slice(server_secret).map_err(|_| CsrfError::EmptySecret)?;
    mac.update(CSRF_BINDING_DOMAIN);
    mac.update(binding_id.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn hex_decode(input: &str) -> Option<Vec<u8>> {
    if !input.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(input.len() / 2);
    let bytes = input.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let hi = decode_nibble(pair[0])?;
        let lo = decode_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"server-side-csrf-secret-for-tests-only";

    #[test]
    fn safe_methods_skip_csrf_check_even_without_tokens() {
        // GET / HEAD / OPTIONS pass without any token — they are
        // not state-changing.
        for method in ["GET", "HEAD", "OPTIONS"] {
            let kind = CsrfRequestMethod::classify(method);
            assert_eq!(kind, CsrfRequestMethod::Safe);
            verify_csrf_double_submit(kind, None, None, SECRET).unwrap();
        }
    }

    #[test]
    fn mint_and_verify_round_trip_on_each_state_changing_method() {
        // POST / PUT / PATCH / DELETE all require the double-
        // submit pair and accept a freshly-minted token.
        let token = mint_csrf_token("sess-1", SECRET).unwrap();
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let kind = CsrfRequestMethod::classify(method);
            assert_eq!(kind, CsrfRequestMethod::StateChanging);
            verify_csrf_double_submit(kind, Some(&token), Some(&token), SECRET).unwrap();
        }
    }

    /// Slice 35V2-P39-C-LR (named-threat: CSRF-bypass-on-
    /// PUT/PATCH/DELETE): a state-changing request that omits
    /// the CSRF header is refused. This is the central
    /// adversarial contract.
    #[test]
    fn csrf_bypass_attempt_without_header_refused_on_put_patch_delete() {
        let token = mint_csrf_token("sess-1", SECRET).unwrap();
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let kind = CsrfRequestMethod::classify(method);
            // Cookie present, header missing — the classic
            // cross-site request shape.
            let err =
                verify_csrf_double_submit(kind, None, Some(&token), SECRET).unwrap_err();
            assert_eq!(err, CsrfError::MissingHeader, "method {method}");
        }
    }

    /// Slice 35V2-P39-C-LR (adversarial): the double-submit
    /// invariant. An attacker who controls the header but cannot
    /// read the victim's cookie supplies different values; the
    /// request is refused on equality alone, before the HMAC
    /// check even runs.
    #[test]
    fn csrf_header_and_cookie_must_match_constant_time() {
        let token = mint_csrf_token("sess-1", SECRET).unwrap();
        let other = mint_csrf_token("sess-2", SECRET).unwrap();
        let err = verify_csrf_double_submit(
            CsrfRequestMethod::StateChanging,
            Some(&token),
            Some(&other),
            SECRET,
        )
        .unwrap_err();
        assert_eq!(err, CsrfError::HeaderCookieMismatch);
    }

    /// Slice 35V2-P39-C-LR (adversarial): a token forged without
    /// the server secret fails HMAC verification even if header
    /// and cookie match (an attacker who controls both surfaces
    /// but does not know the secret cannot mint a valid token).
    #[test]
    fn csrf_token_forged_without_server_secret_refused_on_hmac() {
        let forged = "sess-1.deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let err = verify_csrf_double_submit(
            CsrfRequestMethod::StateChanging,
            Some(forged),
            Some(forged),
            SECRET,
        )
        .unwrap_err();
        assert_eq!(err, CsrfError::InvalidHmac);
    }

    /// Slice 35V2-P39-C-LR (adversarial): a malformed token
    /// (missing the binding.hmac shape) is refused as Malformed,
    /// not silently truncated or parsed past the bug.
    #[test]
    fn csrf_token_without_binding_dot_separator_refused_as_malformed() {
        let bad = "no-separator-at-all";
        let err = verify_csrf_double_submit(
            CsrfRequestMethod::StateChanging,
            Some(bad),
            Some(bad),
            SECRET,
        )
        .unwrap_err();
        assert_eq!(err, CsrfError::Malformed);

        let bad2 = "binding.notvalidhex!!";
        let err2 = verify_csrf_double_submit(
            CsrfRequestMethod::StateChanging,
            Some(bad2),
            Some(bad2),
            SECRET,
        )
        .unwrap_err();
        assert_eq!(err2, CsrfError::Malformed);
    }

    /// Slice 35V2-P39-C-LR (adversarial): an empty server secret
    /// fails closed — refuses to verify rather than silently
    /// passing every request. Catches the misconfiguration where
    /// `CORVID_CSRF_SECRET` is unset in production.
    #[test]
    fn csrf_empty_server_secret_fails_closed_on_state_changing_methods() {
        let token = "sess-1.00";
        let err = verify_csrf_double_submit(
            CsrfRequestMethod::StateChanging,
            Some(token),
            Some(token),
            b"",
        )
        .unwrap_err();
        assert_eq!(err, CsrfError::EmptySecret);
        // Safe methods still pass — they don't touch the secret.
        verify_csrf_double_submit(CsrfRequestMethod::Safe, None, None, b"").unwrap();
        // Minting against an empty secret also fails closed.
        assert_eq!(
            mint_csrf_token("sess-1", b"").unwrap_err(),
            CsrfError::EmptySecret
        );
    }

    #[test]
    fn unknown_methods_fail_closed_as_state_changing() {
        // A non-standard method like CONNECT or TRACE is treated
        // as state-changing — never silently passed.
        assert_eq!(
            CsrfRequestMethod::classify("CONNECT"),
            CsrfRequestMethod::StateChanging
        );
        let err = verify_csrf_double_submit(
            CsrfRequestMethod::classify("CONNECT"),
            None,
            None,
            SECRET,
        )
        .unwrap_err();
        assert_eq!(err, CsrfError::MissingHeader);
    }
}
