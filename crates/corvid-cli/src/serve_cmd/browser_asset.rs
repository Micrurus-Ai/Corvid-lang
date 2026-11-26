use axum::body::Bytes;
use axum::extract::RawQuery;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::MethodRouter;
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::sync::Arc;

use super::ServeState;

#[derive(Default)]
pub(super) struct PageScripts {
    pub(super) events: Option<String>,
    pub(super) quantity: Option<String>,
}

impl PageScripts {
    pub(super) fn tag(name: &str, bytes: &[u8]) -> String {
        let digest = STANDARD.encode(Sha256::digest(bytes));
        format!(
            r#"<script type="module" data-corvid-ui integrity="sha256-{digest}" src="/target/wasm/{name}"></script>"#
        )
    }

    pub(super) fn attach(&self, document: &mut String) {
        const ENTRY: &str =
            r#"<script type="module" data-corvid-ui src="/target/wasm/events.js"></script>"#;
        let Some(main) = document.strip_suffix("</main></body></html>") else {
            return;
        };
        let Some(content) = main.strip_suffix(ENTRY) else {
            return;
        };
        let tag = self
            .quantity
            .as_ref()
            .filter(|_| is_quantity_page(content))
            .or(self.events.as_ref());
        if let Some(tag) = tag {
            document.replace_range(content.len()..main.len(), tag);
        }
    }
}

/// Recognize the standard document footer and only the controls this smaller entrypoint binds.
fn is_quantity_page(content: &str) -> bool {
    if !content.contains("<div data-corvid-quantity-field>") {
        return false;
    }
    let normalized = content.to_ascii_lowercase();
    if normalized.contains("<script") || normalized.contains("<corvid-") {
        return false;
    }
    normalized.split("data-").skip(1).all(|suffix| {
        let name = suffix
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .next();
        matches!(
            name,
            Some("corvid-quantity-field" | "corvid-quantity" | "corvid-list" | "corvid-key" | "corvid-move")
        )
    })
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
