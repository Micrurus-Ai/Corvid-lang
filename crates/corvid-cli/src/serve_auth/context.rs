//! The serve-time auth configuration (slice 52e): everything the login
//! routes need, resolved once at startup from the `identity` block.
//!
//! Translating the declared policy here — providers, first-login policy,
//! tenant source, cookie posture — means the handlers stay thin and the
//! consequential choices (which provider, open vs invited, where the
//! tenant comes from) are resolved and validated before the listener
//! binds. A provider whose credentials or discovery cannot be resolved
//! is a startup error, never a route that half-works.

use std::collections::HashMap;
use std::sync::Arc;

use corvid_ast::{
    FirstLoginPolicy as AstFirstLogin, IdentityDecl, SameSite, TenantAssignment,
};
use corvid_runtime::jwt_verify::{JwksFetcher, ReqwestJwksFetcher};
use corvid_runtime::{FirstLoginPolicy, SessionAuthRuntime, TenantSource};

use super::identity::ProviderGateway;
use super::net::HttpProviderGateway;
use super::provider::{
    resolve_provider, IdentityVerification, OAuthProviderConfig, ResolvedProvider,
};

/// The tenant recorded on a first-login OAuth state row. A login has no
/// real tenant until provisioning assigns one, so its state uses this
/// fixed sentinel; the callback presents the same sentinel, so the
/// single-use / expiry checks still apply.
pub const LOGIN_STATE_TENANT: &str = "__corvid_login__";

/// The session cookie posture, resolved from the identity block's
/// session config (or the safe defaults).
#[derive(Debug, Clone)]
pub struct CookieSettings {
    pub name: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: &'static str,
}

impl Default for CookieSettings {
    fn default() -> Self {
        Self {
            name: "corvid_session".to_string(),
            secure: true,
            http_only: true,
            same_site: "Strict",
        }
    }
}

/// Everything the auth routes read at request time.
pub struct AuthContext {
    pub auth: Arc<SessionAuthRuntime>,
    pub providers: HashMap<String, OAuthProviderConfig>,
    pub first_login: FirstLoginPolicy,
    pub tenant: TenantSource,
    pub tenant_claim_name: Option<String>,
    /// The role an `open` signup receives (`provisioning: default_role`),
    /// or `None` for least privilege (slice 52f).
    pub default_role: Option<String>,
    /// The identity block's role → permission mapping (slice 52f), used
    /// to resolve an actor's effective permissions when enforcing a
    /// route's `requires permission(...)`.
    pub role_permissions: HashMap<String, Vec<String>>,
    /// Per-serve secret for CSRF double-submit HMAC tokens (slice 52f).
    pub csrf_secret: Vec<u8>,
    pub cookie: CookieSettings,
    pub session_lifetime_secs: u64,
    pub gateway: Arc<dyn ProviderGateway + Send + Sync>,
    pub jwks_fetcher: Arc<dyn JwksFetcher>,
    /// The externally reachable base URL (`https://app.example.com`) used
    /// to build the provider `redirect_uri`. `None` falls back to the
    /// request's `Host` header.
    pub public_base_url: Option<String>,
    /// Where the callback sends the browser after a successful login.
    pub post_login_redirect: String,
}

const DEFAULT_SESSION_LIFETIME_SECS: u64 = 8 * 3600;

impl AuthContext {
    /// Build the auth context from a declared identity block, reading
    /// client credentials + configuration from the process environment.
    /// Uses the production HTTP gateway + JWKS fetcher. When `data_dir`
    /// is `Some`, the session + external-identity store persists there
    /// (`auth.sqlite`) and survives a restart; otherwise it is in-memory
    /// (slice 52f).
    pub fn from_identity(
        identity: &IdentityDecl,
        data_dir: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let gateway: Arc<dyn ProviderGateway + Send + Sync> =
            Arc::new(HttpProviderGateway::new()?);
        let jwks_fetcher: Arc<dyn JwksFetcher> = Arc::new(ReqwestJwksFetcher::default());
        let auth = Arc::new(match data_dir {
            Some(dir) => SessionAuthRuntime::open(dir.join("auth.sqlite"))
                .map_err(|e| format!("failed to open durable auth store: {e}"))?,
            None => SessionAuthRuntime::open_in_memory()
                .map_err(|e| format!("failed to open auth store: {e}"))?,
        });
        Self::build(identity, auth, gateway, jwks_fetcher, &|key| {
            std::env::var(key).ok()
        })
    }

    /// The testable core: policy translation + provider resolution with
    /// injected store, gateway, JWKS fetcher, and environment.
    pub fn build(
        identity: &IdentityDecl,
        auth: Arc<SessionAuthRuntime>,
        gateway: Arc<dyn ProviderGateway + Send + Sync>,
        jwks_fetcher: Arc<dyn JwksFetcher>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let provisioning = identity.provisioning.as_ref().ok_or_else(|| {
            // The checker (E5210) guarantees this is present; a serve that
            // reaches here without it refuses rather than guess a policy.
            "identity block has no provisioning policy (should have been a compile error)"
                .to_string()
        })?;
        let first_login = match provisioning.first_login {
            AstFirstLogin::Open => FirstLoginPolicy::Open,
            AstFirstLogin::Invited => FirstLoginPolicy::Invited,
            AstFirstLogin::ApprovalRequired => {
                return Err(
                    "`first_login: approval_required` is not executable yet (should have been a compile error)"
                        .to_string(),
                )
            }
        };
        let (tenant, tenant_claim_name) = match &provisioning.tenant {
            TenantAssignment::Fixed(id) => (TenantSource::Fixed(id.clone()), None),
            TenantAssignment::FromInvitation => (TenantSource::FromInvitation, None),
            TenantAssignment::ClaimMapping { claim, allowlist } => (
                TenantSource::Claim {
                    claim: claim.clone(),
                    allowlist: allowlist.clone(),
                },
                Some(claim.clone()),
            ),
        };

        let mut providers = HashMap::new();
        for provider in &identity.providers {
            match resolve_provider(provider, env)? {
                ResolvedProvider::Ready(config) => {
                    providers.insert(config.provider_name.clone(), *config);
                }
                ResolvedProvider::NeedsDiscovery(needs) => {
                    let config = discover_oidc_provider(&needs)?;
                    providers.insert(config.provider_name.clone(), config);
                }
            }
        }

        let cookie = cookie_settings(identity);
        let session_lifetime_secs = identity
            .session
            .as_ref()
            .and_then(|s| s.lifetime_secs)
            .unwrap_or(DEFAULT_SESSION_LIFETIME_SECS);

        let role_permissions = identity
            .roles
            .iter()
            .map(|r| (r.name.clone(), r.permissions.clone()))
            .collect();

        Ok(Self {
            auth,
            providers,
            first_login,
            tenant,
            tenant_claim_name,
            default_role: provisioning.default_role.clone(),
            role_permissions,
            csrf_secret: random_csrf_secret(),
            cookie,
            session_lifetime_secs,
            gateway,
            jwks_fetcher,
            public_base_url: env("CORVID_PUBLIC_URL").filter(|v| !v.trim().is_empty()),
            post_login_redirect: env("CORVID_POST_LOGIN_REDIRECT")
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "/".to_string()),
        })
    }
}

/// A fresh 32-byte CSRF HMAC secret for this serve process.
fn random_csrf_secret() -> Vec<u8> {
    use rand_core::{OsRng, RngCore};
    let mut secret = vec![0u8; 32];
    OsRng.fill_bytes(&mut secret);
    secret
}

fn cookie_settings(identity: &IdentityDecl) -> CookieSettings {
    let mut settings = CookieSettings::default();
    if let Some(session) = &identity.session {
        settings.secure = session.cookie.secure;
        settings.http_only = session.cookie.http_only;
        settings.same_site = match session.cookie.same_site {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        };
    }
    settings
}

/// Resolve a generic `oidc` provider by fetching its discovery document
/// and pinning the endpoints + signing algorithm. Runs at startup, so a
/// discovery failure refuses to start rather than surfacing mid-login.
fn discover_oidc_provider(
    needs: &super::provider::DiscoveryNeeded,
) -> Result<OAuthProviderConfig, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("corvid-serve")
        .build()
        .map_err(|e| format!("failed to build discovery client: {e}"))?;
    let doc: serde_json::Value = client
        .get(&needs.discovery_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(|e| {
            format!(
                "OIDC discovery failed for `{}` ({}): {e}",
                needs.provider_name, needs.discovery_url
            )
        })?;
    let field = |key: &str| -> Result<String, String> {
        doc.get(key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                format!(
                    "OIDC discovery document for `{}` is missing `{key}`",
                    needs.provider_name
                )
            })
    };
    let issuer = field("issuer")?;
    let authorize_url = field("authorization_endpoint")?;
    let token_url = field("token_endpoint")?;
    let jwks_url = field("jwks_uri")?;
    let algorithm = doc
        .get("id_token_signing_alg_values_supported")
        .and_then(|v| v.as_array())
        .and_then(|algs| {
            // Prefer RS256, else the first advertised alg we support.
            let names: Vec<&str> = algs.iter().filter_map(|a| a.as_str()).collect();
            for preferred in ["RS256", "ES256", "EdDSA"] {
                if names.contains(&preferred) {
                    return Some(preferred.to_string());
                }
            }
            names.first().map(|s| s.to_string())
        })
        .unwrap_or_else(|| "RS256".to_string());
    Ok(OAuthProviderConfig {
        provider_name: needs.provider_name.clone(),
        authorize_url,
        token_url,
        scopes: needs.scopes.clone(),
        verification: IdentityVerification::Oidc {
            issuer,
            jwks_url,
            algorithm,
        },
        client_id: needs.client_id.clone(),
        client_secret: needs.client_secret.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_auth::identity::{TokenExchange, TokenResponse};
    use corvid_ast::{
        CookieConfig, Ident, IdentityProvider, ProviderKind, ProvisioningPolicy, SessionConfig,
        Span,
    };

    struct NullGateway;
    impl ProviderGateway for NullGateway {
        fn exchange_code(
            &self,
            _c: &OAuthProviderConfig,
            _e: &TokenExchange<'_>,
        ) -> Result<TokenResponse, String> {
            Err("unused".to_string())
        }
        fn fetch_userinfo(&self, _u: &str, _a: &str) -> Result<serde_json::Value, String> {
            Err("unused".to_string())
        }
    }

    fn identity(
        providers: Vec<ProviderKind>,
        first_login: AstFirstLogin,
        tenant: TenantAssignment,
        session: Option<SessionConfig>,
    ) -> IdentityDecl {
        IdentityDecl {
            name: Ident {
                name: "users".to_string(),
                span: Span::new(0, 0),
            },
            providers: providers
                .into_iter()
                .map(|kind| IdentityProvider {
                    kind,
                    span: Span::new(0, 0),
                })
                .collect(),
            session,
            linking: None,
            provisioning: Some(ProvisioningPolicy {
                first_login,
                tenant,
                default_role: None,
                span: Span::new(0, 0),
            }),
            roles: Vec::new(),
            span: Span::new(0, 0),
        }
    }

    fn build(id: &IdentityDecl, env: &[(&str, &str)]) -> Result<AuthContext, String> {
        let map: HashMap<String, String> =
            env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let reader = move |key: &str| map.get(key).cloned();
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let jwks = Arc::new(corvid_runtime::jwt_verify::mock_idp::MockIdp::new("i", "a").fetcher());
        AuthContext::build(id, auth, Arc::new(NullGateway), jwks, &reader)
    }

    #[test]
    fn open_fixed_google_translates_and_resolves_the_provider() {
        let id = identity(
            vec![ProviderKind::Google],
            AstFirstLogin::Open,
            TenantAssignment::Fixed("public".to_string()),
            None,
        );
        let ctx = build(
            &id,
            &[
                ("CORVID_OAUTH_GOOGLE_CLIENT_ID", "gid"),
                ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "gsecret"),
            ],
        )
        .unwrap();
        assert!(matches!(ctx.first_login, FirstLoginPolicy::Open));
        assert!(matches!(ctx.tenant, TenantSource::Fixed(ref t) if t == "public"));
        assert!(ctx.tenant_claim_name.is_none());
        assert!(ctx.providers.contains_key("google"));
        // Safe cookie defaults.
        assert!(ctx.cookie.secure && ctx.cookie.http_only);
        assert_eq!(ctx.cookie.same_site, "Strict");
    }

    #[test]
    fn invited_from_invitation_translates() {
        let id = identity(
            vec![ProviderKind::Github],
            AstFirstLogin::Invited,
            TenantAssignment::FromInvitation,
            None,
        );
        let ctx = build(
            &id,
            &[
                ("CORVID_OAUTH_GITHUB_CLIENT_ID", "id"),
                ("CORVID_OAUTH_GITHUB_CLIENT_SECRET", "secret"),
            ],
        )
        .unwrap();
        assert!(matches!(ctx.first_login, FirstLoginPolicy::Invited));
        assert!(matches!(ctx.tenant, TenantSource::FromInvitation));
        assert!(ctx.providers.contains_key("github"));
    }

    #[test]
    fn claim_mapping_sets_the_tenant_claim_name() {
        let id = identity(
            vec![ProviderKind::Google],
            AstFirstLogin::Open,
            TenantAssignment::ClaimMapping {
                claim: "org_id".to_string(),
                allowlist: vec!["acme".to_string()],
            },
            None,
        );
        let ctx = build(
            &id,
            &[
                ("CORVID_OAUTH_GOOGLE_CLIENT_ID", "gid"),
                ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "gsecret"),
            ],
        )
        .unwrap();
        assert_eq!(ctx.tenant_claim_name.as_deref(), Some("org_id"));
        assert!(matches!(ctx.tenant, TenantSource::Claim { .. }));
    }

    #[test]
    fn a_missing_credential_refuses_to_build() {
        let id = identity(
            vec![ProviderKind::Google],
            AstFirstLogin::Open,
            TenantAssignment::Fixed("public".to_string()),
            None,
        );
        let err = build(&id, &[]).err().unwrap();
        assert!(err.contains("CORVID_OAUTH_GOOGLE_CLIENT_ID"), "got: {err}");
    }

    #[test]
    fn custom_cookie_posture_is_carried_through() {
        let session = SessionConfig {
            lifetime_secs: Some(3600),
            cookie: CookieConfig {
                secure: true,
                http_only: true,
                same_site: SameSite::Lax,
                insecure_opt_out: false,
            },
            rotate_on_privilege_change: true,
            span: Span::new(0, 0),
        };
        let id = identity(
            vec![ProviderKind::Google],
            AstFirstLogin::Open,
            TenantAssignment::Fixed("public".to_string()),
            Some(session),
        );
        let ctx = build(
            &id,
            &[
                ("CORVID_OAUTH_GOOGLE_CLIENT_ID", "gid"),
                ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "gsecret"),
            ],
        )
        .unwrap();
        assert_eq!(ctx.cookie.same_site, "Lax");
        assert_eq!(ctx.session_lifetime_secs, 3600);
    }
}
