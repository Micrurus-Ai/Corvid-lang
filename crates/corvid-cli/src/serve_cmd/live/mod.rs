//! Browser socket upgrade and the integration point for RT-1 and RT-2.
mod authority;
mod connection;
mod feed;
pub(super) use feed::share_commits;
mod routes;
pub(super) use routes::Routes;
pub(super) mod protocol;
pub(super) mod source;

#[cfg(test)]
mod tests;

use super::ServeState;
use axum::extract::{ws::WebSocketUpgrade, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::stream::BoxStream;
use protocol::Delivery;
#[cfg(test)]
use protocol::Row;
use std::sync::Arc;

pub(super) const PATH: &str = "/.well-known/corvid/live";
const PROTOCOL: &str = "corvid.live.v1";

pub(super) struct Session {
    source: Box<dyn AuthorizedSource>,
    status: authority::Authority,
}

/// Each open (including re-open after a role change) resolves a declared path,
/// applies serve's route policy and installs the actor-filtered snapshot and
/// stream at one commit boundary. Dropping the stream closes the handle.
pub(super) trait AuthorizedSource: Send {
    fn open(&mut self, path: &str) -> Result<Subscription, Failure>;
}

pub(super) type Subscription = BoxStream<'static, Result<Delivery, Failure>>;

#[derive(Debug)]
pub(super) enum Failure {
    Forbidden,
    InvalidSession,
    Unavailable,
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

fn same_origin(headers: &HeaderMap, public_base: Option<&str>) -> bool {
    let Some(origin) = single_header(headers, "origin") else {
        return false;
    };
    let expected = match public_base {
        Some(base) => reqwest::Url::parse(base).ok(),
        None => single_header(headers, "host")
            .and_then(|host| host.parse::<axum::http::uri::Authority>().ok())
            .filter(|authority| !authority.as_str().contains('@'))
            .and_then(|authority| reqwest::Url::parse(&format!("http://{authority}")).ok()),
    };
    expected.is_some_and(|url| {
        matches!(url.scheme(), "https" | "http") && origin == url.origin().ascii_serialization()
    })
}

pub(super) async fn upgrade(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let base = state
        .auth_context
        .as_ref()
        .and_then(|ctx| ctx.public_base_url.as_deref());
    if !same_origin(&headers, base) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !single_header(&headers, "sec-websocket-protocol")
        .is_some_and(|v| v.split(',').any(|v| v.trim() == PROTOCOL))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if state.live_routes.is_empty() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match source::connect(state.clone(), headers) {
        Ok(session) => {
            let shutdown = state.live_shutdown.subscribe();
            ws.write_buffer_size(0)
                .protocols([PROTOCOL])
                .on_upgrade(move |socket| connection::run(socket, session, shutdown))
        }
        Err(status) => status.into_response(),
    }
}
