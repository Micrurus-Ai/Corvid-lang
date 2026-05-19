//! Session domain — token hashing, row decoding, and the
//! `SessionAuthRuntime` impl block carrying create / get /
//! resolve / rotate / revoke for `auth_sessions`.
//!
//! Session tokens use a SHA-256 prefix-keyed hash rather than the
//! Argon2id used for API keys. Sessions live minutes-to-hours and
//! are validated on every request, so the cheaper hash keeps the
//! per-resolve latency low; the brute-force window is bounded by
//! the session's short TTL anyway.
//!
//! `resolve_session` is the per-request validation: token →
//! session record → freshness / tenancy / actor checks → audit
//! `allowed`/`denied` → `SessionResolution`. `get_session_by_hash`
//! is the private lookup `resolve_session` builds on; mod.rs no
//! longer touches it directly.

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::errors::RuntimeError;
use crate::tracing::now_ms;

use super::{
    validate_non_empty, AuthActor, AuthTraceContext, SessionAuthRuntime, SessionCreate,
    SessionRecord, SessionResolution,
};

/// Named privilege-change events that mandate a session rotation.
/// The kind is recorded in the auth-audit trail so a reviewer can
/// see *why* the rotation fired, not just *that* it fired.
///
/// Pinned to a small, explicit enum (no free-form strings) because
/// the registry row `auth.session_rotation_on_privilege_change`
/// guarantees the rotation happens on the named events; the closed
/// set keeps the contract auditable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeChangeReason {
    /// The actor's role was upgraded (e.g. Member → Admin).
    RoleUpgrade,
    /// The actor's password was changed (initiated by the actor or
    /// reset by an admin).
    PasswordChange,
    /// MFA was enrolled, re-enrolled, or strengthened.
    MfaEnrolled,
    /// An admin forcibly elevated the actor's privileges out-of-band.
    AdminElevation,
}

impl PrivilegeChangeReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoleUpgrade => "role_upgrade",
            Self::PasswordChange => "password_change",
            Self::MfaEnrolled => "mfa_enrolled",
            Self::AdminElevation => "admin_elevation",
        }
    }
}

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-coverage
/// sentinel. Names the registry id whose runtime enforcement lives
/// in `rotate_session_on_privilege_change` below: the session id
/// rotates + an audit row records the privilege-change reason.
#[allow(dead_code)]
pub const GUARANTEE_ID_SESSION_ROTATION_ON_PRIVILEGE_CHANGE: &str =
    "auth.session_rotation_on_privilege_change";

pub fn hash_session_secret(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"corvid-auth-session-v1:");
    hasher.update(raw_token.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

pub(super) fn read_actor_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuthActor> {
    Ok(AuthActor {
        id: row.get(0)?,
        tenant_id: row.get(1)?,
        display_name: row.get(2)?,
        actor_kind: row.get(3)?,
        auth_method: row.get(4)?,
        assurance_level: row.get(5)?,
        role_fingerprint: row.get(6)?,
        permission_fingerprint: row.get(7)?,
        created_ms: row.get::<_, i64>(8)? as u64,
        updated_ms: row.get::<_, i64>(9)? as u64,
    })
}

pub(super) fn read_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    Ok(SessionRecord {
        id: row.get(0)?,
        actor_id: row.get(1)?,
        tenant_id: row.get(2)?,
        token_hash: row.get(3)?,
        issued_ms: row.get::<_, i64>(4)? as u64,
        expires_ms: row.get::<_, i64>(5)? as u64,
        rotation_counter: row.get::<_, i64>(6)? as u64,
        csrf_binding_id: row.get(7)?,
        revoked_ms: row.get::<_, Option<i64>>(8)?.map(|value| value as u64),
        created_ms: row.get::<_, i64>(9)? as u64,
        updated_ms: row.get::<_, i64>(10)? as u64,
    })
}

impl SessionAuthRuntime {
    pub fn create_session(&self, input: SessionCreate) -> Result<SessionRecord, RuntimeError> {
        validate_non_empty("session id", &input.id)?;
        validate_non_empty("actor id", &input.actor_id)?;
        validate_non_empty("tenant id", &input.tenant_id)?;
        validate_non_empty("session token", &input.raw_token)?;
        if input.expires_ms <= input.issued_ms {
            return Err(RuntimeError::Other(
                "session expiry must be after issue time".to_string(),
            ));
        }
        let actor = self.get_actor(&input.actor_id)?.ok_or_else(|| {
            RuntimeError::Other(format!("auth actor `{}` not found", input.actor_id))
        })?;
        if actor.tenant_id != input.tenant_id {
            return Err(RuntimeError::Other(
                "session actor tenant mismatch".to_string(),
            ));
        }
        let token_hash = hash_session_secret(&input.raw_token);
        let now = now_ms();
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_sessions
                 (id, actor_id, tenant_id, token_hash, issued_ms, expires_ms, rotation_counter, csrf_binding_id, revoked_ms, created_ms, updated_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, null, ?8, ?8)",
                params![
                    input.id,
                    input.actor_id,
                    input.tenant_id,
                    token_hash,
                    input.issued_ms as i64,
                    input.expires_ms as i64,
                    input.csrf_binding_id,
                    now as i64,
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to create session: {err}")))?;
        self.get_session(&input.id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth session `{}` not found", input.id)))
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRecord>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "select id, actor_id, tenant_id, token_hash, issued_ms, expires_ms, rotation_counter, csrf_binding_id, revoked_ms, created_ms, updated_ms
                 from auth_sessions where id = ?1",
                params![id],
                read_session_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read session: {err}")))
    }

    pub fn resolve_session(
        &self,
        raw_token: &str,
        expected_tenant_id: &str,
        trace_id: &str,
        replay_key: &str,
        at_ms: u64,
    ) -> Result<SessionResolution, RuntimeError> {
        validate_non_empty("session token", raw_token)?;
        validate_non_empty("tenant id", expected_tenant_id)?;
        validate_non_empty("trace id", trace_id)?;
        let token_hash = hash_session_secret(raw_token);
        let session = match self.get_session_by_hash(&token_hash)? {
            Some(session) => session,
            None => {
                self.insert_audit(
                    "session.resolve",
                    None,
                    Some(expected_tenant_id),
                    None,
                    None,
                    Some(trace_id),
                    "denied",
                    "session token not found",
                )?;
                return Err(RuntimeError::Other(
                    "session resolve denied: token not found".to_string(),
                ));
            }
        };
        if session.revoked_ms.is_some() {
            self.audit_session_denied(&session, trace_id, "session revoked")?;
            return Err(RuntimeError::Other(
                "session resolve denied: session revoked".to_string(),
            ));
        }
        if at_ms >= session.expires_ms {
            self.audit_session_denied(&session, trace_id, "session expired")?;
            return Err(RuntimeError::Other(
                "session resolve denied: session expired".to_string(),
            ));
        }
        if session.tenant_id != expected_tenant_id {
            self.audit_session_denied(&session, trace_id, "tenant mismatch")?;
            return Err(RuntimeError::Other(
                "session resolve denied: tenant mismatch".to_string(),
            ));
        }
        let actor = self
            .get_actor(&session.actor_id)?
            .ok_or_else(|| RuntimeError::Other("session actor not found".to_string()))?;
        if actor.tenant_id != session.tenant_id {
            self.audit_session_denied(&session, trace_id, "actor tenant mismatch")?;
            return Err(RuntimeError::Other(
                "session resolve denied: actor tenant mismatch".to_string(),
            ));
        }
        let trace = AuthTraceContext {
            trace_id: trace_id.to_string(),
            tenant_id: session.tenant_id.clone(),
            actor_id: actor.id.clone(),
            auth_method: actor.auth_method.clone(),
            session_id: session.id.clone(),
            api_key_id: String::new(),
            permission_fingerprint: actor.permission_fingerprint.clone(),
            replay_key: replay_key.to_string(),
        };
        self.insert_audit(
            "session.resolve",
            Some(&actor.id),
            Some(&session.tenant_id),
            Some(&session.id),
            None,
            Some(trace_id),
            "allowed",
            "session resolved",
        )?;
        Ok(SessionResolution {
            actor,
            session,
            trace,
        })
    }

    pub fn rotate_session(
        &self,
        session_id: &str,
        new_raw_token: &str,
        new_expires_ms: u64,
    ) -> Result<SessionRecord, RuntimeError> {
        validate_non_empty("session id", session_id)?;
        validate_non_empty("session token", new_raw_token)?;
        let session = self
            .get_session(session_id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth session `{session_id}` not found")))?;
        if new_expires_ms <= session.issued_ms {
            return Err(RuntimeError::Other(
                "rotated session expiry must be after issue time".to_string(),
            ));
        }
        let now = now_ms();
        let token_hash = hash_session_secret(new_raw_token);
        self.conn
            .lock()
            .unwrap()
            .execute(
                "update auth_sessions
                 set token_hash = ?2, expires_ms = ?3, rotation_counter = rotation_counter + 1, revoked_ms = null, updated_ms = ?4
                 where id = ?1",
                params![session_id, token_hash, new_expires_ms as i64, now as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to rotate session: {err}")))?;
        self.get_session(session_id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth session `{session_id}` not found")))
    }

    /// Privilege-change rotation hook: rotate the session id +
    /// record an audit row naming the privilege-change reason.
    ///
    /// Wraps `rotate_session` so the post-rotation invariants
    /// (old token rejected, rotation_counter bumped, revocation
    /// cleared) hold here too. The audit row's `event_kind` is
    /// `session.rotate_on_privilege_change` and the `reason` is
    /// the typed enum's stable string id — auditable evidence
    /// that the rotation was tied to a named privilege event,
    /// not an arbitrary client refresh.
    ///
    /// Catches the `session-fixation` named threat from the Phase
    /// 39 adversarial corpus: a pre-elevation session token
    /// cannot ride forward into the post-elevation privilege.
    pub fn rotate_session_on_privilege_change(
        &self,
        session_id: &str,
        reason: PrivilegeChangeReason,
        new_raw_token: &str,
        new_expires_ms: u64,
        trace_id: &str,
    ) -> Result<SessionRecord, RuntimeError> {
        validate_non_empty("session id", session_id)?;
        validate_non_empty("session token", new_raw_token)?;
        validate_non_empty("trace id", trace_id)?;
        let rotated = self.rotate_session(session_id, new_raw_token, new_expires_ms)?;
        self.insert_audit(
            "session.rotate_on_privilege_change",
            Some(&rotated.actor_id),
            Some(&rotated.tenant_id),
            Some(&rotated.id),
            None,
            Some(trace_id),
            "ok",
            reason.as_str(),
        )?;
        Ok(rotated)
    }

    pub fn revoke_session(
        &self,
        session_id: &str,
        at_ms: u64,
    ) -> Result<SessionRecord, RuntimeError> {
        validate_non_empty("session id", session_id)?;
        self.conn
            .lock()
            .unwrap()
            .execute(
                "update auth_sessions set revoked_ms = ?2, updated_ms = ?2 where id = ?1",
                params![session_id, at_ms as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to revoke session: {err}")))?;
        self.get_session(session_id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth session `{session_id}` not found")))
    }

    pub(super) fn get_session_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<SessionRecord>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "select id, actor_id, tenant_id, token_hash, issued_ms, expires_ms, rotation_counter, csrf_binding_id, revoked_ms, created_ms, updated_ms
                 from auth_sessions where token_hash = ?1",
                params![token_hash],
                read_session_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read session by token: {err}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthAuditEvent;

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
    fn session_runtime_resolves_actor_context_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.sqlite");
        {
            let auth = SessionAuthRuntime::open(&path).unwrap();
            auth.upsert_actor(actor("user-1", "org-1")).unwrap();
            let session = auth
                .create_session(SessionCreate {
                    id: "sess-1".to_string(),
                    actor_id: "user-1".to_string(),
                    tenant_id: "org-1".to_string(),
                    raw_token: "raw-session-secret".to_string(),
                    issued_ms: 1_000,
                    expires_ms: 9_000,
                    csrf_binding_id: "csrf-1".to_string(),
                })
                .unwrap();
            assert_eq!(
                session.token_hash,
                hash_session_secret("raw-session-secret")
            );
            assert!(!session.token_hash.contains("raw-session-secret"));
        }

        let auth = SessionAuthRuntime::open(&path).unwrap();
        let resolved = auth
            .resolve_session(
                "raw-session-secret",
                "org-1",
                "trace-1",
                "replay-auth-1",
                5_000,
            )
            .unwrap();
        assert_eq!(resolved.actor.id, "user-1");
        assert_eq!(resolved.trace.tenant_id, "org-1");
        assert_eq!(resolved.trace.actor_id, "user-1");
        assert_eq!(resolved.trace.session_id, "sess-1");
        assert_eq!(resolved.trace.replay_key, "replay-auth-1");
        let audit = auth.audit_events().unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].status, "allowed");
        assert_eq!(audit[0].session_id.as_deref(), Some("sess-1"));
    }

    #[test]
    fn session_runtime_rejects_expired_revoked_and_cross_tenant_sessions() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        auth.create_session(SessionCreate {
            id: "sess-1".to_string(),
            actor_id: "user-1".to_string(),
            tenant_id: "org-1".to_string(),
            raw_token: "secret-1".to_string(),
            issued_ms: 1_000,
            expires_ms: 2_000,
            csrf_binding_id: "csrf-1".to_string(),
        })
        .unwrap();

        let expired = auth
            .resolve_session(
                "secret-1",
                "org-1",
                "trace-expired",
                "replay-expired",
                2_000,
            )
            .unwrap_err();
        assert!(expired.to_string().contains("session expired"));
        let tenant = auth
            .resolve_session("secret-1", "org-2", "trace-tenant", "replay-tenant", 1_500)
            .unwrap_err();
        assert!(tenant.to_string().contains("tenant mismatch"));
        auth.revoke_session("sess-1", 1_600).unwrap();
        let revoked = auth
            .resolve_session(
                "secret-1",
                "org-1",
                "trace-revoked",
                "replay-revoked",
                1_700,
            )
            .unwrap_err();
        assert!(revoked.to_string().contains("session revoked"));

        let audit = auth.audit_events().unwrap();
        assert_eq!(audit.len(), 3);
        assert!(audit.iter().all(|event| event.status == "denied"));
        assert!(audit
            .iter()
            .all(|event| { !event.reason.contains("secret-1") && !event.id.contains("secret-1") }));
    }

    #[test]
    fn session_rotation_invalidates_old_token_and_preserves_rotation_counter() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        auth.create_session(SessionCreate {
            id: "sess-1".to_string(),
            actor_id: "user-1".to_string(),
            tenant_id: "org-1".to_string(),
            raw_token: "old-secret".to_string(),
            issued_ms: 1_000,
            expires_ms: 5_000,
            csrf_binding_id: "csrf-1".to_string(),
        })
        .unwrap();

        let rotated = auth.rotate_session("sess-1", "new-secret", 8_000).unwrap();
        assert_eq!(rotated.rotation_counter, 1);
        assert_eq!(rotated.token_hash, hash_session_secret("new-secret"));
        assert!(auth
            .resolve_session("old-secret", "org-1", "trace-old", "replay-old", 2_000)
            .is_err());
        let resolved = auth
            .resolve_session("new-secret", "org-1", "trace-new", "replay-new", 2_000)
            .unwrap();
        assert_eq!(resolved.session.rotation_counter, 1);
    }

    /// Slice 35V2-P39-D-LR (positive, session-fixation threat):
    /// rotating on a typed privilege-change event invalidates the
    /// pre-elevation token AND records the typed reason in the
    /// audit row. This is the named-threat test for the
    /// `session-fixation` adversarial corpus entry — an attacker
    /// who held the pre-elevation cookie cannot ride it forward
    /// into the post-elevation privilege.
    #[test]
    fn session_rotation_on_privilege_change_rejects_pre_elevation_session_fixation_attempt() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        auth.create_session(SessionCreate {
            id: "sess-1".to_string(),
            actor_id: "user-1".to_string(),
            tenant_id: "org-1".to_string(),
            raw_token: "pre-elevation-cookie".to_string(),
            issued_ms: 1_000,
            expires_ms: 5_000,
            csrf_binding_id: "csrf-1".to_string(),
        })
        .unwrap();

        // Attacker captures the pre-elevation cookie; before
        // rotation it would resolve successfully.
        let pre = auth
            .resolve_session(
                "pre-elevation-cookie",
                "org-1",
                "trace-pre",
                "replay-pre",
                1_500,
            )
            .unwrap();
        assert_eq!(pre.session.rotation_counter, 0);

        // Privilege event: user's role is upgraded.
        let rotated = auth
            .rotate_session_on_privilege_change(
                "sess-1",
                PrivilegeChangeReason::RoleUpgrade,
                "post-elevation-cookie",
                8_000,
                "trace-elevation",
            )
            .unwrap();
        assert_eq!(rotated.rotation_counter, 1);

        // Adversarial: attacker replays the captured pre-elevation
        // cookie post-rotation — must be rejected.
        let replay_err = auth
            .resolve_session(
                "pre-elevation-cookie",
                "org-1",
                "trace-replay",
                "replay-replay",
                2_000,
            )
            .unwrap_err()
            .to_string();
        assert!(
            replay_err.contains("session not found") || replay_err.contains("not found"),
            "expected session-not-found rejection, got: {replay_err}"
        );

        // The new cookie resolves cleanly.
        let post = auth
            .resolve_session(
                "post-elevation-cookie",
                "org-1",
                "trace-post",
                "replay-post",
                2_000,
            )
            .unwrap();
        assert_eq!(post.session.rotation_counter, 1);

        // The privilege-change audit row records the typed reason.
        let audit = auth.audit_events().unwrap();
        let rotate_events: Vec<&AuthAuditEvent> = audit
            .iter()
            .filter(|e| e.event_kind == "session.rotate_on_privilege_change")
            .collect();
        assert_eq!(rotate_events.len(), 1);
        assert_eq!(rotate_events[0].status, "ok");
        assert_eq!(rotate_events[0].reason, "role_upgrade");
        assert_eq!(rotate_events[0].session_id.as_deref(), Some("sess-1"));
        // Adversarial-defence invariant: the raw cookies must NEVER
        // appear in the audit row or its id.
        assert!(!rotate_events[0]
            .reason
            .contains("pre-elevation-cookie"));
        assert!(!rotate_events[0].id.contains("post-elevation-cookie"));
    }

    /// Slice 35V2-P39-D-LR (adversarial): a privilege-change
    /// rotation that omits the trace id is refused — the audit
    /// row's grounding requires a trace context, and a silent
    /// rotation without one would defeat the audit-trail
    /// guarantee.
    #[test]
    fn session_rotation_on_privilege_change_refuses_empty_trace_id() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("user-1", "org-1")).unwrap();
        auth.create_session(SessionCreate {
            id: "sess-1".to_string(),
            actor_id: "user-1".to_string(),
            tenant_id: "org-1".to_string(),
            raw_token: "old-secret".to_string(),
            issued_ms: 1_000,
            expires_ms: 5_000,
            csrf_binding_id: "csrf-1".to_string(),
        })
        .unwrap();
        let err = auth
            .rotate_session_on_privilege_change(
                "sess-1",
                PrivilegeChangeReason::PasswordChange,
                "new-secret",
                8_000,
                "",
            )
            .unwrap_err();
        assert!(err.to_string().contains("trace id"));
        // The old session must still resolve — the failed rotation
        // is a no-op, not a half-applied state.
        let pre = auth
            .resolve_session("old-secret", "org-1", "trace-still-ok", "replay-x", 1_500)
            .unwrap();
        assert_eq!(pre.session.rotation_counter, 0);
    }
}
