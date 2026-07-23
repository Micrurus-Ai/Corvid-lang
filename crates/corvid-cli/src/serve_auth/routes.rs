//! The login HTTP surface (slice 52e): the four `/auth/...` routes an
//! `identity` block implies, wired to the auth runtime.
//!
//! The callback runs a strict order and never trusts a client-supplied
//! identity:
//!
//! 1. validate the single-use `state` (which also carries the PKCE
//!    verifier + the login nonce fingerprint),
//! 2. exchange the authorization code for tokens,
//! 3. verify the ID token (signature / iss / aud / exp / nonce) or fetch
//!    the provider userinfo — establishing a server-verified identity,
//! 4. recognise `(issuer, subject)` or provision under the declared
//!    policy,
//! 5. issue a session and set a Secure/HttpOnly cookie.
//!
//! Any failure yields a generic 401 — the specific reason is audited,
//! not leaked to the caller.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use corvid_runtime::{OAuthStateCreate, OAuthStatePurpose, ProvisioningRequest, SessionCreate};
use serde_json::json;

use super::context::{AuthContext, LOGIN_STATE_TENANT};
use super::identity::{nonce_fingerprint, resolve_identity, IdentityResolution, TokenExchange};
use super::pkce::{random_token, LoginMaterial};
use super::provider::{IdentityVerification, OAuthProviderConfig};

/// Time-to-live for a pending login's OAuth state row.
const LOGIN_STATE_TTL_MS: u64 = 10 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The `/auth/...` router. Finalised with the `AuthContext` state and
/// merged into the main serve app.
pub fn auth_router() -> Router<Arc<AuthContext>> {
    Router::new()
        .route("/auth/:provider/login", get(login))
        .route("/auth/:provider/callback", get(callback))
        .route("/auth/logout", post(logout))
        .route("/auth/session", get(session))
}

#[derive(serde::Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// `GET /auth/{provider}/login` — mint PKCE/state/nonce, store the
/// single-use login state, and 302 to the provider's authorize URL.
async fn login(
    State(ctx): State<Arc<AuthContext>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(config) = ctx.providers.get(&provider).cloned() else {
        return (StatusCode::NOT_FOUND, "unknown login provider").into_response();
    };
    let material = LoginMaterial::generate();
    let redirect_uri = callback_redirect_uri(&ctx, &headers, &provider);
    let is_oidc = matches!(config.verification, IdentityVerification::Oidc { .. });

    let state_row = OAuthStateCreate {
        id: format!("oauthstate-{}", random_token()),
        provider: provider.clone(),
        tenant_id: LOGIN_STATE_TENANT.to_string(),
        actor_id: None,
        purpose: OAuthStatePurpose::Login,
        raw_state: material.state.clone(),
        pkce_verifier_ref: material.verifier.clone(),
        nonce_fingerprint: nonce_fingerprint(&material.nonce),
        expires_ms: now_ms().saturating_add(LOGIN_STATE_TTL_MS),
        replay_key: material.state.clone(),
    };
    if ctx.auth.create_oauth_state(state_row).is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not start login").into_response();
    }

    let mut params: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", &config.client_id),
        ("redirect_uri", &redirect_uri),
        ("scope", &config.scopes),
        ("state", &material.state),
    ];
    // PKCE + nonce apply to the OIDC providers; the OAuth2-only providers
    // rely on the client secret + state.
    if is_oidc {
        params.push(("nonce", &material.nonce));
        params.push(("code_challenge", &material.challenge));
        params.push(("code_challenge_method", "S256"));
    }
    let authorize = match reqwest::Url::parse_with_params(&config.authorize_url, &params) {
        Ok(url) => url,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "invalid authorize url").into_response()
        }
    };
    redirect_to(authorize.as_str())
}

/// `GET /auth/{provider}/callback` — the strict callback order.
async fn callback(
    State(ctx): State<Arc<AuthContext>>,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(config) = ctx.providers.get(&provider).cloned() else {
        return (StatusCode::NOT_FOUND, "unknown login provider").into_response();
    };
    if query.error.is_some() {
        return (StatusCode::UNAUTHORIZED, "login was denied at the provider").into_response();
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return (StatusCode::BAD_REQUEST, "callback is missing code or state").into_response();
    };
    let redirect_uri = callback_redirect_uri(&ctx, &headers, &provider);
    let trace_id = format!("oauth-callback-{}", random_token());

    // All the blocking work — DB, the token exchange, and JWKS/userinfo
    // HTTP — runs off the async executor: `reqwest`'s blocking client
    // cannot run inside a Tokio context.
    let ctx_blocking = ctx.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        run_callback(
            &ctx_blocking,
            &config,
            &provider,
            &code,
            &state,
            &redirect_uri,
            &trace_id,
        )
    })
    .await;

    let session_token = match outcome {
        Ok(Ok(token)) => token,
        // A login failure is deliberately opaque to the caller; the
        // specific reason is in the audit log.
        Ok(Err(_)) => return (StatusCode::UNAUTHORIZED, "login failed").into_response(),
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "login error").into_response();
        }
    };

    let cookie = session_cookie(&ctx, &session_token, ctx.session_lifetime_secs);
    (
        StatusCode::FOUND,
        [
            (header::SET_COOKIE, cookie),
            (header::LOCATION, ctx.post_login_redirect.clone()),
        ],
    )
        .into_response()
}

/// The blocking heart of the callback (steps 1–5). Returns the raw
/// session token to set as a cookie, or an error whose detail stays in
/// the audit log.
fn run_callback(
    ctx: &AuthContext,
    config: &OAuthProviderConfig,
    provider: &str,
    code: &str,
    state: &str,
    redirect_uri: &str,
    trace_id: &str,
) -> Result<String, String> {
    let at = now_ms();
    // 1. Validate the single-use state (single-use / expiry / tenant),
    //    recovering the PKCE verifier and nonce fingerprint.
    let resolution = ctx
        .auth
        .resolve_oauth_callback(state, LOGIN_STATE_TENANT, trace_id, at)
        .map_err(|e| e.to_string())?;
    let state_row = resolution.state;

    // 2–4. Exchange the code and establish a server-verified identity.
    let identity = resolve_identity(
        IdentityResolution {
            config,
            exchange: TokenExchange {
                code,
                redirect_uri,
                pkce_verifier: &state_row.pkce_verifier_ref,
            },
            expected_nonce_fingerprint: Some(&state_row.nonce_fingerprint),
            tenant_claim_name: ctx.tenant_claim_name.as_deref(),
            now_ms: at,
        },
        ctx.gateway.as_ref(),
        ctx.jwks_fetcher.clone(),
    )?;

    // 5–6. Recognise the subject or provision under the declared policy.
    let provisioned = ctx
        .auth
        .provision_login(ProvisioningRequest {
            provider,
            issuer: &identity.source,
            subject: &identity.external_id,
            verified_email: identity.email.as_deref(),
            tenant_claim_value: identity.tenant_claim.as_deref(),
            display_name: identity.display_name.as_deref(),
            first_login: ctx.first_login,
            tenant: ctx.tenant.clone(),
            default_role: ctx.default_role.as_deref(),
            trace_id,
            at_ms: at,
        })
        .map_err(|e| e.to_string())?;

    // 7. Issue the session only after provisioning succeeds.
    let raw_token = random_token();
    ctx.auth
        .create_session(SessionCreate {
            id: format!("session-{}", random_token()),
            actor_id: provisioned.actor.id.clone(),
            tenant_id: provisioned.actor.tenant_id.clone(),
            raw_token: raw_token.clone(),
            issued_ms: at,
            expires_ms: at.saturating_add(ctx.session_lifetime_secs.saturating_mul(1000)),
            csrf_binding_id: random_token(),
        })
        .map_err(|e| e.to_string())?;
    Ok(raw_token)
}

/// `POST /auth/logout` — revoke the session and clear the cookie.
async fn logout(State(ctx): State<Arc<AuthContext>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, &ctx.cookie.name) {
        let at = now_ms();
        let trace = format!("logout-{}", random_token());
        if let Ok(resolution) = ctx.auth.resolve_session_cookie(&token, &trace, at) {
            let _ = ctx.auth.revoke_session(&resolution.session.id, at);
        }
    }
    (
        StatusCode::FOUND,
        [
            (header::SET_COOKIE, cleared_cookie(&ctx)),
            (header::LOCATION, "/".to_string()),
        ],
    )
        .into_response()
}

/// `GET /auth/session` — the current actor, or `authenticated: false`.
async fn session(State(ctx): State<Arc<AuthContext>>, headers: HeaderMap) -> Response {
    let Some(token) = cookie_value(&headers, &ctx.cookie.name) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "authenticated": false })),
        )
            .into_response();
    };
    let at = now_ms();
    let trace = format!("session-{}", random_token());
    match ctx.auth.resolve_session_cookie(&token, &trace, at) {
        Ok(resolution) => Json(json!({
            "authenticated": true,
            "actor": {
                "id": resolution.actor.id,
                "tenant": resolution.actor.tenant_id,
                "display_name": resolution.actor.display_name,
            }
        }))
        .into_response(),
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "authenticated": false })),
        )
            .into_response(),
    }
}

/// The provider `redirect_uri` — the configured public base URL, else
/// derived from the request `Host` header (http for local dev).
fn callback_redirect_uri(ctx: &AuthContext, headers: &HeaderMap, provider: &str) -> String {
    let base = ctx.public_base_url.clone().unwrap_or_else(|| {
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        format!("http://{host}")
    });
    let base = base.trim_end_matches('/');
    format!("{base}/auth/{provider}/callback")
}

fn redirect_to(location: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location.to_string())]).into_response()
}

fn session_cookie(ctx: &AuthContext, token: &str, max_age_secs: u64) -> String {
    build_cookie(ctx, token, max_age_secs)
}

fn cleared_cookie(ctx: &AuthContext) -> String {
    build_cookie(ctx, "", 0)
}

fn build_cookie(ctx: &AuthContext, value: &str, max_age_secs: u64) -> String {
    let mut cookie = format!(
        "{}={}; Path=/; Max-Age={}",
        ctx.cookie.name, value, max_age_secs
    );
    if ctx.cookie.http_only {
        cookie.push_str("; HttpOnly");
    }
    if ctx.cookie.secure {
        cookie.push_str("; Secure");
    }
    cookie.push_str("; SameSite=");
    cookie.push_str(ctx.cookie.same_site);
    cookie
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    raw.split(';')
        .map(str::trim)
        .find_map(|pair| pair.strip_prefix(&prefix).map(str::to_string))
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_cookie(secure: bool, http_only: bool, same_site: &'static str) -> AuthContext {
        use super::super::context::CookieSettings;
        use corvid_runtime::jwt_verify::mock_idp::MockIdp;
        use corvid_runtime::{FirstLoginPolicy, SessionAuthRuntime, TenantSource};
        use std::collections::HashMap;

        struct NullGw;
        impl super::super::identity::ProviderGateway for NullGw {
            fn exchange_code(
                &self,
                _c: &OAuthProviderConfig,
                _e: &TokenExchange<'_>,
            ) -> Result<super::super::identity::TokenResponse, String> {
                Err("x".into())
            }
            fn fetch_userinfo(&self, _u: &str, _a: &str) -> Result<serde_json::Value, String> {
                Err("x".into())
            }
        }
        AuthContext {
            auth: Arc::new(SessionAuthRuntime::open_in_memory().unwrap()),
            providers: HashMap::new(),
            first_login: FirstLoginPolicy::Open,
            tenant: TenantSource::Fixed("public".to_string()),
            tenant_claim_name: None,
            default_role: None,
            cookie: CookieSettings {
                name: "corvid_session".to_string(),
                secure,
                http_only,
                same_site,
            },
            session_lifetime_secs: 3600,
            gateway: Arc::new(NullGw),
            jwks_fetcher: Arc::new(MockIdp::new("i", "a").fetcher()),
            public_base_url: None,
            post_login_redirect: "/".to_string(),
        }
    }

    #[test]
    fn session_cookie_carries_the_declared_flags() {
        let ctx = ctx_with_cookie(true, true, "Strict");
        let cookie = session_cookie(&ctx, "tok", 3600);
        assert!(cookie.starts_with("corvid_session=tok"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Max-Age=3600"));
    }

    #[test]
    fn cleared_cookie_expires_immediately() {
        let ctx = ctx_with_cookie(true, true, "Lax");
        let cookie = cleared_cookie(&ctx);
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("SameSite=Lax"));
    }

    #[test]
    fn insecure_posture_omits_secure_flag() {
        let ctx = ctx_with_cookie(false, false, "None");
        let cookie = session_cookie(&ctx, "tok", 60);
        assert!(!cookie.contains("Secure"));
        assert!(!cookie.contains("HttpOnly"));
    }

    #[test]
    fn cookie_value_parses_from_a_multi_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "other=1; corvid_session=the-token; last=2".parse().unwrap(),
        );
        assert_eq!(
            cookie_value(&headers, "corvid_session").as_deref(),
            Some("the-token")
        );
        assert_eq!(cookie_value(&headers, "absent"), None);
    }
}

/// The strict callback order, driven end-to-end against a mock IdP and a
/// mock provider gateway — no live network, deterministic.
#[cfg(test)]
mod callback_tests {
    use super::*;
    use crate::serve_auth::context::CookieSettings;
    use crate::serve_auth::identity::{ProviderGateway, TokenExchange, TokenResponse};
    use crate::serve_auth::provider::IdentityVerification;
    use corvid_runtime::jwt_verify::mock_idp::MockIdp;
    use corvid_runtime::{
        FirstLoginPolicy, InvitationCreate, SessionAuthRuntime, TenantSource,
    };
    use std::collections::HashMap;

    const ISSUER: &str = "https://issuer.test";
    const AUDIENCE: &str = "corvid-test";

    struct StubGateway {
        token: TokenResponse,
        userinfo: serde_json::Value,
    }
    impl ProviderGateway for StubGateway {
        fn exchange_code(
            &self,
            _c: &OAuthProviderConfig,
            _e: &TokenExchange<'_>,
        ) -> Result<TokenResponse, String> {
            Ok(self.token.clone())
        }
        fn fetch_userinfo(&self, _u: &str, _a: &str) -> Result<serde_json::Value, String> {
            Ok(self.userinfo.clone())
        }
    }

    fn oidc_config() -> OAuthProviderConfig {
        OAuthProviderConfig {
            provider_name: "google".to_string(),
            authorize_url: "https://issuer.test/authorize".to_string(),
            token_url: "https://issuer.test/token".to_string(),
            scopes: "openid email".to_string(),
            verification: IdentityVerification::Oidc {
                issuer: ISSUER.to_string(),
                jwks_url: "https://issuer.test/jwks".to_string(),
                algorithm: "EdDSA".to_string(),
            },
            client_id: AUDIENCE.to_string(),
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

    fn ctx(
        auth: Arc<SessionAuthRuntime>,
        gateway: Arc<dyn ProviderGateway + Send + Sync>,
        jwks: Arc<dyn corvid_runtime::jwt_verify::JwksFetcher>,
        first_login: FirstLoginPolicy,
        tenant: TenantSource,
    ) -> AuthContext {
        AuthContext {
            auth,
            providers: HashMap::new(),
            first_login,
            tenant,
            tenant_claim_name: None,
            default_role: None,
            cookie: CookieSettings::default(),
            session_lifetime_secs: 3600,
            gateway,
            jwks_fetcher: jwks,
            public_base_url: None,
            post_login_redirect: "/".to_string(),
        }
    }

    /// Seed a pending Login OAuth state and return its raw `state` value.
    fn seed_login_state(auth: &SessionAuthRuntime, id: &str, nonce: &str) -> String {
        let state = format!("state-{id}");
        auth.create_oauth_state(OAuthStateCreate {
            id: format!("os-{id}"),
            provider: "google".to_string(),
            tenant_id: LOGIN_STATE_TENANT.to_string(),
            actor_id: None,
            purpose: OAuthStatePurpose::Login,
            raw_state: state.clone(),
            pkce_verifier_ref: "verifier".to_string(),
            nonce_fingerprint: nonce_fingerprint(nonce),
            expires_ms: now_ms().saturating_add(600_000),
            replay_key: state.clone(),
        })
        .unwrap();
        state
    }

    fn id_token(idp: &MockIdp, sub: &str, nonce: &str, email: &str) -> String {
        idp.mint_with(|p| {
            p["iss"] = ISSUER.into();
            p["aud"] = AUDIENCE.into();
            p["sub"] = sub.into();
            p["email"] = email.into();
            p["nonce"] = nonce.into();
            p.as_object_mut().unwrap().remove("tenant");
        })
    }

    #[test]
    fn open_login_provisions_an_actor_then_recognises_it() {
        let idp = MockIdp::new(ISSUER, AUDIENCE);
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let nonce = "login-nonce";
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "at".to_string(),
                id_token: Some(id_token(&idp, "sub-1", nonce, "ada@example.com")),
            },
            userinfo: serde_json::Value::Null,
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Open,
            TenantSource::Fixed("public".to_string()),
        );
        let config = oidc_config();

        // First login → a session for a freshly provisioned actor.
        let state1 = seed_login_state(&auth, "1", nonce);
        let token1 = run_callback(
            &context, &config, "google", "code", &state1, "https://app/cb", "trace-1",
        )
        .expect("first callback succeeds");
        let session1 = auth
            .resolve_session_cookie(&token1, "check-1", now_ms())
            .expect("session resolves");
        assert_eq!(session1.actor.tenant_id, "public");
        assert_eq!(session1.actor.auth_method, "oauth");
        let actor_id = session1.actor.id.clone();

        // Second login with the same subject → the SAME actor, recognised.
        let state2 = seed_login_state(&auth, "2", nonce);
        let token2 = run_callback(
            &context, &config, "google", "code", &state2, "https://app/cb", "trace-2",
        )
        .expect("second callback succeeds");
        let session2 = auth
            .resolve_session_cookie(&token2, "check-2", now_ms())
            .unwrap();
        assert_eq!(session2.actor.id, actor_id, "same subject → same actor");
    }

    #[test]
    fn a_reused_state_is_refused() {
        let idp = MockIdp::new(ISSUER, AUDIENCE);
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let nonce = "n";
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "at".to_string(),
                id_token: Some(id_token(&idp, "sub-1", nonce, "ada@example.com")),
            },
            userinfo: serde_json::Value::Null,
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Open,
            TenantSource::Fixed("public".to_string()),
        );
        let config = oidc_config();
        let state = seed_login_state(&auth, "1", nonce);
        run_callback(&context, &config, "google", "c", &state, "cb", "t1").unwrap();
        // Second use of the same state → single-use violation.
        let err = run_callback(&context, &config, "google", "c", &state, "cb", "t2").unwrap_err();
        assert!(err.contains("already used") || err.contains("not found"), "got: {err}");
    }

    #[test]
    fn a_nonce_mismatch_is_refused() {
        let idp = MockIdp::new(ISSUER, AUDIENCE);
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        // The token carries a DIFFERENT nonce than the login state.
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "at".to_string(),
                id_token: Some(id_token(&idp, "sub-1", "attacker-nonce", "e@x.com")),
            },
            userinfo: serde_json::Value::Null,
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Open,
            TenantSource::Fixed("public".to_string()),
        );
        let config = oidc_config();
        let state = seed_login_state(&auth, "1", "the-real-nonce");
        let err = run_callback(&context, &config, "google", "c", &state, "cb", "t").unwrap_err();
        assert!(err.contains("nonce"), "got: {err}");
    }

    #[test]
    fn a_tampered_token_is_refused() {
        let idp = MockIdp::new(ISSUER, AUDIENCE);
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "at".to_string(),
                id_token: Some(idp.mint_tampered_signature()),
            },
            userinfo: serde_json::Value::Null,
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Open,
            TenantSource::Fixed("public".to_string()),
        );
        let config = oidc_config();
        let state = seed_login_state(&auth, "1", "n");
        let err = run_callback(&context, &config, "google", "c", &state, "cb", "t").unwrap_err();
        assert!(err.contains("verification failed"), "got: {err}");
    }

    #[test]
    fn invited_login_refuses_without_an_invitation_and_admits_with_one() {
        let idp = MockIdp::new(ISSUER, AUDIENCE);
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let nonce = "n";
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "at".to_string(),
                id_token: Some(id_token(&idp, "sub-1", nonce, "ada@example.com")),
            },
            userinfo: serde_json::Value::Null,
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Invited,
            TenantSource::FromInvitation,
        );
        let config = oidc_config();

        // No invitation → refused.
        let state1 = seed_login_state(&auth, "1", nonce);
        let err = run_callback(&context, &config, "google", "c", &state1, "cb", "t1").unwrap_err();
        assert!(err.contains("no open invitation"), "got: {err}");

        // Create the invitation → the next login is admitted into its tenant.
        auth.create_invitation(InvitationCreate {
            id: "inv-1".to_string(),
            email: "ada@example.com".to_string(),
            tenant_id: "acme".to_string(),
            role: "member".to_string(),
            expires_ms: Some(now_ms() + 600_000),
        })
        .unwrap();
        let state2 = seed_login_state(&auth, "2", nonce);
        let token = run_callback(&context, &config, "google", "c", &state2, "cb", "t2")
            .expect("invited login with an invitation succeeds");
        let session = auth
            .resolve_session_cookie(&token, "check", now_ms())
            .unwrap();
        assert_eq!(session.actor.tenant_id, "acme");
    }

    #[test]
    fn userinfo_login_provisions_from_the_provider_user_id() {
        let idp = MockIdp::new("unused", "unused");
        let auth = Arc::new(SessionAuthRuntime::open_in_memory().unwrap());
        let gateway = Arc::new(StubGateway {
            token: TokenResponse {
                access_token: "gho_token".to_string(),
                id_token: None,
            },
            userinfo: serde_json::json!({ "id": 4210, "login": "ada", "email": "ada@x.com" }),
        });
        let context = ctx(
            auth.clone(),
            gateway,
            Arc::new(idp.fetcher()),
            FirstLoginPolicy::Open,
            TenantSource::Fixed("public".to_string()),
        );
        let config = userinfo_config();
        // A userinfo login needs no nonce; seed any state.
        let state = seed_login_state(&auth, "1", "n");
        let token = run_callback(&context, &config, "github", "c", &state, "cb", "t")
            .expect("userinfo login succeeds");
        let session = auth
            .resolve_session_cookie(&token, "check", now_ms())
            .unwrap();
        assert_eq!(session.actor.tenant_id, "public");
        assert_eq!(session.actor.display_name, "ada");
    }
}
