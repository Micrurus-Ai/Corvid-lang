//! The production [`ProviderGateway`] (slice 52e) — real HTTPS to the
//! provider's token and user endpoints via `reqwest`.
//!
//! Kept behind the same trait the resolver uses, so tests swap in a mock
//! and never touch the network. All requests are form/JSON over TLS; the
//! client secret travels only in the server-to-server token POST, never
//! to the browser.

use reqwest::blocking::Client;

use super::identity::{ProviderGateway, TokenExchange, TokenResponse};
use super::provider::OAuthProviderConfig;

/// A `ProviderGateway` backed by a blocking `reqwest` client. Handlers
/// call it inside `spawn_blocking`.
pub struct HttpProviderGateway {
    client: Client,
}

impl HttpProviderGateway {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .user_agent("corvid-serve")
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?;
        Ok(Self { client })
    }
}

impl ProviderGateway for HttpProviderGateway {
    fn exchange_code(
        &self,
        config: &OAuthProviderConfig,
        exchange: &TokenExchange<'_>,
    ) -> Result<TokenResponse, String> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", exchange.code),
            ("redirect_uri", exchange.redirect_uri),
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code_verifier", exchange.pkce_verifier),
        ];
        let response = self
            .client
            .post(&config.token_url)
            // Ask non-conforming providers (github) for JSON rather than
            // their default form-encoded body.
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&form)
            .send()
            .map_err(|e| format!("token exchange request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "token endpoint returned status {}",
                response.status().as_u16()
            ));
        }
        let body: serde_json::Value = response
            .json()
            .map_err(|e| format!("token endpoint returned a non-JSON body: {e}"))?;
        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let id_token = body
            .get("id_token")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Ok(TokenResponse {
            access_token,
            id_token,
        })
    }

    fn fetch_userinfo(
        &self,
        userinfo_url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, String> {
        let response = self
            .client
            .get(userinfo_url)
            .bearer_auth(access_token)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .map_err(|e| format!("userinfo request failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "userinfo endpoint returned status {}",
                response.status().as_u16()
            ));
        }
        response
            .json()
            .map_err(|e| format!("userinfo endpoint returned a non-JSON body: {e}"))
    }
}
