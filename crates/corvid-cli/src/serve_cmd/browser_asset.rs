use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::MethodRouter;
use sha2::{Digest, Sha256};
use std::sync::Arc;

use super::ServeState;

/// Recognize the standard document footer and only the controls this smaller entrypoint binds.
pub(super) fn select_quantity_loader(html: &mut String) {
    const FOOTER: &str = "<script type=\"module\" data-corvid-ui src=\"/target/wasm/events.js\"></script></main></body></html>";
    let Some(content) = html.strip_suffix(FOOTER) else {
        return;
    };
    if !content.contains("<div data-corvid-quantity-field>") {
        return;
    }
    let normalized = content.to_ascii_lowercase();
    if normalized.contains("<script") {
        return;
    }
    if normalized.split("data-corvid-").skip(1).any(|suffix| {
        let name = suffix
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .next();
        !matches!(
            name,
            Some("quantity-field" | "quantity" | "list" | "key" | "move")
        )
    }) {
        return;
    }
    let offset = content.len() + FOOTER.find("events.js").unwrap() + "events".len();
    html.insert_str(offset, ".quantity");
}

pub(super) fn browser_asset(
    bytes: Bytes,
    mime: &'static str,
    versioned: bool,
) -> MethodRouter<Arc<ServeState>> {
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let version = format!("v={digest}");
    let etag = HeaderValue::from_str(&format!("\"{digest}\"")).expect("hex digest is ASCII");
    get(move |RawQuery(query): RawQuery, request: HeaderMap| {
        let immutable = versioned && query.as_deref() == Some(&version);
        let wrong_version = versioned
            && !immutable
            && query.as_deref().is_some_and(|q| {
                q.split('&')
                    .any(|part| part == "v" || part.starts_with("v="))
            });
        let bytes = bytes.clone();
        let etag = etag.clone();
        async move {
            if wrong_version {
                return (StatusCode::NOT_FOUND, [(header::CACHE_CONTROL, "no-store")])
                    .into_response();
            }
            let policy = if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            let cached = request
                .get_all(header::IF_NONE_MATCH)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .any(|tag| {
                    tag == "*" || tag.trim_start_matches("W/").as_bytes() == etag.as_bytes()
                });
            let mut response = if cached {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                ([(header::CONTENT_TYPE, mime)], bytes).into_response()
            };
            response.headers_mut().insert(header::ETAG, etag);
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(policy));
            response
        }
    })
}
