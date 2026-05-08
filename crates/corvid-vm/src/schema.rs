//! Re-exports of `corvid-prompt-format` for backwards compatibility
//! with internal callers that reach into `corvid_vm::schema` directly.
//!
//! The actual implementation lives in the dedicated
//! `corvid-prompt-format` crate so the native code generator can
//! depend on it without pulling in the interpreter. This module is a
//! thin shim that preserves the existing `corvid_vm::schema::schema_for`
//! call sites without forcing every caller to update.

pub use corvid_prompt_format::schema_for;
