//! An `Upload`'s content, wherever it is held.
//!
//! `corvid serve` builds an `Upload` with its content in memory (the
//! `bytes` field) or, for a route declared `@upload(..., disk_over_mb:
//! N)` and a file past N MB, in a private temporary file (the `file`
//! field, a `Value::HeldUpload`). Everything that reads an upload's
//! content or digest goes through this module, so the two forms behave
//! the same: `upload.bytes()` and `upload.text()` read the same values,
//! and a trace records the same SHA-256.

use crate::value::{StructValue, Value};

/// The upload's content. A held file is read into memory here, because
/// the program asked for its bytes.
pub fn upload_content(upload: &StructValue) -> Result<corvid_crypto::Bytes, String> {
    match (upload.get_field("bytes"), upload.get_field("file")) {
        (Some(Value::Bytes(bytes)), _) => Ok(bytes),
        (_, Some(Value::HeldUpload(held))) => held
            .read()
            .map(corvid_crypto::Bytes::from)
            .map_err(|e| format!("the upload held on disk could not be read: {e}")),
        _ => Err("the upload has no content".to_string()),
    }
}

/// The SHA-256 a trace records for the upload. A held file's was
/// computed while it was written, so it is never read again to digest.
pub fn upload_sha256_hex(upload: &StructValue) -> String {
    match (upload.get_field("bytes"), upload.get_field("file")) {
        (Some(Value::Bytes(bytes)), _) => corvid_crypto::sha256(&bytes).to_hex(),
        (_, Some(Value::HeldUpload(held))) => held.sha256_hex().to_string(),
        _ => String::new(),
    }
}
