//! Identity resolution (slice 52e) — turn a successful code exchange
//! into a SERVER-VERIFIED external identity.
//!
//! This is the callback's steps 2–4: exchange the authorization code
//! for tokens, then establish the identity by the provider's method:
//!
//! - **OIDC** — verify the ID token against the issuer's JWKS
//!   (signature, issuer, audience, expiry) and the login `nonce`, then
//!   take `(issuer, subject)` from the verified claims.
//! - **Userinfo** — fetch the provider's user endpoint over TLS with the
//!   access token and take `(source_marker, authoritative_user_id)` from
//!   the response.
//!
//! Every network operation goes through the [`ProviderGateway`] trait so
//! the flow is testable without a live provider (a mock IdP mints ID
//! tokens; a mock gateway returns canned userinfo). The JWKS fetch uses
//! the runtime's existing pluggable [`JwksFetcher`].

use std::sync::Arc;

use corvid_runtime::jwt_verify::{JwksFetcher, JwtVerifier};
use corvid_runtime::JwtVerificationContract;
use sha2::{Digest, Sha256};

use super::provider::{IdentityVerification, OAuthProviderConfig};

/// The inputs a token exchange needs (the authorization-code grant).
#[derive(Debug, Clone)]
pub struct TokenExchange<'a> {
    pub code: &'a str,
    pub redirect_uri: &'a str,
    pub pkce_verifier: &'a str,
}

/// The provider's token-endpoint response, reduced to what the flow
/// uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub id_token: Option<String>,
}

/// The network operations a login flow performs against a provider.
/// Abstracted so the resolution logic is exercised without live HTTP.
pub trait ProviderGateway {
    /// Exchange an authorization code for tokens at the provider's token
    /// endpoint (authorization-code + PKCE).
    fn exchange_code(
        &self,
        config: &OAuthProviderConfig,
        exchange: &TokenExchange<'_>,
    ) -> Result<TokenResponse, String>;

    /// Fetch the provider's user endpoint with a bearer access token.
    fn fetch_userinfo(
        &self,
        userinfo_url: &str,
        access_token: &str,
    ) -> Result<serde_json::Value, String>;
}

/// A server-verified external identity — the outcome of resolution.
/// `source` + `external_id` are the durable identity key; the rest are
/// hints used only to provision (email gates invitations, tenant_claim
/// feeds a claim-mapped tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExternalIdentity {
    pub source: String,
    pub external_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub tenant_claim: Option<String>,
}

/// Everything the resolver needs for one callback.
pub struct IdentityResolution<'a> {
    pub config: &'a OAuthProviderConfig,
    pub exchange: TokenExchange<'a>,
    /// The `nonce` the login minted (raw). Compared against the ID
    /// token's `nonce` claim for OIDC; ignored for userinfo.
    pub expected_nonce: Option<&'a str>,
    /// The claim to read the tenant from, when the provisioning policy
    /// maps the tenant from an issuer claim.
    pub tenant_claim_name: Option<&'a str>,
    pub now_ms: u64,
}

/// A stable fingerprint of a nonce, so a stored nonce can be compared to
/// an ID token's nonce claim without keeping the raw value around.
pub fn nonce_fingerprint(nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"corvid-auth-oidc-nonce-v1:");
    hasher.update(nonce.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Resolve the verified external identity for a callback. `gateway`
/// performs the network I/O; `jwks_fetcher` supplies the issuer keys for
/// the OIDC path.
pub fn resolve_identity(
    res: IdentityResolution<'_>,
    gateway: &dyn ProviderGateway,
    jwks_fetcher: Arc<dyn JwksFetcher>,
) -> Result<VerifiedExternalIdentity, String> {
    let tokens = gateway.exchange_code(res.config, &res.exchange)?;
    match &res.config.verification {
        IdentityVerification::Oidc {
            issuer,
            jwks_url,
            algorithm,
        } => resolve_oidc(&res, &tokens, issuer, jwks_url, algorithm, jwks_fetcher),
        IdentityVerification::Userinfo {
            source_marker,
            userinfo_url,
        } => resolve_userinfo(&tokens, source_marker, userinfo_url, gateway),
    }
}

fn resolve_oidc(
    res: &IdentityResolution<'_>,
    tokens: &TokenResponse,
    issuer: &str,
    jwks_url: &str,
    algorithm: &str,
    jwks_fetcher: Arc<dyn JwksFetcher>,
) -> Result<VerifiedExternalIdentity, String> {
    let id_token = tokens
        .id_token
        .as_deref()
        .ok_or("oidc provider returned no id_token")?;
    let contract = JwtVerificationContract {
        issuer: issuer.to_string(),
        audience: res.config.client_id.clone(),
        jwks_url: jwks_url.to_string(),
        algorithm: algorithm.to_string(),
        required_subject_claim: "sub".to_string(),
        // The tenant is required in the token only when the provisioning
        // policy maps it from a claim; otherwise it is absent by design.
        required_tenant_claim: res.tenant_claim_name.unwrap_or("").to_string(),
        clock_skew_ms: 60_000,
    };
    let verifier = JwtVerifier::new(jwks_fetcher);
    let claims = verifier
        .verify(id_token, &contract, res.now_ms)
        .map_err(|e| format!("id token verification failed: {}", e.slug()))?;

    // Bind the ID token to THIS login: its nonce claim must match the
    // nonce the login minted. A missing or mismatched nonce is a replay
    // or a token issued to a different flow.
    if let Some(expected) = res.expected_nonce {
        let claim = claims
            .raw
            .get("nonce")
            .and_then(|v| v.as_str())
            .ok_or("id token is missing the nonce claim")?;
        if nonce_fingerprint(claim) != nonce_fingerprint(expected) {
            return Err("id token nonce does not match the login nonce".to_string());
        }
    }

    let email = string_claim(&claims.raw, "email");
    let display_name = string_claim(&claims.raw, "name");
    let tenant_claim = res
        .tenant_claim_name
        .and_then(|name| string_claim(&claims.raw, name));

    Ok(VerifiedExternalIdentity {
        source: claims.issuer,
        external_id: claims.subject,
        email,
        display_name,
        tenant_claim,
    })
}

fn resolve_userinfo(
    tokens: &TokenResponse,
    source_marker: &str,
    userinfo_url: &str,
    gateway: &dyn ProviderGateway,
) -> Result<VerifiedExternalIdentity, String> {
    if tokens.access_token.trim().is_empty() {
        return Err("oauth2 provider returned no access_token".to_string());
    }
    let userinfo = gateway.fetch_userinfo(userinfo_url, &tokens.access_token)?;
    let external_id = external_id_from_userinfo(&userinfo)
        .ok_or("userinfo response carried no stable user id")?;
    let email = string_claim(&userinfo, "email");
    let display_name = string_claim(&userinfo, "name")
        .or_else(|| string_claim(&userinfo, "login"))
        .or_else(|| string_claim(&userinfo, "username"))
        .or_else(|| string_claim(&userinfo, "global_name"));
    Ok(VerifiedExternalIdentity {
        source: source_marker.to_string(),
        external_id,
        email,
        display_name,
        tenant_claim: None,
    })
}

/// Pull a stable user id out of a userinfo document, tolerant of the
/// per-provider key (`sub` for OIDC-style userinfo, `id` for
/// github/discord) and of numeric vs string encodings.
fn external_id_from_userinfo(userinfo: &serde_json::Value) -> Option<String> {
    for key in ["sub", "id", "user_id"] {
        match userinfo.get(key) {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                return Some(s.clone())
            }
            Some(serde_json::Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn string_claim(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::jwt_verify::mock_idp::MockIdp;
    use std::cell::RefCell;

    /// A gateway that returns pre-canned tokens + userinfo and records
    /// what it was asked to exchange.
    struct MockGateway {
        token: TokenResponse,
        userinfo: serde_json::Value,
        last_code: RefCell<Option<String>>,
    }

    impl MockGateway {
        fn oidc(id_token: String) -> Self {
            Self {
                token: TokenResponse {
                    access_token: "at".to_string(),
                    id_token: Some(id_token),
                },
                userinfo: serde_json::Value::Null,
                last_code: RefCell::new(None),
            }
        }
        fn userinfo(userinfo: serde_json::Value) -> Self {
            Self {
                token: TokenResponse {
                    access_token: "at".to_string(),
                    id_token: None,
                },
                userinfo,
                last_code: RefCell::new(None),
            }
        }
    }

    impl ProviderGateway for MockGateway {
        fn exchange_code(
            &self,
            _config: &OAuthProviderConfig,
            exchange: &TokenExchange<'_>,
        ) -> Result<TokenResponse, String> {
            *self.last_code.borrow_mut() = Some(exchange.code.to_string());
            Ok(self.token.clone())
        }
        fn fetch_userinfo(
            &self,
            _userinfo_url: &str,
            _access_token: &str,
        ) -> Result<serde_json::Value, String> {
            Ok(self.userinfo.clone())
        }
    }

    fn oidc_config(idp: &MockIdp) -> OAuthProviderConfig {
        OAuthProviderConfig {
            provider_name: "mock".to_string(),
            authorize_url: "https://issuer.test/authorize".to_string(),
            token_url: "https://issuer.test/token".to_string(),
            scopes: "openid email".to_string(),
            verification: IdentityVerification::Oidc {
                issuer: "https://issuer.test".to_string(),
                jwks_url: "https://issuer.test/jwks".to_string(),
                algorithm: "EdDSA".to_string(),
            },
            client_id: "corvid-test".to_string(),
            client_secret: "secret".to_string(),
        }
    }

    fn userinfo_config() -> OAuthProviderConfig {
        OAuthProviderConfig {
            provider_name: "github".to_string(),
            authorize_url: "https://github.com/login/oauth/authorize".to_string(),
            token_url: "https://github.com/login/oauth/access_token".to_string(),
            scopes: "read:user".to_string(),
            verification: IdentityVerification::Userinfo {
                source_marker: "github".to_string(),
                userinfo_url: "https://api.github.com/user".to_string(),
            },
            client_id: "id".to_string(),
            client_secret: "secret".to_string(),
        }
    }

    #[test]
    fn oidc_resolves_issuer_subject_from_a_verified_id_token() {
        let idp = MockIdp::new("https://issuer.test", "corvid-test");
        let nonce = "nonce-abc";
        let token = idp.mint_with(|p| {
            p["iss"] = "https://issuer.test".into();
            p["aud"] = "corvid-test".into();
            p["sub"] = "sub-1".into();
            p["email"] = "ada@example.com".into();
            p["name"] = "Ada".into();
            p["nonce"] = nonce.into();
            p.as_object_mut().unwrap().remove("tenant");
        });
        let config = oidc_config(&idp);
        let gateway = MockGateway::oidc(token);
        let identity = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "code-1",
                    redirect_uri: "https://app/callback",
                    pkce_verifier: "verifier",
                },
                expected_nonce: Some(nonce),
                tenant_claim_name: None,
                now_ms: 2_000_000,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap();
        assert_eq!(identity.source, "https://issuer.test");
        assert_eq!(identity.external_id, "sub-1");
        assert_eq!(identity.email.as_deref(), Some("ada@example.com"));
        assert_eq!(identity.display_name.as_deref(), Some("Ada"));
        assert_eq!(gateway.last_code.borrow().as_deref(), Some("code-1"));
    }

    #[test]
    fn oidc_rejects_a_token_whose_nonce_does_not_match() {
        let idp = MockIdp::new("https://issuer.test", "corvid-test");
        let token = idp.mint_with(|p| {
            p["iss"] = "https://issuer.test".into();
            p["aud"] = "corvid-test".into();
            p["sub"] = "sub-1".into();
            p["nonce"] = "attacker-nonce".into();
            p.as_object_mut().unwrap().remove("tenant");
        });
        let config = oidc_config(&idp);
        let gateway = MockGateway::oidc(token);
        let err = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "c",
                    redirect_uri: "r",
                    pkce_verifier: "v",
                },
                expected_nonce: Some("the-real-login-nonce"),
                tenant_claim_name: None,
                now_ms: 2_000_000,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap_err();
        assert!(err.contains("nonce"), "got: {err}");
    }

    #[test]
    fn oidc_rejects_a_tampered_token() {
        let idp = MockIdp::new("https://issuer.test", "corvid-test");
        let config = oidc_config(&idp);
        let gateway = MockGateway::oidc(idp.mint_tampered_signature());
        let err = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "c",
                    redirect_uri: "r",
                    pkce_verifier: "v",
                },
                expected_nonce: None,
                tenant_claim_name: None,
                now_ms: 2_000_000,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap_err();
        assert!(err.contains("verification failed"), "got: {err}");
    }

    #[test]
    fn oidc_reads_a_mapped_tenant_claim() {
        let idp = MockIdp::new("https://issuer.test", "corvid-test");
        let token = idp.mint_with(|p| {
            p["iss"] = "https://issuer.test".into();
            p["aud"] = "corvid-test".into();
            p["sub"] = "sub-1".into();
            p["org_id"] = "acme".into();
        });
        let config = oidc_config(&idp);
        let gateway = MockGateway::oidc(token);
        let identity = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "c",
                    redirect_uri: "r",
                    pkce_verifier: "v",
                },
                expected_nonce: None,
                tenant_claim_name: Some("org_id"),
                now_ms: 2_000_000,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap();
        assert_eq!(identity.tenant_claim.as_deref(), Some("acme"));
    }

    #[test]
    fn userinfo_resolves_a_numeric_github_id() {
        let config = userinfo_config();
        let gateway = MockGateway::userinfo(serde_json::json!({
            "id": 4210,
            "login": "ada",
            "email": "ada@example.com",
        }));
        // The JWKS fetcher is unused on the userinfo path.
        let idp = MockIdp::new("unused", "unused");
        let identity = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "c",
                    redirect_uri: "r",
                    pkce_verifier: "v",
                },
                expected_nonce: None,
                tenant_claim_name: None,
                now_ms: 0,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap();
        assert_eq!(identity.source, "github");
        assert_eq!(identity.external_id, "4210");
        assert_eq!(identity.display_name.as_deref(), Some("ada"));
        assert_eq!(identity.email.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn userinfo_without_a_stable_id_is_refused() {
        let config = userinfo_config();
        let gateway = MockGateway::userinfo(serde_json::json!({ "email": "ada@example.com" }));
        let idp = MockIdp::new("unused", "unused");
        let err = resolve_identity(
            IdentityResolution {
                config: &config,
                exchange: TokenExchange {
                    code: "c",
                    redirect_uri: "r",
                    pkce_verifier: "v",
                },
                expected_nonce: None,
                tenant_claim_name: None,
                now_ms: 0,
            },
            &gateway,
            Arc::new(idp.fetcher()),
        )
        .unwrap_err();
        assert!(err.contains("no stable user id"), "got: {err}");
    }
}
