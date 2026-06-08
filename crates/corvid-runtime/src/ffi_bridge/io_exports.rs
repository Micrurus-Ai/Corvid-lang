//! C-ABI bridge module for the executing file-I/O surface.
//!
//! Phase 33S0 — module scaffold only. The actual extern "C" entry
//! points (`corvid_io_read_text`, `corvid_io_write_text`,
//! `corvid_io_list_dir`) and their codegen-cl declarations land
//! in slice 33S1. This file exists in 33S0 so the per-surface
//! work in 33S1 has a settled home; the registry-side (effect
//! profiles, `io_source` dimension, `SurfaceNotImplemented`
//! error variant) is all wired in 33S0 so an early caller hits
//! a precise diagnostic rather than a missing symbol.

#![allow(unsafe_code)]

use crate::errors::RuntimeError;

/// Phase 33S0 helper — used by 33S1's actual entry points when
/// the runtime side of a file-I/O function is invoked before its
/// implementation lands. Once 33S1 ships, callers reach the real
/// `IoRuntime::read_text` / `write_text` / `list_dir` paths and
/// this helper is unreachable from production code (kept for
/// regression tests that pin the pre-impl behavior).
#[allow(dead_code)]
pub(crate) fn surface_not_implemented(function: &str) -> RuntimeError {
    RuntimeError::SurfaceNotImplemented {
        surface: "io".to_string(),
        function: function.to_string(),
    }
}
