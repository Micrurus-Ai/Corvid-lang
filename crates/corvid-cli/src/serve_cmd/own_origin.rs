//! This server's own origin, as browsers see it: the one rule every check
//! that compares a request's origin uses.

use axum::http::HeaderMap;

/// The only value of header `name`, or `None` when it is absent, repeated
/// or not text.
pub(super) fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

/// The server's own origin: the configured public base URL, or, without
/// one, the request's `Host` (refused when it carries credentials).
pub(super) fn own_origin(headers: &HeaderMap, public_base: Option<&str>) -> Option<reqwest::Url> {
    match public_base {
        Some(base) => reqwest::Url::parse(base).ok(),
        None => single_header(headers, "host")
            .and_then(|host| host.parse::<axum::http::uri::Authority>().ok())
            .filter(|authority| !authority.as_str().contains('@'))
            .and_then(|authority| reqwest::Url::parse(&format!("http://{authority}")).ok()),
    }
}
