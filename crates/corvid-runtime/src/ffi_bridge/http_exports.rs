//! C-ABI bridge module for the executing HTTP-client surface.
//!
//! Phase 33S0 — module scaffold only. Native code has no HTTP entry
//! points yet: connector calls run on the interpreter until native
//! dispatch of runtime-owned calls lands (B05 slice 7). This file
//! keeps the per-surface `SurfaceNotImplemented` helper the registry
//! side (effect profiles, `io_source` dimension) was wired against
//! in 33S0.
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
