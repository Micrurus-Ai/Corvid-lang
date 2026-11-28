//! Verified webhook routes in `corvid serve` (verified webhooks, B16).
//!
//! A route declared `verified <scheme>(secret: "<NAME>", within: ...)`
//! is a provider's signed call. Its secret is read once at startup, and
//! a missing or unusable one refuses to start, as a connector credential
//! does. Each request's signature is checked over the exact bytes that
//! arrived, by `corvid-connector-runtime`'s verifier, before the body is
//! decoded, any policy runs or the handler sees anything; a request that
//! fails is answered 401 and never reaches the program.
//!
//! The trace records each delivery as `webhook.verified` or
//! `webhook.refused`, with the route, the scheme and the provider's
//! delivery id where its headers carry one. Neither the secret nor the
//! signature is ever recorded.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use corvid_connector_runtime::{
    verify_webhook, ReplayWindow, WebhookProvider, WebhookVerificationOutcome, WebhookVerifyInputs,
};
use corvid_crypto::Secret;
use corvid_runtime::Runtime;
use std::collections::BTreeMap;

/// A verified route's scheme, secret and window, resolved at startup.
pub(super) struct RouteWebhook {
    provider: WebhookProvider,
    secret: Secret,
    window: ReplayWindow,
}

impl RouteWebhook {
    /// The route's verification, or `None` for an ordinary route. Fails
    /// when the secret is not set or cannot key the scheme, so the
    /// server refuses to start instead of refusing every delivery.
    pub(super) fn load(route: &corvid_ir::IrRoute, runtime: &Runtime) -> anyhow::Result<Option<Self>> {
        let Some(clause) = &route.verified else { return Ok(None) };
        let place = format!("route `{} {}`", route.method.as_str(), route.path);
        // The checker accepts only the schemes the verifier implements;
        // `the_checker_and_the_verifier_know_the_same_schemes` holds the
        // two lists equal.
        let provider = WebhookProvider::from_slug(&clause.scheme)
            .ok_or_else(|| anyhow::anyhow!("{place} names `{}`, a scheme this server cannot verify", clause.scheme))?;
        let read = runtime.read_env_secret(&clause.secret)?;
        let Some(text) = read.value else {
            anyhow::bail!(
                "{place} is verified with `{}`, but the secret `{}` is not set; set it to the signing secret the provider gave you",
                clause.scheme,
                clause.secret
            );
        };
        let secret = Secret::from_text(text);
        provider
            .signing_key(&secret)
            .map_err(|problem| anyhow::anyhow!("{place}: the secret `{}` cannot sign `{}` deliveries: {problem}", clause.secret, clause.scheme))?;
        let window = match clause.window_secs {
            Some(seconds) => ReplayWindow::Within { max_age_ms: seconds.saturating_mul(1_000) },
            None => ReplayWindow::ReplaysAccepted,
        };
        Ok(Some(RouteWebhook { provider, secret, window }))
    }

    /// Check one delivery against the bytes that arrived. On success the
    /// handler may run; otherwise the 401 to answer with.
    pub(super) fn verify(
        &self,
        runtime: &Runtime,
        route: &str,
        headers: &HeaderMap,
        body: &[u8],
        now_ms: u64,
    ) -> Result<(), Response> {
        let headers = header_map(headers);
        let delivery = self
            .provider
            .delivery_id_header()
            .and_then(|name| headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)))
            .map(|(_, value)| value.clone());
        let outcome = verify_webhook(WebhookVerifyInputs::new(self.provider, &headers, body, &self.secret, now_ms, self.window));
        let scheme = self.provider.slug();
        let (error, detail) = match &outcome {
            WebhookVerificationOutcome::Verified => {
                runtime.emit_host_event(
                    "webhook.verified",
                    serde_json::json!({ "route": route, "scheme": scheme, "delivery_id": delivery }),
                );
                return Ok(());
            }
            WebhookVerificationOutcome::BadSignature => ("webhook_signature_invalid", "the signature does not match".to_string()),
            WebhookVerificationOutcome::Malformed { reason } => ("webhook_signature_invalid", reason.clone()),
            WebhookVerificationOutcome::Stale { .. } => {
                ("webhook_expired", "the signed time is outside the route's window".to_string())
            }
        };
        runtime.emit_host_event(
            "webhook.refused",
            serde_json::json!({ "route": route, "scheme": scheme, "delivery_id": delivery, "error": error }),
        );
        Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": error, "detail": detail }))).into_response())
    }
}

/// The request's headers as the verifier reads them. A header sent more
/// than once keeps its first value: a signature must still verify, so an
/// extra copy gains a sender nothing.
fn header_map(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in headers {
        if let Ok(value) = value.to_str() {
            map.entry(name.as_str().to_string()).or_insert_with(|| value.to_string());
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use corvid_connector_runtime::WebhookProvider;

    /// The checker's schemes (`corvid_types::webhook`) and the
    /// verifier's (`corvid-connector-runtime`) are two lists because
    /// neither crate can depend on the other. They must name the same
    /// schemes, agree on which sign a time, and document the same
    /// headers, or a route could compile that the server cannot verify.
    #[test]
    fn the_checker_and_the_verifier_know_the_same_schemes() {
        let checker: Vec<_> = corvid_types::webhook::SCHEMES
            .iter()
            .map(|scheme| (scheme.name, scheme.signs_time, scheme.signature_headers.to_vec()))
            .collect();
        let verifier: Vec<_> = WebhookProvider::ALL
            .iter()
            .map(|provider| (provider.slug(), provider.signs_time(), provider.signature_headers().to_vec()))
            .collect();
        assert_eq!(checker, verifier);
    }
}
