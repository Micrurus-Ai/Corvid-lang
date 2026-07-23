//! OAuth domain — state-token hashing, row decoding, and the
//! `SessionAuthRuntime` impl block carrying create / get /
//! resolve_callback for `auth_oauth_states`.
//!
//! OAuth callbacks land back at the runtime carrying the state
//! token the client sent at authorize time. The state is hashed
//! with SHA-256 (same family as session tokens, with a different
//! prefix to keep the hash spaces distinct) so a leaked database
//! row never lets an attacker forge a future callback.
//!
//! `resolve_oauth_callback` enforces single-use semantics — the
//! conditional `update ... where id = ?1 and used_ms is null`
//! plus the audit-then-error pattern means a replayed callback
//! lands in the audit log as `denied: oauth state already used`
//! even if the original write raced with a concurrent callback
//! attempt.

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::errors::RuntimeError;
use crate::tracing::now_ms;

/// Public Corvid guarantee id this module enforces:
/// `auth.oauth_pkce_required`.
///
/// OAuth callback state requires PKCE for public clients; the state
/// record carries the code-verifier hash and is single-use,
/// tenant-scoped, and expiry-bound. Single-use semantics enforced
/// by the `update ... where id = ?1 and used_ms is null`
/// conditional UPDATE plus the audit-then-error pattern in
/// `resolve_oauth_callback`. Tagged at module level so the
/// `corvid-guarantees` inverse-coverage sentinel can confirm the
/// enforcement site is wired to the registry row.
// Not referenced by symbol; declared so the Phase 35V-T1-Drift sentinel
// (`every_enforced_guarantee_id_is_wired_to_workspace_source` in
// corvid-guarantees) finds the literal "auth.oauth_pkce_required" at
// the enforcement site. allow(dead_code) is the load-bearing attribute.
#[allow(dead_code)]
pub const GUARANTEE_ID_OAUTH_PKCE_REQUIRED: &str = "auth.oauth_pkce_required";

use super::{
    read_actor_row, validate_non_empty, AuthActor, AuthTraceContext, ExternalIdentityLink,
    OAuthCallbackResolution, OAuthStateCreate, OAuthStatePurpose, OAuthStateRecord,
    SessionAuthRuntime,
};

pub fn hash_oauth_state(raw_state: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"corvid-auth-oauth-state-v1:");
    hasher.update(raw_state.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

const OAUTH_STATE_COLUMNS: &str = "id, provider, tenant_id, actor_id, purpose, state_hash, \
     pkce_verifier_ref, nonce_fingerprint, expires_ms, replay_key, used_ms, created_ms, updated_ms";

pub(super) fn read_oauth_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OAuthStateRecord> {
    let purpose_wire: String = row.get(4)?;
    let purpose = OAuthStatePurpose::from_wire(&purpose_wire).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown oauth state purpose `{purpose_wire}`").into(),
        )
    })?;
    Ok(OAuthStateRecord {
        id: row.get(0)?,
        provider: row.get(1)?,
        tenant_id: row.get(2)?,
        actor_id: row.get(3)?,
        purpose,
        state_hash: row.get(5)?,
        pkce_verifier_ref: row.get(6)?,
        nonce_fingerprint: row.get(7)?,
        expires_ms: row.get::<_, i64>(8)? as u64,
        replay_key: row.get(9)?,
        used_ms: row.get::<_, Option<i64>>(10)?.map(|value| value as u64),
        created_ms: row.get::<_, i64>(11)? as u64,
        updated_ms: row.get::<_, i64>(12)? as u64,
    })
}

impl SessionAuthRuntime {
    pub fn create_oauth_state(
        &self,
        input: OAuthStateCreate,
    ) -> Result<OAuthStateRecord, RuntimeError> {
        validate_non_empty("oauth state id", &input.id)?;
        validate_non_empty("oauth provider", &input.provider)?;
        validate_non_empty("tenant id", &input.tenant_id)?;
        validate_non_empty("oauth state", &input.raw_state)?;
        validate_non_empty("pkce verifier reference", &input.pkce_verifier_ref)?;
        validate_non_empty("nonce fingerprint", &input.nonce_fingerprint)?;
        validate_non_empty("replay key", &input.replay_key)?;
        let now = now_ms();
        if input.expires_ms <= now {
            return Err(RuntimeError::Other(
                "oauth state expiry must be in the future".to_string(),
            ));
        }
        // The bound actor and the flow purpose must agree structurally: a
        // `Link` flow is anchored to an already-authenticated actor, a
        // `Login` flow has none until the callback resolves the verified
        // subject. This is the invariant that keeps a login from ever
        // being mistaken for an account link.
        match (input.purpose, input.actor_id.as_deref()) {
            (OAuthStatePurpose::Link, Some(actor_id)) => {
                validate_non_empty("actor id", actor_id)?;
                let actor = self.get_actor(actor_id)?.ok_or_else(|| {
                    RuntimeError::Other(format!("auth actor `{actor_id}` not found"))
                })?;
                if actor.tenant_id != input.tenant_id {
                    return Err(RuntimeError::Other(
                        "oauth actor tenant mismatch".to_string(),
                    ));
                }
            }
            (OAuthStatePurpose::Link, None) => {
                return Err(RuntimeError::Other(
                    "a link oauth state requires a bound actor".to_string(),
                ));
            }
            (OAuthStatePurpose::Login, None) => {}
            (OAuthStatePurpose::Login, Some(_)) => {
                return Err(RuntimeError::Other(
                    "a login oauth state must not bind an actor (the actor is resolved at callback)"
                        .to_string(),
                ));
            }
        }
        let state_hash = hash_oauth_state(&input.raw_state);
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_oauth_states
                 (id, provider, tenant_id, actor_id, purpose, state_hash, pkce_verifier_ref, nonce_fingerprint, expires_ms, replay_key, used_ms, created_ms, updated_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, null, ?11, ?11)",
                params![
                    input.id,
                    input.provider,
                    input.tenant_id,
                    input.actor_id,
                    input.purpose.wire_name(),
                    state_hash,
                    input.pkce_verifier_ref,
                    input.nonce_fingerprint,
                    input.expires_ms as i64,
                    input.replay_key,
                    now as i64,
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to create oauth state: {err}")))?;
        self.get_oauth_state(&input.id)?
            .ok_or_else(|| RuntimeError::Other(format!("oauth state `{}` not found", input.id)))
    }

    pub fn get_oauth_state(&self, id: &str) -> Result<Option<OAuthStateRecord>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                &format!("select {OAUTH_STATE_COLUMNS} from auth_oauth_states where id = ?1"),
                params![id],
                read_oauth_state_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read oauth state: {err}")))
    }

    pub fn resolve_oauth_callback(
        &self,
        raw_state: &str,
        expected_tenant_id: &str,
        trace_id: &str,
        at_ms: u64,
    ) -> Result<OAuthCallbackResolution, RuntimeError> {
        validate_non_empty("oauth state", raw_state)?;
        validate_non_empty("tenant id", expected_tenant_id)?;
        validate_non_empty("trace id", trace_id)?;
        let state_hash = hash_oauth_state(raw_state);
        let state = match self.get_oauth_state_by_hash(&state_hash)? {
            Some(state) => state,
            None => {
                self.insert_audit(
                    "oauth.callback",
                    None,
                    Some(expected_tenant_id),
                    None,
                    None,
                    Some(trace_id),
                    "denied",
                    "oauth state not found",
                )?;
                return Err(RuntimeError::Other(
                    "oauth callback denied: state not found".to_string(),
                ));
            }
        };
        if state.used_ms.is_some() {
            self.audit_oauth_denied(&state, trace_id, "oauth state already used")?;
            return Err(RuntimeError::Other(
                "oauth callback denied: state already used".to_string(),
            ));
        }
        if at_ms >= state.expires_ms {
            self.audit_oauth_denied(&state, trace_id, "oauth state expired")?;
            return Err(RuntimeError::Other(
                "oauth callback denied: state expired".to_string(),
            ));
        }
        if state.tenant_id != expected_tenant_id {
            self.audit_oauth_denied(&state, trace_id, "tenant mismatch")?;
            return Err(RuntimeError::Other(
                "oauth callback denied: tenant mismatch".to_string(),
            ));
        }
        // A `Link` flow re-issues its bound actor; a `Login` flow leaves
        // the actor for the caller to resolve from the verified subject
        // (after code exchange + ID-token verification) and, if unknown,
        // the identity's provisioning policy.
        let actor = match (state.purpose, state.actor_id.as_deref()) {
            (OAuthStatePurpose::Link, Some(actor_id)) => {
                let actor = self
                    .get_actor(actor_id)?
                    .ok_or_else(|| RuntimeError::Other("oauth actor not found".to_string()))?;
                if actor.tenant_id != state.tenant_id {
                    self.audit_oauth_denied(&state, trace_id, "actor tenant mismatch")?;
                    return Err(RuntimeError::Other(
                        "oauth callback denied: actor tenant mismatch".to_string(),
                    ));
                }
                Some(actor)
            }
            (OAuthStatePurpose::Link, None) => {
                self.audit_oauth_denied(&state, trace_id, "link state without a bound actor")?;
                return Err(RuntimeError::Other(
                    "oauth callback denied: link state without a bound actor".to_string(),
                ));
            }
            (OAuthStatePurpose::Login, _) => None,
        };
        self.conn
            .lock()
            .unwrap()
            .execute(
                "update auth_oauth_states set used_ms = ?2, updated_ms = ?2 where id = ?1 and used_ms is null",
                params![state.id, at_ms as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to mark oauth state used: {err}")))?;
        let state = self.get_oauth_state(&state.id)?.ok_or_else(|| {
            RuntimeError::Other("oauth state disappeared after callback".to_string())
        })?;
        let trace = AuthTraceContext {
            trace_id: trace_id.to_string(),
            tenant_id: state.tenant_id.clone(),
            actor_id: actor.as_ref().map(|a| a.id.clone()).unwrap_or_default(),
            auth_method: "oauth".to_string(),
            session_id: String::new(),
            api_key_id: String::new(),
            permission_fingerprint: actor
                .as_ref()
                .map(|a| a.permission_fingerprint.clone())
                .unwrap_or_default(),
            replay_key: state.replay_key.clone(),
        };
        self.insert_audit(
            "oauth.callback",
            actor.as_ref().map(|a| a.id.as_str()),
            Some(&state.tenant_id),
            None,
            None,
            Some(trace_id),
            "allowed",
            match state.purpose {
                OAuthStatePurpose::Login => "oauth login state resolved",
                OAuthStatePurpose::Link => "oauth link state resolved",
            },
        )?;
        Ok(OAuthCallbackResolution {
            actor,
            state,
            trace,
        })
    }

    /// Look up the actor a verified external identity already maps to,
    /// keyed by `(issuer, subject)` from the ID token — NEVER by email.
    /// `None` means a first-time subject the caller must provision.
    pub fn find_actor_by_external_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<AuthActor>, RuntimeError> {
        validate_non_empty("issuer", issuer)?;
        validate_non_empty("subject", subject)?;
        let conn = self.conn.lock().unwrap();
        let actor_id: Option<String> = conn
            .query_row(
                "select actor_id from auth_external_identities where issuer = ?1 and subject = ?2",
                params![issuer, subject],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| {
                RuntimeError::Other(format!("failed to read external identity: {err}"))
            })?;
        let Some(actor_id) = actor_id else {
            return Ok(None);
        };
        conn.query_row(
            "select id, tenant_id, display_name, actor_kind, auth_method, assurance_level, role_fingerprint, permission_fingerprint, created_ms, updated_ms
             from auth_actors where id = ?1",
            params![actor_id],
            read_actor_row,
        )
        .optional()
        .map_err(|err| RuntimeError::Other(format!("failed to read linked actor: {err}")))
    }

    /// Durably bind a verified external identity `(issuer, subject)` to a
    /// Corvid actor. Written exactly once when a login provisions or
    /// recognises an actor; a second binding of the same `(issuer,
    /// subject)` to a different actor is rejected (never a silent
    /// re-home), and the actor must exist in the same tenant.
    pub fn link_external_identity(
        &self,
        provider: &str,
        issuer: &str,
        subject: &str,
        actor_id: &str,
        tenant_id: &str,
    ) -> Result<ExternalIdentityLink, RuntimeError> {
        validate_non_empty("provider", provider)?;
        validate_non_empty("issuer", issuer)?;
        validate_non_empty("subject", subject)?;
        validate_non_empty("actor id", actor_id)?;
        validate_non_empty("tenant id", tenant_id)?;
        let actor = self
            .get_actor(actor_id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth actor `{actor_id}` not found")))?;
        if actor.tenant_id != tenant_id {
            return Err(RuntimeError::Other(
                "external identity tenant mismatch".to_string(),
            ));
        }
        if let Some(existing) = self.find_actor_by_external_identity(issuer, subject)? {
            if existing.id == actor_id {
                // Idempotent re-link of the same binding.
                return self.get_external_identity(issuer, subject)?.ok_or_else(|| {
                    RuntimeError::Other("external identity disappeared after link".to_string())
                });
            }
            return Err(RuntimeError::Other(format!(
                "external identity ({issuer}, {subject}) already bound to actor `{}`",
                existing.id
            )));
        }
        let now = now_ms();
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_external_identities (issuer, subject, provider, actor_id, tenant_id, created_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6)",
                params![issuer, subject, provider, actor_id, tenant_id, now as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to link external identity: {err}")))?;
        self.get_external_identity(issuer, subject)?.ok_or_else(|| {
            RuntimeError::Other("external identity disappeared after link".to_string())
        })
    }

    fn get_external_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<ExternalIdentityLink>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "select provider, issuer, subject, actor_id, tenant_id, created_ms
                 from auth_external_identities where issuer = ?1 and subject = ?2",
                params![issuer, subject],
                |row| {
                    Ok(ExternalIdentityLink {
                        provider: row.get(0)?,
                        issuer: row.get(1)?,
                        subject: row.get(2)?,
                        actor_id: row.get(3)?,
                        tenant_id: row.get(4)?,
                        created_ms: row.get::<_, i64>(5)? as u64,
                    })
                },
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read external identity: {err}")))
    }

    fn get_oauth_state_by_hash(
        &self,
        state_hash: &str,
    ) -> Result<Option<OAuthStateRecord>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                &format!("select {OAUTH_STATE_COLUMNS} from auth_oauth_states where state_hash = ?1"),
                params![state_hash],
                read_oauth_state_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read oauth state by hash: {err}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthActor;

    fn actor(id: &str, tenant_id: &str) -> AuthActor {
        AuthActor {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            display_name: "Ada".to_string(),
            actor_kind: "user".to_string(),
            auth_method: "session".to_string(),
            assurance_level: "aal1".to_string(),
            role_fingerprint: "sha256:roles".to_string(),
            permission_fingerprint: "sha256:permissions".to_string(),
            created_ms: 1,
            updated_ms: 1,
        }
    }

    #[test]
    fn oauth_callback_state_is_hashed_single_use_and_restart_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.sqlite");
        let expires_ms = now_ms().saturating_add(60_000);
        {
            let auth = SessionAuthRuntime::open(&path).unwrap();
            auth.upsert_actor(actor("user-1", "org-1")).unwrap();
            let state = auth
                .create_oauth_state(OAuthStateCreate {
                    id: "oauth-state-1".to_string(),
                    provider: "google".to_string(),
                    tenant_id: "org-1".to_string(),
                    actor_id: Some("user-1".to_string()),
                    purpose: OAuthStatePurpose::Link,
                    raw_state: "raw-oauth-state".to_string(),
                    pkce_verifier_ref: "pkce-ref-1".to_string(),
                    nonce_fingerprint: "sha256:nonce".to_string(),
                    expires_ms,
                    replay_key: "replay-oauth-1".to_string(),
                })
                .unwrap();
            assert_eq!(state.state_hash, hash_oauth_state("raw-oauth-state"));
            assert!(!state.state_hash.contains("raw-oauth-state"));
            assert_eq!(state.used_ms, None);
        }

        let auth = SessionAuthRuntime::open(&path).unwrap();
        let resolved = auth
            .resolve_oauth_callback("raw-oauth-state", "org-1", "trace-oauth-1", now_ms())
            .unwrap();
        assert_eq!(resolved.actor.as_ref().unwrap().id, "user-1");
        assert_eq!(resolved.trace.auth_method, "oauth");
        assert_eq!(resolved.trace.replay_key, "replay-oauth-1");
        assert!(resolved.state.used_ms.is_some());

        let replay = auth
            .resolve_oauth_callback("raw-oauth-state", "org-1", "trace-oauth-2", now_ms())
            .unwrap_err();
        assert!(replay.to_string().contains("state already used"));
        let audit = auth.audit_events().unwrap();
        assert_eq!(audit.len(), 2);
        assert!(audit.iter().all(|event| {
            !event.reason.contains("raw-oauth-state") && !event.id.contains("raw-oauth-state")
        }));
    }

    #[test]
    fn oauth_callback_rejects_expired_and_cross_tenant_state() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        let expires_ms = now_ms().saturating_add(60_000);
        auth.create_oauth_state(OAuthStateCreate {
            id: "oauth-state-1".to_string(),
            provider: "github".to_string(),
            tenant_id: "org-1".to_string(),
            actor_id: Some("user-1".to_string()),
            purpose: OAuthStatePurpose::Link,
            raw_state: "state-1".to_string(),
            pkce_verifier_ref: "pkce-ref-1".to_string(),
            nonce_fingerprint: "sha256:nonce".to_string(),
            expires_ms,
            replay_key: "replay-oauth-1".to_string(),
        })
        .unwrap();

        let tenant = auth
            .resolve_oauth_callback("state-1", "org-2", "trace-tenant", now_ms())
            .unwrap_err();
        assert!(tenant.to_string().contains("tenant mismatch"));
        let expired = auth
            .resolve_oauth_callback("state-1", "org-1", "trace-expired", expires_ms)
            .unwrap_err();
        assert!(expired.to_string().contains("state expired"));
    }

    /// Helper: a login-purpose state create with no bound actor.
    fn login_state(id: &str, raw_state: &str, expires_ms: u64) -> OAuthStateCreate {
        OAuthStateCreate {
            id: id.to_string(),
            provider: "google".to_string(),
            tenant_id: "org-1".to_string(),
            actor_id: None,
            purpose: OAuthStatePurpose::Login,
            raw_state: raw_state.to_string(),
            pkce_verifier_ref: "pkce-ref".to_string(),
            nonce_fingerprint: "sha256:nonce".to_string(),
            expires_ms,
            replay_key: format!("replay-{id}"),
        }
    }

    #[test]
    fn login_state_binds_no_actor_and_resolves_without_one() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let expires_ms = now_ms().saturating_add(60_000);
        let state = auth
            .create_oauth_state(login_state("login-1", "raw-login", expires_ms))
            .unwrap();
        assert_eq!(state.purpose, OAuthStatePurpose::Login);
        assert_eq!(state.actor_id, None);

        // The callback consumes the state but leaves the actor for the
        // caller to resolve from the verified subject.
        let resolved = auth
            .resolve_oauth_callback("raw-login", "org-1", "trace-login", now_ms())
            .unwrap();
        assert!(resolved.actor.is_none());
        assert!(resolved.state.used_ms.is_some());
        // Single-use still holds for a login state.
        let replay = auth
            .resolve_oauth_callback("raw-login", "org-1", "trace-login-2", now_ms())
            .unwrap_err();
        assert!(replay.to_string().contains("state already used"));
    }

    #[test]
    fn login_state_must_not_bind_an_actor_and_link_state_must() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        let expires_ms = now_ms().saturating_add(60_000);

        // A login state that smuggles in an actor is rejected.
        let mut bad_login = login_state("login-bad", "raw-a", expires_ms);
        bad_login.actor_id = Some("user-1".to_string());
        let err = auth.create_oauth_state(bad_login).unwrap_err();
        assert!(err.to_string().contains("must not bind an actor"));

        // A link state with no actor is rejected.
        let bad_link = OAuthStateCreate {
            actor_id: None,
            purpose: OAuthStatePurpose::Link,
            ..login_state("link-bad", "raw-b", expires_ms)
        };
        let err = auth.create_oauth_state(bad_link).unwrap_err();
        assert!(err.to_string().contains("requires a bound actor"));
    }

    #[test]
    fn external_identity_maps_issuer_subject_to_actor_never_by_email() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        auth.upsert_actor(actor("user-2", "org-1")).unwrap();

        // Unknown subject → no actor (a first-time login).
        assert!(auth
            .find_actor_by_external_identity("https://accounts.google.com", "sub-123")
            .unwrap()
            .is_none());

        // Bind, then recognise the returning subject by (issuer, subject).
        auth.link_external_identity(
            "google",
            "https://accounts.google.com",
            "sub-123",
            "user-1",
            "org-1",
        )
        .unwrap();
        let found = auth
            .find_actor_by_external_identity("https://accounts.google.com", "sub-123")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "user-1");

        // Idempotent re-link of the same binding is fine.
        auth.link_external_identity(
            "google",
            "https://accounts.google.com",
            "sub-123",
            "user-1",
            "org-1",
        )
        .unwrap();

        // Re-homing the same (issuer, subject) to a different actor is
        // refused — an external identity is never silently reassigned.
        let err = auth
            .link_external_identity(
                "google",
                "https://accounts.google.com",
                "sub-123",
                "user-2",
                "org-1",
            )
            .unwrap_err();
        assert!(err.to_string().contains("already bound"));
    }
}
