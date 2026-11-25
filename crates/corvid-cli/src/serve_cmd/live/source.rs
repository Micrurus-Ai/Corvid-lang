//! Bind each live query to the same authenticated actor a GET route receives.
use super::{AuthorizedSource, Failure, ServeState, Session, SessionStatus, Subscription};
use crate::serve_cmd::{cookie_value, enforce_route_policy, EnforcedRouteActor};
use axum::http::{HeaderMap, StatusCode};
use corvid_ast::HttpMethod;
use corvid_ir::IrRoutePolicy;
use std::sync::Arc;
use tokio::sync::watch;

pub(in crate::serve_cmd) trait Provider: Send + Sync {
    /// RT-1 resolves only declared live paths and binds their typed arguments.
    fn resolve(&self, path: &str) -> Option<Box<dyn Query>>;

    /// Validate this serve session and register for changes atomically.
    /// Publish Active on changes to roles or visibility, Invalid on expiry or
    /// logout, synchronized with the change becoming effective.
    fn observe_session(&self, token: &str) -> Result<watch::Receiver<SessionStatus>, StatusCode>;
}

pub(in crate::serve_cmd) trait Query {
    fn policy(&self) -> &IrRoutePolicy;
    /// RT-2 opens the filtered, revisioned stream at the snapshot boundary.
    fn open(self: Box<Self>, actor: EnforcedRouteActor) -> Result<Subscription, Failure>;
}

struct RouteSource {
    state: Arc<ServeState>,
    provider: Arc<dyn Provider>,
    headers: HeaderMap,
}

impl AuthorizedSource for RouteSource {
    fn open(&mut self, path: &str) -> Result<Subscription, Failure> {
        let query = self.provider.resolve(path).ok_or(Failure::Forbidden)?;
        if !query.policy().requires_auth() {
            return Err(Failure::Unavailable);
        }
        let actor = enforce_route_policy(
            &self.state,
            query.policy(),
            HttpMethod::Get,
            path,
            &self.headers,
        )
        .map_err(|response| match response.status() {
            StatusCode::UNAUTHORIZED => Failure::InvalidSession,
            StatusCode::FORBIDDEN => Failure::Forbidden,
            _ => Failure::Unavailable,
        })?;
        query.open(actor)
    }
}

pub(super) fn connect(
    state: Arc<ServeState>,
    provider: Arc<dyn Provider>,
    headers: HeaderMap,
) -> Result<Session, StatusCode> {
    let auth = state
        .auth_context
        .as_ref()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token = cookie_value(&headers, &auth.cookie.name).ok_or(StatusCode::UNAUTHORIZED)?;
    let status = provider.observe_session(&token)?;
    if matches!(*status.borrow(), SessionStatus::Invalid) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Session {
        status,
        source: Box::new(RouteSource {
            state,
            provider,
            headers,
        }),
    })
}
