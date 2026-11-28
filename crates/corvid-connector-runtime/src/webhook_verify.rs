//! Per-provider webhook signature verifier — slice 41M-A, widened by
//! verified webhooks (B16).
//!
//! Inbound webhook payloads from third-party providers carry their
//! signature in a provider-specific header with a provider-specific
//! signing scheme. This module is the one verifier for every scheme
//! Corvid knows; a `verified <scheme>(...)` route clause in `corvid
//! serve` and `corvid connectors verify-webhook` both call it:
//!
//!   - **Stripe** (`Stripe-Signature: t=<ts>,v1=<hex>[,v1=<hex>]`):
//!     HMAC-SHA256 over `<ts>.<body>` with the signing secret's text.
//!   - **Standard Webhooks** (`webhook-id`, `webhook-timestamp`,
//!     `webhook-signature: v1,<base64> [v1,<base64>]`): HMAC-SHA256 over
//!     `<id>.<ts>.<body>` with the key written in base64 after `whsec_`.
//!   - **Slack** (`X-Slack-Signature` + `X-Slack-Request-Timestamp`):
//!     HMAC-SHA256 over `v0:<ts>:<body>`, hex, prefixed with `v0=`.
//!   - **GitHub** (`X-Hub-Signature-256`): HMAC-SHA256 over the raw
//!     body, hex, prefixed with `sha256=`. GitHub signs no timestamp.
//!   - **Linear** (`Linear-Signature`): HMAC-SHA256 over the raw body,
//!     bare hex. Linear's header signs no timestamp.
//!
//! The replay window is never a default: every call states a
//! [`ReplayWindow`], and a window that does not fit the scheme (none for
//! a scheme that signs a time, one for a scheme that signs none) is
//! refused. The signature is checked before the timestamp, so a request
//! that is not from the provider learns only that its signature is bad.
//! Every signature a header carries is tried, so a provider rolling its
//! key keeps working. Tags are compared as bytes, in constant time, with
//! `corvid-crypto`'s comparison; the key stays a wiped `Secret`.

use corvid_crypto::{hmac_sha256, Bytes, HmacKey, Secret};
use std::collections::BTreeMap;

/// Public Corvid guarantee id this webhook verifier enforces:
/// `connector.webhook_signature_verified`.
///
/// Inbound webhook payloads are HMAC-SHA256 verified against the
/// declared shared secret before any handler runs. Per-provider schemes
/// are honored (Stripe, Standard Webhooks, Slack, GitHub, Linear), the
/// replay window is declared by the caller and checked for every scheme
/// that signs a time, and comparison is constant-time. A malformed
/// header, mismatched digest, or stale timestamp returns a categorical
/// `WebhookVerificationOutcome` that the dispatcher must reject before
/// any side effect. Tagged at module level so the `corvid-guarantees`
/// inverse-coverage sentinel can confirm the enforcement site is wired
/// to the registry row.
pub const GUARANTEE_ID_WEBHOOK_SIGNATURE_VERIFIED: &str = "connector.webhook_signature_verified";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookProvider {
    Stripe,
    StandardWebhooks,
    Slack,
    GitHub,
    Linear,
}

impl WebhookProvider {
    /// Every scheme, in the order error messages list them.
    pub const ALL: [WebhookProvider; 5] = [
        WebhookProvider::Stripe,
        WebhookProvider::StandardWebhooks,
        WebhookProvider::Slack,
        WebhookProvider::GitHub,
        WebhookProvider::Linear,
    ];

    pub fn slug(&self) -> &'static str {
        match self {
            WebhookProvider::Stripe => "stripe",
            WebhookProvider::StandardWebhooks => "standard_webhooks",
            WebhookProvider::Slack => "slack",
            WebhookProvider::GitHub => "github",
            WebhookProvider::Linear => "linear",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| provider.slug() == slug)
    }

    /// Whether the scheme signs a timestamp, so a replay window can be
    /// checked. A scheme that signs none cannot tell a replayed request
    /// from a fresh one.
    pub fn signs_time(&self) -> bool {
        matches!(self, WebhookProvider::Stripe | WebhookProvider::StandardWebhooks | WebhookProvider::Slack)
    }

    /// The request headers the scheme reads, as providers spell them.
    pub fn signature_headers(&self) -> &'static [&'static str] {
        match self {
            WebhookProvider::Stripe => &["Stripe-Signature"],
            WebhookProvider::StandardWebhooks => &["webhook-id", "webhook-timestamp", "webhook-signature"],
            WebhookProvider::Slack => &["X-Slack-Signature", "X-Slack-Request-Timestamp"],
            WebhookProvider::GitHub => &["X-Hub-Signature-256"],
            WebhookProvider::Linear => &["Linear-Signature"],
        }
    }

    /// The header naming one delivery, where the scheme sends one, so a
    /// trace can say which event arrived without recording its body.
    pub fn delivery_id_header(&self) -> Option<&'static str> {
        match self {
            WebhookProvider::StandardWebhooks => Some("webhook-id"),
            WebhookProvider::GitHub => Some("X-GitHub-Delivery"),
            WebhookProvider::Linear => Some("Linear-Delivery"),
            WebhookProvider::Stripe | WebhookProvider::Slack => None,
        }
    }

    /// The HMAC key the scheme signs with, from the secret as the
    /// provider hands it out: the text itself, or, for Standard
    /// Webhooks, the bytes written in base64 after `whsec_`. A server
    /// calls this at startup so a malformed secret refuses to start
    /// instead of failing every request. The error never quotes the
    /// secret.
    pub fn signing_key(&self, secret: &Secret) -> Result<Secret, String> {
        match self {
            WebhookProvider::StandardWebhooks => secret.decode_base64_after("whsec_"),
            _ => Ok(secret.clone()),
        }
    }
}

/// How old a signed request may be. Stated by every caller: there is no
/// default window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayWindow {
    /// The signed timestamp must be within this many milliseconds of
    /// the clock, in either direction.
    Within { max_age_ms: u64 },
    /// The scheme signs no time, and the caller accepts that a recorded
    /// request can be sent again.
    ReplaysAccepted,
}

/// Outcome of a webhook verification. The HTTP layer maps each
/// variant to a status code: `Verified` → the handler runs,
/// `BadSignature`, `Stale` and `Malformed` → 401.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookVerificationOutcome {
    Verified,
    BadSignature,
    /// The signature matched but the signed timestamp was outside the
    /// declared window. Replay protection.
    Stale {
        provided_ts_ms: u64,
        now_ms: u64,
        delta_ms: u64,
    },
    /// Required header missing or in an unparseable shape, or a window
    /// that does not fit the scheme.
    Malformed {
        reason: String,
    },
}

impl WebhookVerificationOutcome {
    pub fn is_verified(&self) -> bool {
        matches!(self, WebhookVerificationOutcome::Verified)
    }
}

/// Inputs to a webhook verification. Headers are case-insensitive
/// per RFC 9110 — keys are normalised to lowercase before lookup.
#[derive(Debug, Clone)]
pub struct WebhookVerifyInputs<'a> {
    pub provider: WebhookProvider,
    pub headers: &'a BTreeMap<String, String>,
    pub body: &'a [u8],
    /// The secret as the provider hands it out; the scheme's key is
    /// derived from it by [`WebhookProvider::signing_key`].
    pub secret: &'a Secret,
    pub now_ms: u64,
    pub window: ReplayWindow,
}

impl<'a> WebhookVerifyInputs<'a> {
    pub fn new(
        provider: WebhookProvider,
        headers: &'a BTreeMap<String, String>,
        body: &'a [u8],
        secret: &'a Secret,
        now_ms: u64,
        window: ReplayWindow,
    ) -> Self {
        Self { provider, headers, body, secret, now_ms, window }
    }
}

/// Top-level entrypoint: dispatch by provider.
pub fn verify_webhook(inputs: WebhookVerifyInputs<'_>) -> WebhookVerificationOutcome {
    let provider = inputs.provider;
    match (provider.signs_time(), inputs.window) {
        (true, ReplayWindow::ReplaysAccepted) => {
            return malformed(format!("`{}` signs a timestamp, so a replay window is required", provider.slug()));
        }
        (false, ReplayWindow::Within { .. }) => {
            return malformed(format!(
                "`{}` signs no timestamp, so a replay window cannot be checked; replays must be accepted explicitly",
                provider.slug()
            ));
        }
        _ => {}
    }
    let key = match provider.signing_key(inputs.secret) {
        Ok(key) => key,
        Err(reason) => return malformed(reason),
    };
    let signed = match signed_request(&inputs) {
        Ok(signed) => signed,
        Err(reason) => return malformed(reason),
    };
    let expected = hmac_sha256(&Bytes::new(&signed.payload), HmacKey::Secret(&key));
    if !signed.signatures.iter().any(|received| received.constant_time_eq(&expected)) {
        return WebhookVerificationOutcome::BadSignature;
    }
    if let (Some(provided_ts_s), ReplayWindow::Within { max_age_ms }) = (signed.timestamp_s, inputs.window) {
        let provided_ts_ms = provided_ts_s.saturating_mul(1_000);
        let delta_ms = inputs.now_ms.abs_diff(provided_ts_ms);
        if delta_ms > max_age_ms {
            return WebhookVerificationOutcome::Stale { provided_ts_ms, now_ms: inputs.now_ms, delta_ms };
        }
    }
    WebhookVerificationOutcome::Verified
}

/// What a scheme signs and the signatures the request carries.
struct SignedRequest {
    payload: Vec<u8>,
    /// The signed time in Unix seconds, for a scheme that signs one.
    timestamp_s: Option<u64>,
    /// Every signature the header carries; one matching is enough.
    signatures: Vec<Bytes>,
}

fn signed_request(inputs: &WebhookVerifyInputs<'_>) -> Result<SignedRequest, String> {
    let header = |name: &str| header_lower(inputs.headers, name).ok_or_else(|| format!("missing {name} header"));
    let body = inputs.body;
    match inputs.provider {
        WebhookProvider::Stripe => {
            let value = header("Stripe-Signature")?;
            let mut timestamp = None;
            let mut signatures = Vec::new();
            for part in value.split(',') {
                match part.trim().split_once('=') {
                    Some(("t", t)) if timestamp.is_none() => timestamp = Some(t.trim()),
                    Some(("v1", hex)) => signatures.extend(Bytes::from_hex(hex.trim()).ok()),
                    _ => {}
                }
            }
            let timestamp = timestamp.ok_or("Stripe-Signature has no `t=` timestamp")?;
            Ok(SignedRequest {
                payload: joined(&[timestamp.as_bytes(), b".", body]),
                timestamp_s: Some(parse_seconds("Stripe-Signature `t=`", timestamp)?),
                signatures,
            })
        }
        WebhookProvider::StandardWebhooks => {
            let id = header("webhook-id")?.trim();
            let timestamp = header("webhook-timestamp")?.trim();
            let signatures = header("webhook-signature")?
                .split_whitespace()
                .filter_map(|entry| entry.strip_prefix("v1,"))
                .filter_map(|encoded| Bytes::from_base64(encoded).ok())
                .collect();
            Ok(SignedRequest {
                payload: joined(&[id.as_bytes(), b".", timestamp.as_bytes(), b".", body]),
                timestamp_s: Some(parse_seconds("webhook-timestamp", timestamp)?),
                signatures,
            })
        }
        WebhookProvider::Slack => {
            let signature = header("X-Slack-Signature")?;
            let timestamp = header("X-Slack-Request-Timestamp")?.trim();
            let hex = signature.trim().strip_prefix("v0=").ok_or("X-Slack-Signature missing `v0=` prefix")?;
            Ok(SignedRequest {
                payload: joined(&[b"v0:", timestamp.as_bytes(), b":", body]),
                timestamp_s: Some(parse_seconds("X-Slack-Request-Timestamp", timestamp)?),
                signatures: Bytes::from_hex(hex).ok().into_iter().collect(),
            })
        }
        WebhookProvider::GitHub => {
            let value = header("X-Hub-Signature-256")?;
            let hex = value.trim().strip_prefix("sha256=").ok_or("X-Hub-Signature-256 missing `sha256=` prefix")?;
            Ok(SignedRequest { payload: body.to_vec(), timestamp_s: None, signatures: Bytes::from_hex(hex).ok().into_iter().collect() })
        }
        WebhookProvider::Linear => {
            let hex = header("Linear-Signature")?.trim();
            Ok(SignedRequest { payload: body.to_vec(), timestamp_s: None, signatures: Bytes::from_hex(hex).ok().into_iter().collect() })
        }
    }
}

fn malformed(reason: impl Into<String>) -> WebhookVerificationOutcome {
    WebhookVerificationOutcome::Malformed { reason: reason.into() }
}

fn joined(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// A signed time in whole Unix seconds, as every scheme sends it.
fn parse_seconds(what: &str, text: &str) -> Result<u64, String> {
    text.parse().map_err(|_| format!("{what} `{text}` is not a whole number of seconds"))
}

fn header_lower<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hmac_hex(secret: &[u8], body: &[u8]) -> String {
        hmac_sha256(&Bytes::new(body), HmacKey::Bytes(&Bytes::new(secret))).to_hex()
    }

    fn secret(text: &str) -> Secret {
        Secret::new(text)
    }

    fn verify(
        provider: WebhookProvider,
        headers: &[(&str, &str)],
        body: &[u8],
        key: &str,
        now_ms: u64,
        window: ReplayWindow,
    ) -> WebhookVerificationOutcome {
        let headers: BTreeMap<String, String> =
            headers.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
        let key = secret(key);
        verify_webhook(WebhookVerifyInputs::new(provider, &headers, body, &key, now_ms, window))
    }

    const FIVE_MINUTES: ReplayWindow = ReplayWindow::Within { max_age_ms: 5 * 60 * 1_000 };

    fn compute_slack_signature(secret: &[u8], ts_s: u64, body: &[u8]) -> String {
        let basestring = joined(&[b"v0:", ts_s.to_string().as_bytes(), b":", body]);
        format!("v0={}", hmac_hex(secret, &basestring))
    }

    // Recorded requests: the same vectors `corvid-driver`'s
    // `webhook_verification.rs` checks in Corvid code, computed with
    // Python's `hmac` module and `openssl dgst -sha256 -hmac`.
    const BODY: &str = r#"{"id":"evt_1QZ","type":"payment_intent.succeeded","data":{"object":{"id":"pi_3Q","amount":2000,"currency":"usd"}}}"#;
    const TIMESTAMP_S: u64 = 1_732_480_000;
    const STRIPE_KEY: &str = "whsec_corvid_test_4eC39HqLyjWDarjtT1zdp7dc";
    const STRIPE_SIGNATURE: &str = "9d4aa1ff5638dd3708a9c29539870fc8fc470059f7feeb7ff559ab641d7f6dce";
    const STANDARD_KEY: &str = "whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYn";
    const STANDARD_MESSAGE_ID: &str = "msg_2KWPBgLlAfxdpx2AI54pPJ85f4W";
    const STANDARD_SIGNATURE: &str = "v1,+dJkrlylFBxSVAn/YI6mFfwStrqxrlMnbfCAVbmRja4=";

    fn stripe(header: &str, body: &str, now_s: u64) -> WebhookVerificationOutcome {
        verify(WebhookProvider::Stripe, &[("Stripe-Signature", header)], body.as_bytes(), STRIPE_KEY, now_s * 1_000, FIVE_MINUTES)
    }

    fn standard(id: &str, timestamp: &str, signature: &str, body: &str, key: &str) -> WebhookVerificationOutcome {
        verify(
            WebhookProvider::StandardWebhooks,
            &[("webhook-id", id), ("webhook-timestamp", timestamp), ("webhook-signature", signature)],
            body.as_bytes(),
            key,
            TIMESTAMP_S * 1_000,
            FIVE_MINUTES,
        )
    }

    #[test]
    fn a_recorded_stripe_request_verifies_and_every_tampered_field_is_refused() {
        let header = format!("t={TIMESTAMP_S},v1={STRIPE_SIGNATURE}");
        assert!(stripe(&header, BODY, TIMESTAMP_S).is_verified());
        assert_eq!(stripe(&header, &BODY.replacen("2000", "9000", 1), TIMESTAMP_S), WebhookVerificationOutcome::BadSignature);
        assert_eq!(stripe(&format!("t=1732480001,v1={STRIPE_SIGNATURE}"), BODY, TIMESTAMP_S), WebhookVerificationOutcome::BadSignature);
        let flipped = format!("t={TIMESTAMP_S},v1={}f", &STRIPE_SIGNATURE[..63]);
        assert_eq!(stripe(&flipped, BODY, TIMESTAMP_S), WebhookVerificationOutcome::BadSignature);
        assert_eq!(stripe(&format!("t={TIMESTAMP_S}"), BODY, TIMESTAMP_S), WebhookVerificationOutcome::BadSignature);
        assert!(matches!(stripe(&format!("v1={STRIPE_SIGNATURE}"), BODY, TIMESTAMP_S), WebhookVerificationOutcome::Malformed { .. }));
    }

    #[test]
    fn a_stripe_key_being_rolled_verifies_with_either_signature_and_uppercase_hex() {
        let rolled = format!("t={TIMESTAMP_S},v1={},v1={STRIPE_SIGNATURE}", "0".repeat(64));
        assert!(stripe(&rolled, BODY, TIMESTAMP_S).is_verified());
        let upper = format!("t={TIMESTAMP_S},v1={}", STRIPE_SIGNATURE.to_ascii_uppercase());
        assert!(stripe(&upper, BODY, TIMESTAMP_S).is_verified(), "hex is read in either case");
    }

    #[test]
    fn a_stripe_request_outside_the_window_is_stale_and_inside_it_verifies() {
        let header = format!("t={TIMESTAMP_S},v1={STRIPE_SIGNATURE}");
        assert!(stripe(&header, BODY, TIMESTAMP_S + 300).is_verified(), "exactly at the edge");
        assert!(matches!(stripe(&header, BODY, TIMESTAMP_S + 301), WebhookVerificationOutcome::Stale { .. }));
        assert!(matches!(stripe(&header, BODY, TIMESTAMP_S - 301), WebhookVerificationOutcome::Stale { .. }), "from the future too");
    }

    #[test]
    fn a_stale_request_with_a_bad_signature_learns_only_that_its_signature_is_bad() {
        let forged = format!("t={TIMESTAMP_S},v1={}", "0".repeat(64));
        assert_eq!(stripe(&forged, BODY, TIMESTAMP_S + 3_600), WebhookVerificationOutcome::BadSignature);
    }

    #[test]
    fn a_recorded_standard_webhook_verifies_and_every_tampered_field_is_refused() {
        let ts = TIMESTAMP_S.to_string();
        assert!(standard(STANDARD_MESSAGE_ID, &ts, STANDARD_SIGNATURE, BODY, STANDARD_KEY).is_verified());
        let bad = WebhookVerificationOutcome::BadSignature;
        assert_eq!(standard(STANDARD_MESSAGE_ID, &ts, STANDARD_SIGNATURE, &BODY.replacen("2000", "9000", 1), STANDARD_KEY), bad);
        assert_eq!(standard(STANDARD_MESSAGE_ID, "1732480001", STANDARD_SIGNATURE, BODY, STANDARD_KEY), bad);
        assert_eq!(standard("msg_2KWPBgLlAfxdpx2AI54pPJ85f4X", &ts, STANDARD_SIGNATURE, BODY, STANDARD_KEY), bad);
        assert_eq!(standard(STANDARD_MESSAGE_ID, &ts, STANDARD_SIGNATURE, BODY, "whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYo"), bad);
        let several = format!("v1,{} {STANDARD_SIGNATURE}", "A".repeat(43) + "=");
        assert!(standard(STANDARD_MESSAGE_ID, &ts, &several, BODY, STANDARD_KEY).is_verified(), "any v1 verifies");
    }

    #[test]
    fn a_standard_webhooks_secret_without_its_prefix_is_refused_without_quoting_it() {
        let ts = TIMESTAMP_S.to_string();
        let outcome = standard(STANDARD_MESSAGE_ID, &ts, STANDARD_SIGNATURE, BODY, "EBESExQVFhcYGRobHB0eHyAhIiMkJSYn");
        assert_eq!(outcome, WebhookVerificationOutcome::Malformed { reason: "the secret does not start with `whsec_`".into() });
        assert!(WebhookProvider::StandardWebhooks.signing_key(&secret(STANDARD_KEY)).is_ok());
    }

    #[test]
    fn a_window_that_does_not_fit_the_scheme_is_refused() {
        let header = format!("t={TIMESTAMP_S},v1={STRIPE_SIGNATURE}");
        let accepted = verify(WebhookProvider::Stripe, &[("Stripe-Signature", &header)], BODY.as_bytes(), STRIPE_KEY, 0, ReplayWindow::ReplaysAccepted);
        assert!(matches!(accepted, WebhookVerificationOutcome::Malformed { reason } if reason.contains("replay window is required")));
        let body = b"{}";
        let sig = format!("sha256={}", hmac_hex(b"shhh", body));
        let windowed = verify(WebhookProvider::GitHub, &[("X-Hub-Signature-256", &sig)], body, "shhh", 0, FIVE_MINUTES);
        assert!(matches!(windowed, WebhookVerificationOutcome::Malformed { reason } if reason.contains("signs no timestamp")));
    }

    #[test]
    fn github_verifies_correct_signature() {
        let body = b"{\"event\":\"push\"}";
        let sig = format!("sha256={}", hmac_hex(b"shhh", body));
        let outcome = verify(WebhookProvider::GitHub, &[("X-Hub-Signature-256", &sig)], body, "shhh", 0, ReplayWindow::ReplaysAccepted);
        assert!(outcome.is_verified());
    }

    #[test]
    fn github_rejects_tampered_body() {
        let sig = format!("sha256={}", hmac_hex(b"shhh", b"{\"event\":\"push\"}"));
        let outcome = verify(WebhookProvider::GitHub, &[("X-Hub-Signature-256", &sig)], b"{\"event\":\"PUSH\"}", "shhh", 0, ReplayWindow::ReplaysAccepted);
        assert_eq!(outcome, WebhookVerificationOutcome::BadSignature);
    }

    #[test]
    fn github_missing_header_or_prefix_is_malformed() {
        let missing = verify(WebhookProvider::GitHub, &[], b"body", "secret", 0, ReplayWindow::ReplaysAccepted);
        assert!(matches!(missing, WebhookVerificationOutcome::Malformed { .. }));
        let unprefixed = verify(WebhookProvider::GitHub, &[("X-Hub-Signature-256", "deadbeef")], b"body", "secret", 0, ReplayWindow::ReplaysAccepted);
        assert!(matches!(unprefixed, WebhookVerificationOutcome::Malformed { .. }));
    }

    #[test]
    fn slack_verifies_correct_signature_inside_window() {
        let body = b"token=xyz&team_id=T01";
        let now_s: u64 = 1_700_000_000;
        let sig = compute_slack_signature(b"slack-secret", now_s, body);
        let ts = now_s.to_string();
        let outcome = verify(WebhookProvider::Slack, &[("X-Slack-Signature", &sig), ("X-Slack-Request-Timestamp", &ts)], body, "slack-secret", now_s * 1_000, FIVE_MINUTES);
        assert!(outcome.is_verified());
    }

    #[test]
    fn slack_rejects_stale_timestamp_outside_window() {
        let then_s: u64 = 1_700_000_000;
        let sig = compute_slack_signature(b"slack-secret", then_s, b"old=event");
        let ts = then_s.to_string();
        let outcome = verify(WebhookProvider::Slack, &[("X-Slack-Signature", &sig), ("X-Slack-Request-Timestamp", &ts)], b"old=event", "slack-secret", (then_s + 600) * 1_000, FIVE_MINUTES);
        assert!(matches!(outcome, WebhookVerificationOutcome::Stale { .. }));
    }

    #[test]
    fn slack_rejects_tampered_body_and_a_missing_timestamp() {
        let now_s: u64 = 1_700_000_000;
        let sig = compute_slack_signature(b"slack-secret", now_s, b"original");
        let ts = now_s.to_string();
        let tampered = verify(WebhookProvider::Slack, &[("X-Slack-Signature", &sig), ("X-Slack-Request-Timestamp", &ts)], b"tampered", "slack-secret", now_s * 1_000, FIVE_MINUTES);
        assert_eq!(tampered, WebhookVerificationOutcome::BadSignature);
        let no_time = verify(WebhookProvider::Slack, &[("X-Slack-Signature", "v0=deadbeef")], b"body", "secret", 0, FIVE_MINUTES);
        assert!(matches!(no_time, WebhookVerificationOutcome::Malformed { .. }));
    }

    #[test]
    fn linear_verifies_correct_signature() {
        let body = b"{\"data\":{\"action\":\"create\"}}";
        let sig = hmac_hex(b"linear-secret", body);
        let outcome = verify(WebhookProvider::Linear, &[("Linear-Signature", &sig)], body, "linear-secret", 0, ReplayWindow::ReplaysAccepted);
        assert!(outcome.is_verified());
        let wrong = verify(WebhookProvider::Linear, &[("Linear-Signature", &sig)], body, "different-secret", 0, ReplayWindow::ReplaysAccepted);
        assert_eq!(wrong, WebhookVerificationOutcome::BadSignature);
    }

    /// Header lookup is case-insensitive per RFC 9110 — providers vary
    /// in case (`X-Hub-Signature-256` vs `x-hub-signature-256`).
    #[test]
    fn case_insensitive_header_lookup() {
        let body = b"{\"event\":\"push\"}";
        let sig = format!("sha256={}", hmac_hex(b"shhh", body));
        let outcome = verify(WebhookProvider::GitHub, &[("x-hub-signature-256", &sig)], body, "shhh", 0, ReplayWindow::ReplaysAccepted);
        assert!(outcome.is_verified());
    }

    #[test]
    fn provider_slug_round_trip() {
        for provider in WebhookProvider::ALL {
            assert_eq!(WebhookProvider::from_slug(provider.slug()), Some(provider));
        }
        assert!(WebhookProvider::from_slug("discord").is_none());
    }
}
