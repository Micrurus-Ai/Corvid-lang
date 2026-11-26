//! A durable authority revision brackets validation and guards live delivery.
use super::{hash_session_secret, SessionAuthRuntime};
use crate::errors::RuntimeError;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use std::sync::Arc;

/// The authority under which a live connection last resolved its cookie.
pub struct SessionObservation {
    store: Arc<SessionAuthRuntime>,
    clock: fn() -> u64,
    token_hash: String,
    authority: (String, i64),
    expires_ms: u64,
}

fn storage_error(error: rusqlite::Error) -> RuntimeError {
    RuntimeError::Other(format!("session observation failed: {error}"))
}

pub(super) fn initialize(conn: &Connection) -> Result<(), RuntimeError> {
    let tx = conn.unchecked_transaction().map_err(storage_error)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_authority_revisions (
        actor_id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(typeof(revision) = 'integer'))",
    )
    .map_err(storage_error)?;
    for (table, actor) in [
        ("auth_sessions", "actor_id"),
        ("auth_actor_roles", "actor_id"),
        ("auth_actors", "id"),
    ] {
        for (event, row) in [("insert", "new"), ("delete", "old"), ("update", "old")] {
            tx.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS observe_{table}_{event}
                AFTER {event} ON {table} BEGIN
                INSERT INTO auth_authority_revisions VALUES ({row}.{actor}, 1)
                ON CONFLICT(actor_id) DO UPDATE SET revision = revision + 1;
                END"
            ))
            .map_err(storage_error)?;
        }
    }
    tx.commit().map_err(storage_error)
}

fn revision(conn: &Connection, token_hash: &str) -> Result<Option<(String, i64)>, RuntimeError> {
    conn.query_row(
        "SELECT s.actor_id, coalesce(r.revision, 0) FROM auth_sessions s
        LEFT JOIN auth_authority_revisions r ON r.actor_id = s.actor_id
        WHERE s.token_hash = ?1",
        [token_hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(storage_error)
}

impl SessionAuthRuntime {
    /// Validate a cookie and capture the authority revision used by that validation.
    pub fn observe_session_cookie(
        self: &Arc<Self>,
        token: &str,
        trace: &str,
        clock: fn() -> u64,
    ) -> Result<SessionObservation, RuntimeError> {
        let token_hash = hash_session_secret(token);
        let before = revision(&self.conn.lock().unwrap(), &token_hash)?;
        let resolved = self.resolve_session_cookie(token, trace, clock())?;
        let observation = SessionObservation {
            store: self.clone(),
            clock,
            token_hash,
            authority: before
                .ok_or_else(|| RuntimeError::Other("session changed during validation".into()))?,
            expires_ms: resolved.session.expires_ms,
        };
        if !observation.is_current()? {
            return Err(RuntimeError::Other(
                "session changed during validation".into(),
            ));
        }
        Ok(observation)
    }
}

impl SessionObservation {
    /// Milliseconds until this observation reaches the session's expiry.
    pub fn remaining_ms(&self) -> u64 {
        self.expires_ms.saturating_sub((self.clock)())
    }

    fn current_in(&self, conn: &Connection) -> Result<bool, RuntimeError> {
        Ok(
            revision(conn, &self.token_hash)?.as_ref() == Some(&self.authority)
                && self.remaining_ms() > 0,
        )
    }

    /// Check the observed revision and expiry, including changes from other processes.
    pub fn is_current(&self) -> Result<bool, RuntimeError> {
        self.current_in(&self.store.conn.lock().unwrap())
    }

    /// Run a synchronous enqueue only while this session still has the observed
    /// authority. The callback must neither block on I/O nor reenter the auth store.
    pub fn while_current<T>(&self, enqueue: impl FnOnce() -> T) -> Result<Option<T>, RuntimeError> {
        let mut conn = self.store.conn.lock().unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        Ok(self.current_in(&tx)?.then(enqueue))
    }
}
