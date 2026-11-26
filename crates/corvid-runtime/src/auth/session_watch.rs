//! Watching a session for the changes that end or alter it (B14 RT-3).
//!
//! A live subscription stays open long after the request that started
//! it, so it must hear when its session stops being valid or its
//! authority changes. [`SessionAuthRuntime::observe_session`] validates
//! a session token and registers a watcher in one step; from then on
//! the session's state is published to every watcher:
//!
//! - `Invalid` when the session is revoked (logout), its actor's
//!   sessions are revoked (a role is taken away), it is rotated (the old
//!   token no longer works), or it expires;
//! - `Active` again when its actor's authority grows or changes without
//!   ending the session (a role granted, the actor record updated), so
//!   the watcher re-checks what the session may see.
//!
//! Validation and registration happen under the watcher lock, and every
//! change writes the database first and then notifies under that same
//! lock. A change can therefore never fall between the check and the
//! registration: it either happened before the check, which then fails,
//! or it notifies a watcher that is already registered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use super::SessionAuthRuntime;
use crate::errors::RuntimeError;
use crate::tracing::now_ms;

/// What a watched session may do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Valid. Published again when the actor's authority changes, so a
    /// watcher re-checks what the session may see.
    Active,
    /// Revoked, rotated or expired: the session no longer authorizes
    /// anything. Final.
    Invalid,
}

/// The sessions being watched, by session id.
#[derive(Default, Clone)]
pub(super) struct SessionWatchers {
    by_session: Arc<Mutex<HashMap<String, Watched>>>,
    next_generation: Arc<std::sync::atomic::AtomicU64>,
}

struct Watched {
    actor_id: String,
    sender: watch::Sender<SessionState>,
    /// Which watch of this session id this is: a rotated session keeps
    /// its id, and the old watch's expiry timer must not end the new one.
    generation: u64,
}

impl SessionWatchers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Watched>> {
        self.by_session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Publish `state` to the watchers of one session.
    pub(super) fn session_changed(&self, session_id: &str, state: SessionState) {
        let mut watched = self.lock();
        publish(&mut watched, |id, _| id == session_id, state);
    }

    /// Publish `state` to the watchers of every session of an actor.
    pub(super) fn actor_changed(&self, actor_id: &str, state: SessionState) {
        let mut watched = self.lock();
        publish(&mut watched, |_, entry| entry.actor_id == actor_id, state);
    }

    /// End one watch of a session when it expires, unless that watch has
    /// already ended and the id is watched again after a rotation.
    fn expired(&self, session_id: &str, generation: u64) {
        let mut watched = self.lock();
        publish(&mut watched, |id, entry| id == session_id && entry.generation == generation, SessionState::Invalid);
    }
}

fn publish(
    watched: &mut HashMap<String, Watched>,
    matches: impl Fn(&str, &Watched) -> bool,
    state: SessionState,
) {
    for (id, entry) in watched.iter() {
        if matches(id, entry) {
            entry.sender.send_replace(state);
        }
    }
    // An invalid session is never valid again, and a session nobody
    // watches any more needs no entry.
    watched.retain(|id, entry| {
        !(state == SessionState::Invalid && matches(id, entry)) && entry.sender.receiver_count() > 0
    });
}

impl SessionAuthRuntime {
    /// Validate a session cookie's token and watch the session from now
    /// on, atomically: no change to it can be missed between the check
    /// and the first value the receiver holds. The receiver starts at
    /// `Active`; see the module docs for what is published after.
    ///
    /// Must be called inside a Tokio runtime, which times the session's
    /// expiry. An invalid, expired or unknown token is an error, like
    /// [`Self::resolve_session_cookie`], and is audited the same way.
    pub fn observe_session(&self, raw_token: &str, trace_id: &str) -> Result<watch::Receiver<SessionState>, RuntimeError> {
        self.observe_with_clock(raw_token, trace_id, now_ms).map(|(receiver, _)| receiver)
    }

    pub(super) fn observe_with_clock(
        &self,
        raw_token: &str,
        trace_id: &str,
        clock: fn() -> u64,
    ) -> Result<(watch::Receiver<SessionState>, u64), RuntimeError> {
        let timer = tokio::runtime::Handle::try_current().map_err(|_| {
            RuntimeError::Other("watching a session needs a Tokio runtime to time its expiry".into())
        })?;
        let mut watched = self.watchers.lock();
        let at = clock();
        let resolved = self.resolve_session_cookie(raw_token, trace_id, at)?;
        let session = resolved.session;
        if let Some(entry) = watched.get(&session.id) {
            return Ok((entry.sender.subscribe(), session.expires_ms));
        }
        let (sender, receiver) = watch::channel(SessionState::Active);
        let generation = self.watchers.next_generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        watched.insert(session.id.clone(), Watched { actor_id: session.actor_id.clone(), sender, generation });
        drop(watched);
        // Expiry is the one change no call makes, so a timer makes it.
        let watchers = self.watchers.clone();
        let remaining = Duration::from_millis(session.expires_ms.saturating_sub(at));
        timer.spawn(async move {
            tokio::time::sleep(remaining).await;
            watchers.expired(&session.id, generation);
        });
        Ok((receiver, session.expires_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{AuthActor, SessionCreate};

    fn actor() -> AuthActor {
        AuthActor {
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
        }
    }

    /// A store with one session for `user-1`, `lifetime_ms` from now.
    fn store(id: &str, token: &str, lifetime_ms: u64) -> SessionAuthRuntime {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor()).unwrap();
        add_session(&auth, id, token, lifetime_ms);
        auth
    }

    fn add_session(auth: &SessionAuthRuntime, id: &str, token: &str, lifetime_ms: u64) {
        let now = now_ms();
        auth.create_session(SessionCreate {
            id: id.into(),
            actor_id: "user-1".into(),
            tenant_id: "org-1".into(),
            raw_token: token.into(),
            issued_ms: now - 1,
            expires_ms: now + lifetime_ms,
            csrf_binding_id: "csrf".into(),
        })
        .unwrap();
    }

    async fn next(receiver: &mut watch::Receiver<SessionState>) -> SessionState {
        tokio::time::timeout(Duration::from_secs(5), receiver.changed()).await.expect("a change arrives").expect("open");
        *receiver.borrow_and_update()
    }

    #[tokio::test]
    async fn logout_invalidates_every_watcher_of_the_session() {
        let auth = store("sess-1", "token-1", 60_000);
        let mut first = auth.observe_session("token-1", "trace").unwrap();
        let mut second = auth.observe_session("token-1", "trace").unwrap();
        assert_eq!(*first.borrow(), SessionState::Active);
        auth.revoke_session("sess-1", now_ms()).unwrap();
        assert_eq!(next(&mut first).await, SessionState::Invalid);
        assert_eq!(next(&mut second).await, SessionState::Invalid);
        assert!(auth.observe_session("token-1", "trace").is_err(), "a revoked session cannot be watched");
    }

    #[tokio::test]
    async fn an_unknown_or_expired_token_is_refused() {
        let auth = store("sess-1", "token-1", 60_000);
        assert!(auth.observe_session("not-a-token", "trace").is_err());
        let now = now_ms();
        auth.create_session(SessionCreate {
            id: "old".into(),
            actor_id: "user-1".into(),
            tenant_id: "org-1".into(),
            raw_token: "old-token".into(),
            issued_ms: now - 10,
            expires_ms: now - 1,
            csrf_binding_id: "csrf".into(),
        })
        .unwrap();
        assert!(auth.observe_session("old-token", "trace").is_err());
    }

    #[tokio::test]
    async fn a_session_is_invalid_when_it_expires() {
        let auth = store("sess-1", "token-1", 150);
        let mut watcher = auth.observe_session("token-1", "trace").unwrap();
        assert_eq!(next(&mut watcher).await, SessionState::Invalid);
    }

    #[tokio::test]
    async fn rotation_invalidates_the_old_token_watchers() {
        let auth = store("sess-1", "token-1", 60_000);
        let mut watcher = auth.observe_session("token-1", "trace").unwrap();
        auth.rotate_session("sess-1", "token-2", now_ms() + 60_000).unwrap();
        assert_eq!(next(&mut watcher).await, SessionState::Invalid);
        let renewed = auth.observe_session("token-2", "trace").unwrap();
        assert_eq!(*renewed.borrow(), SessionState::Active, "the new token is watched afresh");
    }

    #[tokio::test]
    async fn the_old_tokens_expiry_does_not_end_the_rotated_sessions_watch() {
        let auth = store("sess-1", "token-1", 200);
        let mut old = auth.observe_session("token-1", "trace").unwrap();
        auth.rotate_session("sess-1", "token-2", now_ms() + 60_000).unwrap();
        assert_eq!(next(&mut old).await, SessionState::Invalid);
        let mut renewed = auth.observe_session("token-2", "trace").unwrap();
        // Past the old token's expiry: its timer fires, but for a watch
        // that has already ended.
        let quiet = tokio::time::timeout(Duration::from_millis(500), renewed.changed()).await;
        assert!(quiet.is_err(), "the rotated session is still valid");
        assert_eq!(*renewed.borrow(), SessionState::Active);
    }

    #[tokio::test]
    async fn a_role_taken_away_invalidates_and_a_role_granted_re_checks() {
        let auth = store("sess-1", "token-1", 60_000);
        add_session(&auth, "sess-2", "token-2", 60_000);
        let mut one = auth.observe_session("token-1", "trace").unwrap();
        let mut two = auth.observe_session("token-2", "trace").unwrap();

        auth.grant_actor_role("user-1", "editor", now_ms()).unwrap();
        assert_eq!(next(&mut one).await, SessionState::Active, "more authority: re-check what it may see");
        assert_eq!(next(&mut two).await, SessionState::Active);

        auth.revoke_actor_role("user-1", "editor", now_ms()).unwrap();
        assert_eq!(next(&mut one).await, SessionState::Invalid, "less authority: the sessions end");
        assert_eq!(next(&mut two).await, SessionState::Invalid);
    }

    #[tokio::test]
    async fn updating_the_actor_re_checks_its_sessions() {
        let auth = store("sess-1", "token-1", 60_000);
        let mut watcher = auth.observe_session("token-1", "trace").unwrap();
        let mut changed = actor();
        changed.permission_fingerprint = "sha256:new-permissions".into();
        changed.updated_ms = 0;
        auth.upsert_actor(changed).unwrap();
        assert_eq!(next(&mut watcher).await, SessionState::Active);
    }

    #[tokio::test]
    async fn another_actors_change_reaches_no_one_else() {
        let auth = store("sess-1", "token-1", 60_000);
        let mut watcher = auth.observe_session("token-1", "trace").unwrap();
        let mut other = actor();
        other.id = "user-2".into();
        auth.upsert_actor(other).unwrap();
        auth.grant_actor_role("user-2", "editor", now_ms()).unwrap();
        let quiet = tokio::time::timeout(Duration::from_millis(200), watcher.changed()).await;
        assert!(quiet.is_err(), "user-2's changes are not user-1's");
    }

    #[tokio::test]
    async fn a_session_nobody_watches_leaves_no_entry() {
        let auth = store("sess-1", "token-1", 60_000);
        drop(auth.observe_session("token-1", "trace").unwrap());
        auth.grant_actor_role("user-1", "editor", now_ms()).unwrap();
        assert!(auth.watchers.lock().is_empty());
    }

    #[test]
    fn watching_outside_a_tokio_runtime_is_refused_by_name() {
        let auth = store("sess-1", "token-1", 60_000);
        let error = auth.observe_session("token-1", "trace").unwrap_err().to_string();
        assert!(error.contains("Tokio runtime"), "{error}");
    }
}
