//! `corvid connectors verify-webhook` — standalone webhook
//! signature verifier for inbound payloads.
//!
//! With `--provider stripe|standard_webhooks|slack|github|linear`,
//! dispatches to the per-provider verifier in
//! `corvid-connector-runtime::webhook_verify`, the same one a
//! `verified` route in `corvid serve` uses. A provider that signs a
//! timestamp needs `--within-seconds`; the replay window is never a
//! default.
//!
//! Without a provider, falls back to a raw HMAC-SHA256 verifier
//! suitable for custom webhook integrations whose signing scheme
//! is "secret + body → hex digest" with optional `sha256=` prefix.

use anyhow::{anyhow, Context, Result};
use corvid_connector_runtime::{
    verify_webhook, ReplayWindow, WebhookProvider, WebhookVerificationOutcome, WebhookVerifyInputs,
};
use corvid_crypto::{hmac_sha256, Bytes, HmacKey, Secret};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct WebhookVerifyOutput {
    pub valid: bool,
    pub algorithm: String,
    pub outcome: String,
}

#[derive(Debug, Clone)]
pub struct WebhookVerifyArgs {
    pub signature: String,
    pub secret_env: String,
    pub body_file: PathBuf,
    pub provider: Option<String>,
    pub within_seconds: Option<u64>,
    pub headers: Vec<(String, String)>,
}

/// Verify an inbound webhook signature. With `--provider` set,
/// dispatches to the per-provider verifier; without, falls back to
/// raw HMAC-SHA256 verification.
pub fn run_verify_webhook(args: WebhookVerifyArgs) -> Result<WebhookVerifyOutput> {
    let body = fs::read(&args.body_file).with_context(|| {
        format!("reading webhook body from `{}`", args.body_file.display())
    })?;
    let secret = Secret::from_text(std::env::var(&args.secret_env).with_context(|| {
        format!(
            "webhook verification requires `{}` env var to hold the manifest's webhook secret",
            args.secret_env
        )
    })?);

    if let Some(provider_slug) = args.provider.as_deref() {
        let provider = WebhookProvider::from_slug(provider_slug).ok_or_else(|| {
            let known: Vec<&str> = WebhookProvider::ALL.iter().map(WebhookProvider::slug).collect();
            anyhow!("unknown webhook provider `{provider_slug}`; expected {}", known.join("|"))
        })?;
        let window = replay_window(provider, args.within_seconds)?;
        let headers: std::collections::BTreeMap<String, String> = args.headers.iter().cloned().collect();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let outcome = verify_webhook(WebhookVerifyInputs::new(provider, &headers, &body, &secret, now_ms, window));
        return Ok(WebhookVerifyOutput {
            valid: outcome.is_verified(),
            algorithm: "hmac-sha256".to_string(),
            outcome: outcome_label(&outcome),
        });
    }

    // Provider-agnostic fallback: raw HMAC-SHA256 with the legacy
    // `sha256=` prefix tolerance, compared as bytes in constant time.
    let computed = hmac_sha256(&Bytes::new(&body), HmacKey::Secret(&secret));
    let expected_hex = args
        .signature
        .trim()
        .strip_prefix("sha256=")
        .unwrap_or(args.signature.trim());
    let valid = Bytes::from_hex(expected_hex).is_ok_and(|expected| expected.constant_time_eq(&computed));
    Ok(WebhookVerifyOutput {
        valid,
        algorithm: "hmac-sha256".to_string(),
        outcome: if valid {
            "verified".to_string()
        } else {
            "bad_signature".to_string()
        },
    })
}

/// The window the operator stated: required for a provider that signs
/// a timestamp, refused for one that signs none.
fn replay_window(provider: WebhookProvider, within_seconds: Option<u64>) -> Result<ReplayWindow> {
    let slug = provider.slug();
    match (provider.signs_time(), within_seconds) {
        (true, Some(seconds)) => Ok(ReplayWindow::Within { max_age_ms: seconds.saturating_mul(1_000) }),
        (true, None) => Err(anyhow!(
            "`{slug}` signs a timestamp; pass `--within-seconds` to say how old a request may be"
        )),
        (false, None) => Ok(ReplayWindow::ReplaysAccepted),
        (false, Some(_)) => Err(anyhow!("`{slug}` signs no timestamp, so `--within-seconds` cannot be checked")),
    }
}

fn outcome_label(outcome: &WebhookVerificationOutcome) -> String {
    match outcome {
        WebhookVerificationOutcome::Verified => "verified".to_string(),
        WebhookVerificationOutcome::BadSignature => "bad_signature".to_string(),
        WebhookVerificationOutcome::Stale { delta_ms, .. } => {
            format!("stale (delta_ms={delta_ms})")
        }
        WebhookVerificationOutcome::Malformed { reason } => {
            format!("malformed ({reason})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hmac_hex(secret: &str, body: &[u8]) -> String {
        hmac_sha256(&Bytes::new(body), HmacKey::Secret(&Secret::new(secret))).to_hex()
    }

    fn args(env: &str, body_path: PathBuf, provider: Option<&str>, within_seconds: Option<u64>) -> WebhookVerifyArgs {
        WebhookVerifyArgs {
            signature: String::new(),
            secret_env: env.to_string(),
            body_file: body_path,
            provider: provider.map(str::to_string),
            within_seconds,
            headers: Vec::new(),
        }
    }

    /// Slice 41L: `verify-webhook` validates a body against an
    /// HMAC-SHA256 signature computed with the given secret.
    #[test]
    fn webhook_verify_accepts_correct_signature() {
        let temp = tempfile::tempdir().unwrap();
        let body_path = temp.path().join("payload.json");
        let body = b"{\"event\":\"push\"}";
        std::fs::write(&body_path, body).unwrap();
        std::env::set_var("CORVID_TEST_WEBHOOK_SECRET", "shhhh");
        let output = run_verify_webhook(WebhookVerifyArgs {
            signature: format!("sha256={}", hmac_hex("shhhh", body)),
            ..args("CORVID_TEST_WEBHOOK_SECRET", body_path, None, None)
        })
        .expect("verify");
        std::env::remove_var("CORVID_TEST_WEBHOOK_SECRET");
        assert!(output.valid);
        assert_eq!(output.algorithm, "hmac-sha256");
        assert_eq!(output.outcome, "verified");
    }

    /// Slice 41L adversarial: a tampered body fails verification.
    #[test]
    fn webhook_verify_rejects_tampered_body() {
        let temp = tempfile::tempdir().unwrap();
        let body_path = temp.path().join("payload.json");
        std::fs::write(&body_path, b"{\"event\":\"tampered\"}").unwrap();
        std::env::set_var("CORVID_TEST_WEBHOOK_SECRET_2", "shhhh");
        let output = run_verify_webhook(WebhookVerifyArgs {
            signature: hmac_hex("shhhh", b"{\"event\":\"original\"}"),
            ..args("CORVID_TEST_WEBHOOK_SECRET_2", body_path, None, None)
        })
        .expect("verify runs");
        std::env::remove_var("CORVID_TEST_WEBHOOK_SECRET_2");
        assert!(!output.valid);
        assert_eq!(output.outcome, "bad_signature");
    }

    /// Slice 41M-A: provider-aware GitHub path uses the
    /// `X-Hub-Signature-256` header from `--header` rather than the
    /// generic `--signature` value.
    #[test]
    fn webhook_verify_dispatches_to_github_provider() {
        let temp = tempfile::tempdir().unwrap();
        let body_path = temp.path().join("payload.json");
        let body = b"{\"event\":\"push\"}";
        std::fs::write(&body_path, body).unwrap();
        std::env::set_var("CORVID_TEST_GITHUB_WEBHOOK", "github-secret");
        let output = run_verify_webhook(WebhookVerifyArgs {
            headers: vec![("X-Hub-Signature-256".to_string(), format!("sha256={}", hmac_hex("github-secret", body)))],
            ..args("CORVID_TEST_GITHUB_WEBHOOK", body_path, Some("github"), None)
        })
        .expect("verify");
        std::env::remove_var("CORVID_TEST_GITHUB_WEBHOOK");
        assert!(output.valid);
        assert_eq!(output.outcome, "verified");
    }

    /// The replay window is the operator's to state: required for a
    /// provider that signs a timestamp, refused for one that signs none.
    #[test]
    fn webhook_verify_requires_a_window_exactly_where_the_provider_signs_a_time() {
        let temp = tempfile::tempdir().unwrap();
        let body_path = temp.path().join("payload.json");
        std::fs::write(&body_path, b"x").unwrap();
        std::env::set_var("CORVID_TEST_WINDOW_SECRET", "secret");
        let missing = run_verify_webhook(args("CORVID_TEST_WINDOW_SECRET", body_path.clone(), Some("stripe"), None)).unwrap_err();
        assert!(missing.to_string().contains("--within-seconds"), "{missing}");
        let extra = run_verify_webhook(args("CORVID_TEST_WINDOW_SECRET", body_path, Some("github"), Some(300))).unwrap_err();
        assert!(extra.to_string().contains("signs no timestamp"), "{extra}");
        std::env::remove_var("CORVID_TEST_WINDOW_SECRET");
    }

    /// Slice 41M-A: an unknown provider yields a clear diagnostic.
    #[test]
    fn webhook_verify_unknown_provider_refused() {
        let temp = tempfile::tempdir().unwrap();
        let body_path = temp.path().join("payload.json");
        std::fs::write(&body_path, b"x").unwrap();
        std::env::set_var("CORVID_TEST_UNKNOWN_PROVIDER", "secret");
        let err = run_verify_webhook(args("CORVID_TEST_UNKNOWN_PROVIDER", body_path, Some("discord"), None)).unwrap_err();
        std::env::remove_var("CORVID_TEST_UNKNOWN_PROVIDER");
        assert!(err.to_string().contains("unknown webhook provider"));
        assert!(err.to_string().contains("standard_webhooks"), "names every provider: {err}");
    }
}
