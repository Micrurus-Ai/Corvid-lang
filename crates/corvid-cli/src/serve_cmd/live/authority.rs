//! Observe the same durable cookie and actor authority that HTTP handlers use.
use corvid_runtime::{auth::SessionObservation, SessionAuthRuntime};
use std::{sync::Arc, time::Duration};

pub(super) struct Authority {
    auth: Arc<SessionAuthRuntime>,
    token: String,
    observed: SessionObservation,
}

fn validate(
    auth: &Arc<SessionAuthRuntime>,
    token: &str,
) -> Result<SessionObservation, corvid_runtime::RuntimeError> {
    let trace = format!("live-auth-{}", crate::serve_auth::pkce::random_token());
    auth.observe_session_cookie(token, &trace, crate::serve_cmd::epoch_ms)
}

impl Authority {
    pub(super) fn observe(
        auth: Arc<SessionAuthRuntime>,
        token: String,
    ) -> Result<Self, axum::http::StatusCode> {
        let observed = validate(&auth, &token).map_err(|_| axum::http::StatusCode::UNAUTHORIZED)?;
        Ok(Self {
            auth,
            token,
            observed,
        })
    }

    /// True when every active query needs to be authorized again.
    pub(super) fn refresh(&mut self) -> Result<bool, u16> {
        if self.observed.is_current().map_err(|_| 1011u16)? {
            return Ok(false);
        }
        self.observed = validate(&self.auth, &self.token).map_err(|_| 1008u16)?;
        Ok(true)
    }

    pub(super) fn next_check(&self) -> Duration {
        Duration::from_millis(self.observed.remaining_ms().min(1000))
    }

    pub(super) fn enqueue(&self, send: impl FnOnce() -> Result<(), u16>) -> Result<(), u16> {
        self.observed
            .while_current(send)
            .map_err(|_| 1011u16)?
            .transpose()?;
        Ok(())
    }
}
