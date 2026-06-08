//! C-ABI bridge module for the executing HTTP-client surface.
//!
//! Phase 33S0 — module scaffold only. The actual extern "C" entry
//! points (`corvid_http_get`, `corvid_http_post_json`) and their
//! codegen-cl declarations land in slice 33S2. This file exists
//! in 33S0 so the per-surface work in 33S2 has a settled home;
//! the registry-side (effect profiles, `io_source` dimension,
//! `SurfaceNotImplemented` error variant) is all wired in 33S0.
//!
//! Note: the SSRF-block + `[http] allow` allowlist enforcement
//! lives in 33S2 alongside the runtime wiring, since both need
//! the actual `HttpClient::send` call site to plug into.

#![allow(unsafe_code)]

use crate::errors::RuntimeError;

/// Phase 33S0 helper — see the doc on
/// `io_exports::surface_not_implemented` for the contract.
#[allow(dead_code)]
pub(crate) fn surface_not_implemented(function: &str) -> RuntimeError {
    RuntimeError::SurfaceNotImplemented {
        surface: "http".to_string(),
        function: function.to_string(),
    }
}
