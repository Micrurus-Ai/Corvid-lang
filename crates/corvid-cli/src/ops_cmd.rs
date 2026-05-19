//! `corvid ops show` — verify a signed `/__ops` snapshot.
//!
//! File-mode flow: the operator captures the envelope JSON via
//! `curl http://prod/__ops > ops.json` and pipes it through
//! this command with the deploy public key. The verifier fails
//! closed on signature mismatch, payload tampering, or wrong
//! payload-type. On success the parsed snapshot prints as
//! pretty JSON.

use anyhow::{anyhow, Context, Result};
use corvid_abi::load_verifying_key;
use corvid_runtime::ops_show::verify_ops_snapshot;
use std::path::Path;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose CLI surface
/// lives in `run_ops_show` below (delegating to the canonical
/// runtime verifier).
#[allow(dead_code)]
pub const GUARANTEE_ID_OPS_LIVE_INTROSPECTION_SIGNED: &str =
    "ops.live_introspection_signed";

/// Verify the envelope at `envelope_file_path` against the
/// public key at `pubkey_path` and print the parsed snapshot
/// as pretty JSON to stdout. Returns exit code 0 on success;
/// the caller maps the typed Err into a non-zero exit.
pub fn run_ops_show(envelope_file_path: &Path, pubkey_path: &Path) -> Result<u8> {
    let envelope_bytes = std::fs::read(envelope_file_path).with_context(|| {
        format!("read ops envelope file `{}`", envelope_file_path.display())
    })?;
    let verifying_key = load_verifying_key(pubkey_path)
        .map_err(|e| anyhow!("load verifying key from `{}`: {e}", pubkey_path.display()))?;
    let snapshot = verify_ops_snapshot(&envelope_bytes, &verifying_key)
        .map_err(|e| anyhow!("ops show verification failed: {e}"))?;
    let pretty = serde_json::to_string_pretty(&serde_json::json!({
        "build_id": snapshot.build_id,
        "started_unix_ms": snapshot.started_unix_ms,
        "generated_unix_ms": snapshot.generated_unix_ms,
        "request_count": snapshot.request_count,
        "claim_manifest_ids": snapshot.claim_manifest_ids,
        "verification": "signature-verified",
    }))?;
    println!("{pretty}");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_runtime::ops_show::{sign_ops_snapshot, OpsShowSnapshot};
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;

    fn snapshot() -> OpsShowSnapshot {
        OpsShowSnapshot {
            build_id: "git:beef1234".to_string(),
            started_unix_ms: 1_700_000_000_000,
            generated_unix_ms: 1_700_000_360_000,
            request_count: 7,
            claim_manifest_ids: vec!["auth.csrf_double_submit".to_string()],
        }
    }

    fn write_envelope(dir: &Path, key: &SigningKey) -> std::path::PathBuf {
        let envelope = sign_ops_snapshot(&snapshot(), key, "deploy-key").unwrap();
        let json = serde_json::to_vec(&envelope).unwrap();
        let path = dir.join("ops.json");
        std::fs::write(&path, json).unwrap();
        path
    }

    fn write_pubkey(dir: &Path, key: &SigningKey) -> std::path::PathBuf {
        let path = dir.join("deploy.pub");
        let hex = hex::encode(key.verifying_key().as_bytes());
        std::fs::write(&path, hex).unwrap();
        path
    }

    /// Slice 43P-LR (positive, end-to-end): the CLI verifies an
    /// envelope signed by the matching key and reports success.
    /// File-mode is the v1.0 surface; operators capture via
    /// curl + verify via this CLI subcommand.
    #[test]
    fn ops_show_verifies_envelope_signed_with_matching_key() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let envelope_path = write_envelope(dir.path(), &key);
        let pubkey_path = write_pubkey(dir.path(), &key);
        let code = run_ops_show(&envelope_path, &pubkey_path).unwrap();
        assert_eq!(code, 0);
    }

    /// Slice 43P-LR (adversarial, MITM): an envelope signed
    /// with a different key — exactly the man-in-the-middle
    /// shape the registry row's description names — fails the
    /// CLI's verification with a typed error mentioning the
    /// signature.
    #[test]
    fn ops_show_refuses_envelope_signed_with_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let server_key = SigningKey::generate(&mut OsRng);
        let attacker_key = SigningKey::generate(&mut OsRng);
        let envelope_path = write_envelope(dir.path(), &attacker_key);
        let pubkey_path = write_pubkey(dir.path(), &server_key);
        let err = run_ops_show(&envelope_path, &pubkey_path)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("verification failed") && err.contains("signature"),
            "expected signature-verify failure, got: {err}"
        );
    }

    /// Slice 43P-LR (adversarial, malformed envelope): a
    /// non-JSON envelope file surfaces a typed error referencing
    /// the verification stage rather than crashing or silently
    /// succeeding.
    #[test]
    fn ops_show_refuses_malformed_envelope_file() {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::generate(&mut OsRng);
        let envelope_path = dir.path().join("ops.json");
        std::fs::write(&envelope_path, "not a dsse envelope").unwrap();
        let pubkey_path = write_pubkey(dir.path(), &key);
        let err = run_ops_show(&envelope_path, &pubkey_path)
            .unwrap_err()
            .to_string();
        assert!(err.contains("verification failed"), "got: {err}");
    }
}
