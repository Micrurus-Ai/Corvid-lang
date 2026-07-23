//! OAuth/OIDC provider configuration resolution (slice 52e).
//!
//! Maps a declared `provider` to the concrete endpoints a login flow
//! needs, plus the method by which the callback establishes a
//! server-verified identity:
//!
//! - **OIDC providers** (google/microsoft/apple, and generic `oidc`)
//!   return an ID token the callback verifies via JWKS; identity is the
//!   token's `(issuer, subject)`.
//! - **OAuth2-only providers** (github/slack/discord) do not issue a
//!   verifiable ID token, so the callback fetches the provider's own
//!   user endpoint over TLS after code exchange; identity is
//!   `(provider_marker, authoritative_user_id)`.
//!
//! Either way the identity is established SERVER-SIDE from a value the
//! provider vouches for — never a client-forgeable claim.
//!
//! Client credentials are never hardcoded: they are read from the
//! environment (`CORVID_OAUTH_<PROVIDER>_CLIENT_ID` /
//! `_CLIENT_SECRET`). A missing credential is a startup error, not a
//! silent empty string — a login route that cannot authenticate is not
//! executable.

use corvid_ast::{IdentityProvider, ProviderKind};

/// How the callback turns a successful code exchange into a verified
/// external identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityVerification {
    /// Verify the ID token against the issuer's JWKS; identity is
    /// `(issuer, subject)`. `algorithm` pins the expected signing
    /// algorithm (the verifier still refuses `alg=none` and any
    /// unsupported alg, and rejects a header/contract mismatch).
    Oidc {
        issuer: String,
        jwks_url: String,
        algorithm: String,
    },
    /// Fetch the provider's user endpoint server-side; identity is
    /// `(source_marker, authoritative_user_id)`.
    Userinfo {
        source_marker: String,
        userinfo_url: String,
    },
}

/// The resolved configuration for one login provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProviderConfig {
    /// The name used in the `/auth/{provider}/…` route path (`google`,
    /// `github`, or the `oidc` alias).
    pub provider_name: String,
    pub authorize_url: String,
    pub token_url: String,
    /// Space-separated OAuth scopes requested at authorize time.
    pub scopes: String,
    pub verification: IdentityVerification,
    pub client_id: String,
    pub client_secret: String,
}

/// A provider that still needs its endpoints resolved from an OIDC
/// discovery document before it is usable — the generic `oidc`
/// provider. The discovery fetch is an HTTP step handled in a later
/// slice; this keeps configuration resolution pure and testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryNeeded {
    pub provider_name: String,
    pub discovery_url: String,
    pub scopes: String,
    pub client_id: String,
    pub client_secret: String,
}

/// The outcome of resolving a declared provider: either a ready config
/// (well-known preset) or a provider awaiting OIDC discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedProvider {
    Ready(Box<OAuthProviderConfig>),
    NeedsDiscovery(Box<DiscoveryNeeded>),
}

/// The environment variable name a provider's client id is read from.
pub fn client_id_var(provider_name: &str) -> String {
    format!("CORVID_OAUTH_{}_CLIENT_ID", env_key(provider_name))
}

/// The environment variable name a provider's client secret is read
/// from.
pub fn client_secret_var(provider_name: &str) -> String {
    format!("CORVID_OAUTH_{}_CLIENT_SECRET", env_key(provider_name))
}

fn env_key(provider_name: &str) -> String {
    provider_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
        .collect()
}

/// Resolve a declared provider into its configuration. `env` reads an
/// environment variable (injected so tests need not touch the real
/// process environment). Client credentials are required — a missing
/// one is an error naming the variable to set.
pub fn resolve_provider(
    provider: &IdentityProvider,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<ResolvedProvider, String> {
    let preset = preset_for(&provider.kind);
    let (client_id, client_secret) = read_credentials(&preset.provider_name, env)?;
    match preset.endpoints {
        PresetEndpoints::Discovery { discovery_url } => {
            Ok(ResolvedProvider::NeedsDiscovery(Box::new(DiscoveryNeeded {
                provider_name: preset.provider_name,
                discovery_url,
                scopes: preset.scopes,
                client_id,
                client_secret,
            })))
        }
        PresetEndpoints::Ready {
            authorize_url,
            token_url,
            verification,
        } => Ok(ResolvedProvider::Ready(Box::new(OAuthProviderConfig {
            provider_name: preset.provider_name,
            authorize_url,
            token_url,
            scopes: preset.scopes,
            verification,
            client_id,
            client_secret,
        }))),
    }
}

fn read_credentials(
    provider_name: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<(String, String), String> {
    let id_var = client_id_var(provider_name);
    let secret_var = client_secret_var(provider_name);
    let client_id = env(&id_var).filter(|v| !v.trim().is_empty()).ok_or_else(|| {
        format!("missing OAuth client id for provider `{provider_name}`: set `{id_var}`")
    })?;
    let client_secret = env(&secret_var)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!("missing OAuth client secret for provider `{provider_name}`: set `{secret_var}`")
        })?;
    Ok((client_id, client_secret))
}

struct Preset {
    provider_name: String,
    scopes: String,
    endpoints: PresetEndpoints,
}

enum PresetEndpoints {
    Ready {
        authorize_url: String,
        token_url: String,
        verification: IdentityVerification,
    },
    Discovery {
        discovery_url: String,
    },
}

fn preset_for(kind: &ProviderKind) -> Preset {
    match kind {
        ProviderKind::Google => Preset {
            provider_name: "google".to_string(),
            scopes: "openid email profile".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token_url: "https://oauth2.googleapis.com/token".to_string(),
                verification: IdentityVerification::Oidc {
                    issuer: "https://accounts.google.com".to_string(),
                    jwks_url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                    algorithm: "RS256".to_string(),
                },
            },
        },
        ProviderKind::Microsoft => Preset {
            provider_name: "microsoft".to_string(),
            scopes: "openid email profile".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url:
                    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
                token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
                verification: IdentityVerification::Oidc {
                    issuer: "https://login.microsoftonline.com/common/v2.0".to_string(),
                    jwks_url: "https://login.microsoftonline.com/common/discovery/v2.0/keys"
                        .to_string(),
                    algorithm: "RS256".to_string(),
                },
            },
        },
        ProviderKind::Apple => Preset {
            provider_name: "apple".to_string(),
            scopes: "openid email name".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url: "https://appleid.apple.com/auth/authorize".to_string(),
                token_url: "https://appleid.apple.com/auth/token".to_string(),
                verification: IdentityVerification::Oidc {
                    issuer: "https://appleid.apple.com".to_string(),
                    jwks_url: "https://appleid.apple.com/auth/keys".to_string(),
                    algorithm: "RS256".to_string(),
                },
            },
        },
        ProviderKind::Github => Preset {
            provider_name: "github".to_string(),
            scopes: "read:user user:email".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url: "https://github.com/login/oauth/authorize".to_string(),
                token_url: "https://github.com/login/oauth/access_token".to_string(),
                verification: IdentityVerification::Userinfo {
                    source_marker: "github".to_string(),
                    userinfo_url: "https://api.github.com/user".to_string(),
                },
            },
        },
        ProviderKind::Slack => Preset {
            provider_name: "slack".to_string(),
            scopes: "openid email profile".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url: "https://slack.com/openid/connect/authorize".to_string(),
                token_url: "https://slack.com/api/openid.connect.token".to_string(),
                verification: IdentityVerification::Userinfo {
                    source_marker: "slack".to_string(),
                    userinfo_url: "https://slack.com/api/openid.connect.userInfo".to_string(),
                },
            },
        },
        ProviderKind::Discord => Preset {
            provider_name: "discord".to_string(),
            scopes: "identify email".to_string(),
            endpoints: PresetEndpoints::Ready {
                authorize_url: "https://discord.com/api/oauth2/authorize".to_string(),
                token_url: "https://discord.com/api/oauth2/token".to_string(),
                verification: IdentityVerification::Userinfo {
                    source_marker: "discord".to_string(),
                    userinfo_url: "https://discord.com/api/users/@me".to_string(),
                },
            },
        },
        ProviderKind::Oidc { discovery_url, alias } => Preset {
            provider_name: alias.name.clone(),
            scopes: "openid email profile".to_string(),
            endpoints: PresetEndpoints::Discovery {
                discovery_url: discovery_url.clone(),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{Ident, Span};
    use std::collections::HashMap;

    fn provider(kind: ProviderKind) -> IdentityProvider {
        IdentityProvider {
            kind,
            span: Span::new(0, 0),
        }
    }

    fn env_map(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn google_resolves_to_an_oidc_preset_with_credentials() {
        let env = env_map(&[
            ("CORVID_OAUTH_GOOGLE_CLIENT_ID", "gid"),
            ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "gsecret"),
        ]);
        let resolved = resolve_provider(&provider(ProviderKind::Google), &env).unwrap();
        let ResolvedProvider::Ready(config) = resolved else {
            panic!("expected a ready config");
        };
        assert_eq!(config.provider_name, "google");
        assert_eq!(config.client_id, "gid");
        assert_eq!(config.client_secret, "gsecret");
        assert!(config.authorize_url.starts_with("https://accounts.google.com"));
        assert_eq!(
            config.verification,
            IdentityVerification::Oidc {
                issuer: "https://accounts.google.com".to_string(),
                jwks_url: "https://www.googleapis.com/oauth2/v3/certs".to_string(),
                algorithm: "RS256".to_string(),
            }
        );
    }

    #[test]
    fn github_resolves_to_a_userinfo_preset() {
        let env = env_map(&[
            ("CORVID_OAUTH_GITHUB_CLIENT_ID", "id"),
            ("CORVID_OAUTH_GITHUB_CLIENT_SECRET", "secret"),
        ]);
        let resolved = resolve_provider(&provider(ProviderKind::Github), &env).unwrap();
        let ResolvedProvider::Ready(config) = resolved else {
            panic!("expected a ready config");
        };
        assert_eq!(
            config.verification,
            IdentityVerification::Userinfo {
                source_marker: "github".to_string(),
                userinfo_url: "https://api.github.com/user".to_string(),
            }
        );
    }

    #[test]
    fn oidc_provider_needs_discovery() {
        let env = env_map(&[
            ("CORVID_OAUTH_CORP_CLIENT_ID", "id"),
            ("CORVID_OAUTH_CORP_CLIENT_SECRET", "secret"),
        ]);
        let kind = ProviderKind::Oidc {
            discovery_url: "https://issuer.example.com/.well-known/openid-configuration".to_string(),
            alias: Ident {
                name: "corp".to_string(),
                span: Span::new(0, 0),
            },
        };
        let resolved = resolve_provider(&provider(kind), &env).unwrap();
        let ResolvedProvider::NeedsDiscovery(needs) = resolved else {
            panic!("expected discovery-needed");
        };
        assert_eq!(needs.provider_name, "corp");
        assert!(needs.discovery_url.ends_with("openid-configuration"));
        assert_eq!(needs.client_id, "id");
    }

    #[test]
    fn a_missing_client_secret_is_a_named_error() {
        let env = env_map(&[("CORVID_OAUTH_GOOGLE_CLIENT_ID", "gid")]);
        let err = resolve_provider(&provider(ProviderKind::Google), &env).unwrap_err();
        assert!(err.contains("CORVID_OAUTH_GOOGLE_CLIENT_SECRET"), "got: {err}");
    }

    #[test]
    fn an_empty_credential_is_treated_as_missing() {
        let env = env_map(&[
            ("CORVID_OAUTH_GOOGLE_CLIENT_ID", "  "),
            ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "gsecret"),
        ]);
        let err = resolve_provider(&provider(ProviderKind::Google), &env).unwrap_err();
        assert!(err.contains("CORVID_OAUTH_GOOGLE_CLIENT_ID"), "got: {err}");
    }
}
