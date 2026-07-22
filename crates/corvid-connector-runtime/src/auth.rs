use crate::manifest::ConnectorAuthorize;
use std::collections::BTreeSet;

/// Public Corvid guarantee id this module enforces (slice 51j):
/// `connector.per_user_token_separate_from_session`. Declared as a
/// literal so the `corvid-guarantees` inverse-coverage sentinel finds
/// the enforcement site (`ConnectorAuthState::authorize` refusing a
/// `CredentialKind::LoginSession` credential) wired to the registry
/// row. `allow(dead_code)` is the load-bearing attribute.
#[allow(dead_code)]
pub const GUARANTEE_ID_CONNECTOR_TOKEN_SEPARATION: &str =
    "connector.per_user_token_separate_from_session";

/// What kind of credential a token is (slice 51j). A connector call is
/// authorized ONLY by a `ConnectorAccess` credential; a `LoginSession`
/// credential (the identity token from the `identity` block) is refused
/// at the connector boundary. This makes "the login session is not a
/// workspace access token" a runtime-enforced guarantee, not a
/// convention — the two are different credentials that never
/// interchange, even though a per-user connector token is bound to the
/// same end-user actor as their login session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CredentialKind {
    #[default]
    ConnectorAccess,
    LoginSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorAuthState {
    pub tenant_id: String,
    pub actor_id: String,
    pub token_id: String,
    pub scopes: BTreeSet<String>,
    pub expires_at_ms: u64,
    /// The ownership mode the token was issued under (slice 51j).
    pub authorize: ConnectorAuthorize,
    /// The credential kind — always `ConnectorAccess` for a token that
    /// may authorize a connector call.
    pub credential_kind: CredentialKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectorAuthError {
    MissingTenant,
    MissingActor,
    MissingToken,
    ExpiredToken,
    RevokedRefreshToken,
    TenantMismatch,
    MissingScope(String),
    /// The presented credential is not a connector access token
    /// (slice 51j) — e.g. a login-session identity token. The login
    /// session and connector workspace tokens never interchange.
    NotAConnectorCredential,
    /// A `per_user` connector was authorized without an end-user
    /// actor (slice 51j).
    PerUserRequiresEndUser,
}

impl std::fmt::Display for ConnectorAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTenant => write!(f, "connector auth requires tenant_id"),
            Self::MissingActor => write!(f, "connector auth requires actor_id"),
            Self::MissingToken => write!(f, "connector auth requires token_id"),
            Self::ExpiredToken => write!(f, "connector auth token is expired"),
            Self::RevokedRefreshToken => write!(f, "connector refresh token is revoked"),
            Self::TenantMismatch => write!(f, "connector refresh token tenant mismatch"),
            Self::MissingScope(scope) => write!(f, "connector token missing scope `{scope}`"),
            Self::NotAConnectorCredential => write!(
                f,
                "credential is not a connector access token (a login-session identity token cannot authorize a connector)"
            ),
            Self::PerUserRequiresEndUser => write!(
                f,
                "a per_user connector requires an end-user actor to authorize"
            ),
        }
    }
}

impl std::error::Error for ConnectorAuthError {}

impl ConnectorAuthState {
    pub fn new(
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        token_id: impl Into<String>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
        expires_at_ms: u64,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            actor_id: actor_id.into(),
            token_id: token_id.into(),
            scopes: scopes.into_iter().map(Into::into).collect(),
            expires_at_ms,
            authorize: ConnectorAuthorize::Workspace,
            credential_kind: CredentialKind::ConnectorAccess,
        }
    }

    /// Mark this token as issued under a per-user authorization
    /// (slice 51j) — its `actor_id` is the consenting end user.
    pub fn per_user(mut self) -> Self {
        self.authorize = ConnectorAuthorize::PerUser;
        self
    }

    /// Stamp a credential kind (slice 51j). Only `ConnectorAccess`
    /// credentials pass `authorize`.
    pub fn with_credential_kind(mut self, kind: CredentialKind) -> Self {
        self.credential_kind = kind;
        self
    }

    pub fn authorize(&self, required_scope: &str, now_ms: u64) -> Result<(), ConnectorAuthError> {
        // A login-session identity token is not a connector credential
        // (slice 51j) — the two never interchange.
        if self.credential_kind != CredentialKind::ConnectorAccess {
            return Err(ConnectorAuthError::NotAConnectorCredential);
        }
        if self.tenant_id.trim().is_empty() {
            return Err(ConnectorAuthError::MissingTenant);
        }
        if self.actor_id.trim().is_empty() {
            // A per-user connector has no meaning without the end user;
            // a workspace connector still needs its service actor.
            return Err(if self.authorize == ConnectorAuthorize::PerUser {
                ConnectorAuthError::PerUserRequiresEndUser
            } else {
                ConnectorAuthError::MissingActor
            });
        }
        if self.token_id.trim().is_empty() {
            return Err(ConnectorAuthError::MissingToken);
        }
        if self.expires_at_ms <= now_ms {
            return Err(ConnectorAuthError::ExpiredToken);
        }
        if !self.scopes.contains(required_scope) {
            return Err(ConnectorAuthError::MissingScope(required_scope.to_string()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorRefreshTokenState {
    pub tenant_id: String,
    pub actor_id: String,
    pub refresh_token_id: String,
    pub scopes: BTreeSet<String>,
    pub revoked: bool,
    /// The ownership mode minted access tokens inherit (slice 51j).
    pub authorize: ConnectorAuthorize,
}

impl ConnectorRefreshTokenState {
    pub fn new(
        tenant_id: impl Into<String>,
        actor_id: impl Into<String>,
        refresh_token_id: impl Into<String>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            actor_id: actor_id.into(),
            refresh_token_id: refresh_token_id.into(),
            scopes: scopes.into_iter().map(Into::into).collect(),
            revoked: false,
            authorize: ConnectorAuthorize::Workspace,
        }
    }

    /// Mark this refresh token (and the access tokens it mints) as
    /// per-user (slice 51j).
    pub fn per_user(mut self) -> Self {
        self.authorize = ConnectorAuthorize::PerUser;
        self
    }

    pub fn refresh(
        &self,
        tenant_id: &str,
        new_token_id: impl Into<String>,
        expires_at_ms: u64,
    ) -> Result<ConnectorAuthState, ConnectorAuthError> {
        if self.revoked {
            return Err(ConnectorAuthError::RevokedRefreshToken);
        }
        if self.tenant_id != tenant_id {
            return Err(ConnectorAuthError::TenantMismatch);
        }
        Ok(ConnectorAuthState {
            tenant_id: self.tenant_id.clone(),
            actor_id: self.actor_id.clone(),
            token_id: new_token_id.into(),
            scopes: self.scopes.clone(),
            expires_at_ms,
            authorize: self.authorize,
            credential_kind: CredentialKind::ConnectorAccess,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_token_mints_tenant_scoped_access_state() {
        let refresh = ConnectorRefreshTokenState::new(
            "tenant-1",
            "actor-1",
            "refresh-1",
            ["ms365.mail_search", "ms365.calendar_events"],
        );
        let access = refresh.refresh("tenant-1", "access-2", 1000).unwrap();
        assert_eq!(access.tenant_id, "tenant-1");
        assert_eq!(access.actor_id, "actor-1");
        assert!(access.scopes.contains("ms365.mail_search"));
        access.authorize("ms365.calendar_events", 1).unwrap();
    }

    #[test]
    fn login_session_credential_cannot_authorize_a_connector() {
        // Slice 51j — a login-session identity token presented at the
        // connector boundary is refused; the login session and the
        // connector workspace token never interchange.
        let state = ConnectorAuthState::new("tenant-1", "user-1", "tok-1", ["gmail.read"], 1000)
            .with_credential_kind(CredentialKind::LoginSession);
        assert_eq!(
            state.authorize("gmail.read", 1),
            Err(ConnectorAuthError::NotAConnectorCredential)
        );
    }

    #[test]
    fn per_user_connector_requires_an_end_user_actor() {
        // Slice 51j — a per_user connector token without an end-user
        // actor is refused with the per-user-specific error.
        let state = ConnectorAuthState::new("tenant-1", "", "tok-1", ["gmail.read"], 1000).per_user();
        assert_eq!(
            state.authorize("gmail.read", 1),
            Err(ConnectorAuthError::PerUserRequiresEndUser)
        );
        // With the end user present it authorizes normally.
        let ok = ConnectorAuthState::new("tenant-1", "user-1", "tok-1", ["gmail.read"], 1000)
            .per_user();
        assert!(ok.authorize("gmail.read", 1).is_ok());
        assert_eq!(ok.authorize, ConnectorAuthorize::PerUser);
    }

    #[test]
    fn refresh_rejects_cross_tenant_or_revoked_token() {
        let refresh = ConnectorRefreshTokenState::new("tenant-1", "actor-1", "refresh-1", ["a"]);
        let err = refresh.refresh("tenant-2", "access", 100).unwrap_err();
        assert_eq!(err, ConnectorAuthError::TenantMismatch);

        let mut revoked = refresh.clone();
        revoked.revoked = true;
        let err = revoked.refresh("tenant-1", "access", 100).unwrap_err();
        assert_eq!(err, ConnectorAuthError::RevokedRefreshToken);
    }
}
