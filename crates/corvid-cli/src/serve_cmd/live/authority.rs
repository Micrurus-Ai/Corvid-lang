//! Observe the same durable cookie and actor authority that HTTP handlers use.
use corvid_runtime::{
    auth::{SessionObservation, SessionState},
    SessionAuthRuntime,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;

pub(super) struct Authority {
    auth: Arc<SessionAuthRuntime>,
    token: String,
    observed: SessionObservation,
    updates: watch::Receiver<SessionState>,
}

fn validate(
    auth: &Arc<SessionAuthRuntime>,
    token: &str,
) -> Result<(SessionObservation, watch::Receiver<SessionState>), corvid_runtime::RuntimeError> {
    let trace = format!("live-auth-{}", crate::serve_auth::pkce::random_token());
    auth.watch_session_cookie(token, &trace, crate::serve_cmd::epoch_ms)
}

impl Authority {
    pub(super) fn observe(
        auth: Arc<SessionAuthRuntime>,
        token: String,
    ) -> Result<Self, axum::http::StatusCode> {
        let (observed, updates) =
            validate(&auth, &token).map_err(|_| axum::http::StatusCode::UNAUTHORIZED)?;
        Ok(Self {
            auth,
            token,
            observed,
            updates,
        })
    }

    /// True when every active query needs to be authorized again.
    pub(super) fn refresh(&mut self) -> Result<bool, u16> {
        if *self.updates.borrow() == SessionState::Active
            && self.observed.is_current().map_err(|_| 1011u16)?
        {
            return Ok(false);
        }
        (self.observed, self.updates) = validate(&self.auth, &self.token).map_err(|_| 1008u16)?;
        Ok(true)
    }

    pub(super) async fn changed(&mut self) {
        let delay = Duration::from_millis(self.observed.remaining_ms().min(1000));
        tokio::select! {
            _ = self.updates.changed() => {}
            _ = tokio::time::sleep(delay) => {}
        }
    }

    pub(super) fn enqueue(&self, send: impl FnOnce() -> Result<(), u16>) -> Result<(), u16> {
        self.observed
            .while_current(send)
            .map_err(|_| 1011u16)?
            .transpose()?;
        Ok(())
    }
}
