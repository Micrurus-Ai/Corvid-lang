//! C-ABI bridge module for the executing SQLite surface.
//!
//! Phase 33S0 — module scaffold only. The actual extern "C" entry
//! points (`corvid_db_open`, `corvid_db_query`, `corvid_db_execute`)
//! and their codegen-cl declarations land in slice 33S3. This file
//! exists in 33S0 so the per-surface work in 33S3 has a settled
//! home; the registry-side (effect profiles, `io_source`
//! dimension, `SurfaceNotImplemented` error variant) is all wired
//! in 33S0.
//!
//! Note: the `Value::DbHandle` opaque connection-handle variant +
//! the per-handle quarantine mode for replay land in 33S3
//! alongside the runtime wiring, since the handle's lifecycle is
//! tied to the actual `rusqlite::Connection` it wraps.

#![allow(unsafe_code)]

use crate::errors::RuntimeError;

/// Phase 33S0 helper — see the doc on
/// `io_exports::surface_not_implemented` for the contract.
#[allow(dead_code)]
pub(crate) fn surface_not_implemented(function: &str) -> RuntimeError {
    RuntimeError::SurfaceNotImplemented {
        surface: "db".to_string(),
        function: function.to_string(),
    }
}
