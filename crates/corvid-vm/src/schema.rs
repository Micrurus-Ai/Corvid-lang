//! Re-exports of `corvid-prompt-format` for backwards compatibility
//! with internal callers that reach into `corvid_vm::schema` directly.
//!
//! The actual implementation lives in the dedicated
//! `corvid-prompt-format` crate so the native code generator can
//! depend on it without pulling in the interpreter. This module is a
//! thin shim that preserves the existing `corvid_vm::schema::schema_for`
//! call sites without forcing every caller to update.

pub use corvid_prompt_format::schema_for;

/// The schema a prompt's reply is requested with; see
/// `corvid_prompt_format::reply_schema`, which native code asks too.
pub(crate) fn prompt_output_schema(
    ty: &corvid_types::Type,
    types: &std::collections::HashMap<corvid_resolve::DefId, &corvid_ir::IrType>,
) -> Option<serde_json::Value> {
    corvid_prompt_format::reply_schema(ty, types)
}
