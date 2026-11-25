//! Bind each live query to the same authenticated actor a GET route receives.
use super::{AuthorizedSource, Failure, ServeState, Session, SessionStatus, Subscription};
use crate::serve_cmd::{cookie_value, enforce_route_policy, EnforcedRouteActor};
use axum::http::{HeaderMap, StatusCode};
use corvid_ast::HttpMethod;
use std::sync::Arc;
use tokio::sync::watch;

pub(in crate::serve_cmd) trait Provider: Send + Sync {
    /// Open RT-2's consistent snapshot and feed for the declared handler.
    fn open(
        &self,
        route: &corvid_ir::IrRoute,
        path: corvid_driver::Value,
        actor: EnforcedRouteActor,
    ) -> Result<Subscription, Failure>;

    /// Validate this serve session and register for changes atomically.
    /// Publish Active on changes to roles or visibility, Invalid on expiry or
    /// logout, synchronized with the change becoming effective.
    fn observe_session(&self, token: &str) -> Result<watch::Receiver<SessionStatus>, StatusCode>;
}

struct RouteSource {
    state: Arc<ServeState>,
    provider: Arc<dyn Provider>,
    headers: HeaderMap,
}

impl AuthorizedSource for RouteSource {
    fn open(&mut self, path: &str) -> Result<Subscription, Failure> {
        let (route, argument) = self
            .state
            .live_routes
            .resolve(&self.state.ir, path)
            .ok_or(Failure::Forbidden)?;
        let policy = route
            .policy
            .as_ref()
            .filter(|p| p.requires_auth())
            .ok_or(Failure::Unavailable)?;
        let actor = enforce_route_policy(
            &self.state,
            policy,
            HttpMethod::Get,
            &route.path,
            &self.headers,
        )
        .map_err(|response| match response.status() {
            StatusCode::UNAUTHORIZED => Failure::InvalidSession,
            StatusCode::FORBIDDEN => Failure::Forbidden,
            _ => Failure::Unavailable,
        })?;
        self.provider.open(route, argument, actor)
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
