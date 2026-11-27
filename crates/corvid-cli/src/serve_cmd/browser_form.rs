//! Answering a plain browser form posted to an action (B10-2b-5).
//!
//! An action returns data, not a page. A form a person submits from a page
//! (`application/x-www-form-urlencoded` asking for `text/html`) therefore
//! gets:
//! - `303 See Other` back to the page it came from, when the action
//!   succeeded. The page is read from `Referer` and used only when it names
//!   this server's own host, and only as a path, so the answer can never
//!   send the browser elsewhere. Otherwise it goes to `/`.
//! - a small HTML page naming what went wrong, with a link back, when the
//!   input was refused or the action failed;
//! - a page with the approval's id when the action is waiting for one.
//!
//! JSON callers, and forms submitted by the page script with `fetch`, get
//! the action's ordinary answer.
//!
//! A plain form cannot set the `X-CSRF-Token` header, so it carries the
//! token in the reserved `corvid_csrf` field; [`with_form_csrf`] moves it
//! into the header the one double-submit check reads.

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use corvid_types::action_input::CSRF_FORM_FIELD;

/// True for a form a browser posted and expects a page back from.
pub(super) fn wants_page(headers: &HeaderMap) -> bool {
    is_form(headers)
        && headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|accept| accept.split(',').any(|part| part.trim().starts_with("text/html")))
}

fn is_form(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/x-www-form-urlencoded"))
}

/// The headers the policy check reads: as sent, or, for a form without an
/// `X-CSRF-Token` header, with the token from its `corvid_csrf` field. The
/// token is checked by the same double-submit rule either way; only where
/// the copy travels changes. An empty or missing field leaves the header
/// absent, so the post is refused: it fails closed.
pub(super) fn with_form_csrf(headers: &HeaderMap, body: &Bytes) -> HeaderMap {
    let mut headers = headers.clone();
    if !is_form(&headers) || headers.contains_key("x-csrf-token") {
        return headers;
    }
    let token = std::str::from_utf8(body)
        .ok()
        .and_then(|text| crate::request_text::urlencoded_values(text).ok())
        .and_then(|values| values.get(CSRF_FORM_FIELD).and_then(|v| v.first().cloned()))
        .filter(|token| !token.is_empty());
    if let Some(value) = token.and_then(|token| HeaderValue::from_str(&token).ok()) {
        headers.insert("x-csrf-token", value);
    }
    headers
}

/// The page a browser form gets for the action's `answer`.
pub(super) async fn page_answer(answer: Response, request: &HeaderMap, public_base: Option<&str>, action: &str) -> Response {
    let status = answer.status();
    let back = back_path(request, public_base);
    if status.is_success() && status != StatusCode::ACCEPTED {
        return (StatusCode::SEE_OTHER, [(header::LOCATION, back)]).into_response();
    }
    let body = axum::body::to_bytes(answer.into_body(), 1 << 20).await.unwrap_or_default();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    let text = |key: &str| json.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let (heading, detail) = if status == StatusCode::ACCEPTED {
        (format!("{action} is waiting for approval"), format!("Approval {}", text("approval_id")))
    } else {
        let error = text("error");
        let detail = text("detail");
        (
            format!("{action} did not run"),
            match (error.is_empty(), detail.is_empty()) {
                (true, true) => status.canonical_reason().unwrap_or("refused").to_string(),
                (false, true) => error,
                (_, false) => detail,
            },
        )
    };
    let page = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{h}</title></head><body><h1>{h}</h1><p>{d}</p><p><a href=\"{b}\">Back</a></p></body></html>",
        h = escape(&heading),
        d = escape(&detail),
        b = escape(&back),
    );
    (status, Html(page)).into_response()
}

/// The path of the page the form came from, when `Referer` has this
/// server's own origin (the rule the live endpoint checks `Origin` with);
/// `/` otherwise. Only a path is ever returned, never a host.
fn back_path(request: &HeaderMap, public_base: Option<&str>) -> String {
    let own = super::own_origin::own_origin(request, public_base);
    let referer = super::own_origin::single_header(request, "referer").and_then(|r| reqwest::Url::parse(r).ok());
    let (Some(own), Some(referer)) = (own, referer) else { return "/".into() };
    if !matches!(referer.scheme(), "http" | "https") || referer.origin() != own.origin() {
        return "/".into();
    }
    let mut path = referer.path().to_string();
    if let Some(query) = referer.query() {
        path.push('?');
        path.push_str(query);
    }
    // A path that begins `//` would be read as another host.
    if !path.starts_with('/') || path.starts_with("//") {
        return "/".into();
    }
    path
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn back_goes_only_to_this_servers_own_pages() {
        let back = |referer: &str| back_path(&headers(&[("host", "app.example:8080"), ("referer", referer)]), None);
        assert_eq!(back("http://app.example:8080/board/7?tab=done#top"), "/board/7?tab=done");
        assert_eq!(back("https://app.example:8080"), "/");
        assert_eq!(back("https://evil.example/board"), "/", "another host is never a destination");
        assert_eq!(back("http://app.example:8080//evil.example/x"), "/", "a path read as a host is refused");
        assert_eq!(back("javascript:alert(1)"), "/");
        assert_eq!(back_path(&headers(&[("host", "app.example:8080")]), None), "/");
    }

    #[test]
    fn a_form_carries_its_csrf_token_into_the_header_the_check_reads() {
        let form = headers(&[("content-type", "application/x-www-form-urlencoded")]);
        let moved = with_form_csrf(&form, &Bytes::from_static(b"card=3&corvid_csrf=abc123"));
        assert_eq!(moved.get("x-csrf-token").unwrap(), "abc123");
        let empty = with_form_csrf(&form, &Bytes::from_static(b"card=3&corvid_csrf="));
        assert!(!empty.contains_key("x-csrf-token"), "an empty field leaves the check to refuse");
        let sent = headers(&[("content-type", "application/x-www-form-urlencoded"), ("x-csrf-token", "from-header")]);
        assert_eq!(with_form_csrf(&sent, &Bytes::from_static(b"corvid_csrf=from-field")).get("x-csrf-token").unwrap(), "from-header");
        let json = headers(&[("content-type", "application/json")]);
        assert!(!with_form_csrf(&json, &Bytes::from_static(b"{\"corvid_csrf\":\"x\"}")).contains_key("x-csrf-token"));
    }

    #[test]
    fn only_a_browser_form_asking_for_html_gets_a_page() {
        assert!(wants_page(&headers(&[("content-type", "application/x-www-form-urlencoded"), ("accept", "text/html,application/xhtml+xml")])));
        assert!(!wants_page(&headers(&[("content-type", "application/x-www-form-urlencoded"), ("accept", "application/json")])));
        assert!(!wants_page(&headers(&[("content-type", "application/json"), ("accept", "text/html")])));
    }
}
