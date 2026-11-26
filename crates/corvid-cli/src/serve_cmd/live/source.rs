//! Bind each live query to the same authenticated actor a GET route receives.
use super::{AuthorizedSource, Failure, ServeState, Session, Subscription};
use crate::serve_cmd::{cookie_value, enforce_route_policy};
use axum::http::{HeaderMap, StatusCode};
use corvid_ast::HttpMethod;
use std::sync::Arc;

struct RouteSource {
    state: Arc<ServeState>,
    headers: HeaderMap,
}

impl AuthorizedSource for RouteSource {
    fn open(&mut self, path: &str) -> Result<Subscription, Failure> {
        let (route, argument) = self
            .state
            .live_routes
            .resolve(&self.state.ir, path)
            .ok_or(Failure::Forbidden)?;
        let actor = match route
            .policy
            .as_ref()
            .filter(|policy| policy.requires_auth())
        {
            Some(policy) => {
                enforce_route_policy(
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
                })?
                .value
            }
            None => crate::serve_cmd::stub_actor_value(),
        };
        super::feed::subscribe(
            self.state.ir.clone(),
            self.state.runtime.clone(),
            self.state.declared_types.clone(),
            route,
            argument,
            actor,
        )
    }
}

pub(super) fn connect(state: Arc<ServeState>, headers: HeaderMap) -> Result<Session, StatusCode> {
    let auth = state
        .auth_context
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let token = cookie_value(&headers, &auth.cookie.name).ok_or(StatusCode::UNAUTHORIZED)?;
    let status = super::authority::Authority::observe(auth.auth.clone(), token)?;
    Ok(Session {
        status,
        source: Box::new(RouteSource { state, headers }),
    })
}
