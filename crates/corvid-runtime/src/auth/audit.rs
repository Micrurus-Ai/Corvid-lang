//! Auth audit-event runtime methods + the supporting helpers.
//!
//! `audit_events` is the operator-facing listing endpoint;
//! `insert_audit` is the cross-domain write the session /
//! api_key / oauth resolve paths share, and the three
//! per-domain `audit_*_denied` helpers are the failure-path
//! shorthand each domain calls when it rejects a request.
//!
//! `stable_suffix` produces a 16-hex digest from the event kind plus
//! the session and trace ids the event pertains to; `insert_audit`
//! mints `auth_audit_<ts>_<digest>_<random>` ids. The random part
//! keeps two events with the same kind, session and trace in the same
//! millisecond apart: without it the second insert hit the primary key
//! and failed the auth operation that was being recorded, so a store
//! could refuse a valid session for being checked twice at once. Events
//! list in the order they were written (`created_ms`, then `rowid`).
//! `read_audit_row` is the SQLite row → `AuthAuditEvent` deserializer.

use rusqlite::params;
use sha2::{Digest, Sha256};

use crate::errors::RuntimeError;
use crate::tracing::now_ms;

use super::{
    ApiKeyRecord, AuthAuditEvent, OAuthStateRecord, SessionAuthRuntime, SessionRecord,
};

pub(super) fn stable_suffix(
    event_kind: &str,
    session_id: Option<&str>,
    trace_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_kind.as_bytes());
    hasher.update(b":");
    hasher.update(session_id.unwrap_or("").as_bytes());
    hasher.update(b":");
    hasher.update(trace_id.unwrap_or("").as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..16].to_string()
}

pub(super) fn read_audit_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuthAuditEvent> {
    Ok(AuthAuditEvent {
        id: row.get(0)?,
        event_kind: row.get(1)?,
        actor_id: row.get(2)?,
        tenant_id: row.get(3)?,
        session_id: row.get(4)?,
        api_key_id: row.get(5)?,
        trace_id: row.get(6)?,
        status: row.get(7)?,
        reason: row.get(8)?,
        created_ms: row.get::<_, i64>(9)? as u64,
    })
}

impl SessionAuthRuntime {
    pub fn audit_events(&self) -> Result<Vec<AuthAuditEvent>, RuntimeError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "select id, event_kind, actor_id, tenant_id, session_id, api_key_id, trace_id, status, reason, created_ms
                 from auth_audit_events order by created_ms, rowid",
            )
            .map_err(|err| RuntimeError::Other(format!("failed to prepare auth audit list: {err}")))?;
        let rows = stmt
            .query_map([], read_audit_row)
            .map_err(|err| RuntimeError::Other(format!("failed to list auth audit events: {err}")))?;
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|err| {
                RuntimeError::Other(format!("failed to decode auth audit event: {err}"))
            })?);
        }
        Ok(events)
    }

    pub(super) fn audit_session_denied(
        &self,
        session: &SessionRecord,
        trace_id: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.insert_audit(
            "session.resolve",
            Some(&session.actor_id),
            Some(&session.tenant_id),
            Some(&session.id),
            None,
            Some(trace_id),
            "denied",
            reason,
        )
    }

    pub(super) fn audit_api_key_denied(
        &self,
        key: &ApiKeyRecord,
        trace_id: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.insert_audit(
            "api_key.resolve",
            Some(&key.service_actor_id),
            Some(&key.tenant_id),
            None,
            Some(&key.id),
            Some(trace_id),
            "denied",
            reason,
        )
    }

    pub(super) fn audit_oauth_denied(
        &self,
        state: &OAuthStateRecord,
        trace_id: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.insert_audit(
            "oauth.callback",
            state.actor_id.as_deref(),
            Some(&state.tenant_id),
            None,
            None,
            Some(trace_id),
            "denied",
            reason,
        )
    }

    pub(super) fn insert_audit(
        &self,
        event_kind: &str,
        actor_id: Option<&str>,
        tenant_id: Option<&str>,
        session_id: Option<&str>,
        api_key_id: Option<&str>,
        trace_id: Option<&str>,
        status: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        let now = now_ms();
        let unique = {
            use rand_core::RngCore;
            rand_core::OsRng.next_u64()
        };
        let id = format!("auth_audit_{now}_{}_{unique:016x}", stable_suffix(event_kind, session_id, trace_id));
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_audit_events
                 (id, event_kind, actor_id, tenant_id, session_id, api_key_id, trace_id, status, reason, created_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    event_kind,
                    actor_id,
                    tenant_id,
                    session_id,
                    api_key_id,
                    trace_id,
                    status,
                    reason,
                    now as i64,
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to insert auth audit event: {err}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::auth::{AuthActor, SessionAuthRuntime, SessionCreate};
    use crate::tracing::now_ms;

    /// A session checked twice under one trace id in one millisecond
    /// is two audit events, and both checks succeed. Before the random
    /// part, the second event's id repeated the first and the second
    /// check failed on the audit table's primary key.
    #[test]
    fn the_same_check_twice_in_one_millisecond_records_two_events() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(AuthActor {
            id: "user-1".into(),
            tenant_id: "org-1".into(),
            display_name: "Ada".into(),
            actor_kind: "user".into(),
            auth_method: "session".into(),
            assurance_level: "aal1".into(),
            role_fingerprint: "sha256:roles".into(),
            permission_fingerprint: "sha256:permissions".into(),
            created_ms: 1,
            updated_ms: 1,
        })
        .unwrap();
        let now = now_ms();
        auth.create_session(SessionCreate {
            id: "sess-1".into(),
            actor_id: "user-1".into(),
            tenant_id: "org-1".into(),
            raw_token: "token-1".into(),
            issued_ms: now - 1,
            expires_ms: now + 60_000,
            csrf_binding_id: "csrf".into(),
        })
        .unwrap();
        // Enough checks that several land in one millisecond.
        for _ in 0..200 {
            auth.resolve_session_cookie("token-1", "one-trace", now_ms()).expect("every check succeeds");
        }
        let events = auth.audit_events().unwrap();
        assert_eq!(events.len(), 200);
        let mut ids: Vec<_> = events.iter().map(|e| e.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 200, "every event has its own id");
    }
}
