//! Signed live-ops introspection snapshot.
//!
//! `OpsShowSnapshot` is the payload `corvid ops show <url>`
//! consumes from a rendered backend's `/__ops` endpoint: the
//! binary's claim manifest + lightweight runtime counters
//! (`request_count`, `started_unix_ms`, `generated_unix_ms`) +
//! its self-identified `build_id`.
//!
//! The whole snapshot is wrapped in a DSSE v1 envelope signed
//! with the binary's ed25519 signing key (typically the same
//! key that signed the cdylib's ABI attestation). The CLI
//! verifies the envelope against an operator-supplied public
//! key — a mismatch means either a man-in-the-middle is
//! intercepting the call or the wrong binary is running at the
//! URL. Either way, the row's contract is upheld: the operator
//! never trusts the response without a matching signature.
//!
//! The canonical envelope payload type is `corvid.ops.show.v1`,
//! pinned through DSSE's payloadType allow-list so a signature
//! valid over a *different* artifact (an ABI attestation, a
//! receipt) cannot be replayed against the ops surface.

use corvid_abi::{sign_envelope, verify_envelope, DsseEnvelope};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `verify_ops_snapshot` below.
#[allow(dead_code)]
pub const GUARANTEE_ID_LIVE_INTROSPECTION_SIGNED: &str = "ops.live_introspection_signed";

/// DSSE payload type for `corvid ops show` snapshots. Pinned
/// so the verifier rejects a signature valid over a different
/// signed artifact (an ABI attestation, a receipt, an in-toto
/// statement) replayed against this surface.
pub const OPS_SHOW_PAYLOAD_TYPE: &str = "application/vnd.corvid.ops.show+json; version=1";

/// One snapshot of a live rendered backend's operational state.
/// Designed to be canonical-JSON serialisable so two backends
/// with the same observable state produce byte-identical
/// payloads + signatures (useful for golden-fixture tests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpsShowSnapshot {
    /// Free-form binary identifier (typically the git SHA + the
    /// release-channel tag the binary was built from). The
    /// operator compares this against the expected deployed
    /// build id.
    pub build_id: String,
    /// Unix-epoch milliseconds at which the binary process
    /// started.
    pub started_unix_ms: u64,
    /// Unix-epoch milliseconds at which this snapshot was
    /// captured.
    pub generated_unix_ms: u64,
    /// Total requests served since process start.
    pub request_count: u64,
    /// Claim-manifest rows the binary's embedded claim asserts
    /// (mirrors the `corvid claim --explain` output's id list).
    /// The CLI compares this against the expected manifest to
    /// detect a mis-deployed binary running at the URL.
    #[serde(default)]
    pub claim_manifest_ids: Vec<String>,
}

/// Errors surfaced by snapshot verification.
#[derive(Debug)]
pub enum OpsShowError {
    /// The DSSE envelope failed to parse or verify (signature
    /// mismatch, wrong key, malformed JSON, etc.).
    EnvelopeVerify(corvid_abi::VerifyError),
    /// The signed payload was valid DSSE but its inner JSON did
    /// not deserialise to an `OpsShowSnapshot`.
    SnapshotJson(serde_json::Error),
}

impl std::fmt::Display for OpsShowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnvelopeVerify(e) => write!(f, "ops snapshot envelope failed verification: {e}"),
            Self::SnapshotJson(e) => write!(f, "ops snapshot payload is not valid JSON: {e}"),
        }
    }
}

impl std::error::Error for OpsShowError {}

/// Canonical JSON serialisation of an `OpsShowSnapshot`. Sorts
/// object keys (top-level fields are already sorted by
/// `serde_json::to_value` for structs but
/// `claim_manifest_ids` is preserved in operator-declared
/// order). Returns the raw bytes the DSSE PAE wraps.
pub fn canonical_snapshot_bytes(snapshot: &OpsShowSnapshot) -> Result<Vec<u8>, serde_json::Error> {
    // `serde_json::to_vec` on a struct uses declaration order,
    // which is stable across runs. We do NOT use a key-sorting
    // canonicaliser because the ids vector intentionally
    // preserves operator-declared order (the snapshot is a
    // verbatim copy of `corvid claim --explain --json`'s id
    // list).
    serde_json::to_vec(snapshot)
}

/// Sign a snapshot. The returned envelope is what the rendered
/// backend's `/__ops` endpoint returns to clients.
pub fn sign_ops_snapshot(
    snapshot: &OpsShowSnapshot,
    key: &SigningKey,
    key_id: &str,
) -> Result<DsseEnvelope, serde_json::Error> {
    let payload = canonical_snapshot_bytes(snapshot)?;
    Ok(sign_envelope(&payload, OPS_SHOW_PAYLOAD_TYPE, key, key_id))
}

/// Verify an envelope (as returned by the rendered backend's
/// `/__ops` endpoint) and return the parsed snapshot on
/// success. Fails closed on:
///
///   - signature mismatch (wrong key, tampered payload)
///   - payloadType other than `corvid.ops.show.v1`
///   - malformed envelope JSON
///   - inner snapshot JSON does not deserialise
pub fn verify_ops_snapshot(
    envelope_json: &[u8],
    key: &VerifyingKey,
) -> Result<OpsShowSnapshot, OpsShowError> {
    let payload = verify_envelope(envelope_json, &[OPS_SHOW_PAYLOAD_TYPE], key)
        .map_err(OpsShowError::EnvelopeVerify)?;
    serde_json::from_slice(&payload).map_err(OpsShowError::SnapshotJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;

    fn sample_snapshot() -> OpsShowSnapshot {
        OpsShowSnapshot {
            build_id: "git:abcdef1234567890".to_string(),
            started_unix_ms: 1_700_000_000_000,
            generated_unix_ms: 1_700_000_360_000,
            request_count: 4_242,
            claim_manifest_ids: vec![
                "approval.dangerous_call_requires_token".to_string(),
                "auth.csrf_double_submit".to_string(),
                "connector.replay_quarantine".to_string(),
            ],
        }
    }

    fn fresh_key() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }

    /// Slice 43P-LR (positive): sign → verify produces the
    /// original snapshot byte-for-byte. The roundtrip is the
    /// central contract — operators trust the parsed snapshot
    /// only if the signature matches.
    #[test]
    fn ops_snapshot_round_trips_through_sign_then_verify() {
        let key = fresh_key();
        let snapshot = sample_snapshot();
        let envelope = sign_ops_snapshot(&snapshot, &key, "deploy-key-1").unwrap();
        let envelope_json = serde_json::to_vec(&envelope).unwrap();
        let recovered = verify_ops_snapshot(&envelope_json, &key.verifying_key()).unwrap();
        assert_eq!(recovered, snapshot);
    }

    /// Slice 43P-LR (adversarial, MITM): a snapshot signed with
    /// a different key fails verification. This is the
    /// man-in-the-middle case — the row's contract says "the
    /// response is signed by the binary's signing key"; a
    /// non-matching key is precisely what an interceptor would
    /// produce.
    #[test]
    fn ops_snapshot_signed_with_wrong_key_fails_verification() {
        let server_key = fresh_key();
        let attacker_key = fresh_key();
        assert_ne!(
            server_key.verifying_key().as_bytes(),
            attacker_key.verifying_key().as_bytes()
        );
        let envelope = sign_ops_snapshot(&sample_snapshot(), &attacker_key, "attacker").unwrap();
        let envelope_json = serde_json::to_vec(&envelope).unwrap();
        let err = verify_ops_snapshot(&envelope_json, &server_key.verifying_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("signature"), "got: {err}");
    }

    /// Slice 43P-LR (adversarial, payload tampering): mutating
    /// the snapshot JSON after signing breaks the signature.
    /// Operators cannot trust a snapshot whose request_count or
    /// claim_manifest_ids was edited in transit.
    #[test]
    fn ops_snapshot_tampered_payload_fails_verification() {
        let key = fresh_key();
        let mut envelope = sign_ops_snapshot(&sample_snapshot(), &key, "k").unwrap();
        // Re-encode a different payload under the same
        // payloadType + same signature — exactly what an
        // attacker who intercepts the response would try.
        let tampered = OpsShowSnapshot {
            request_count: 999_999,
            ..sample_snapshot()
        };
        envelope.payload = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &canonical_snapshot_bytes(&tampered).unwrap(),
        );
        let envelope_json = serde_json::to_vec(&envelope).unwrap();
        let err = verify_ops_snapshot(&envelope_json, &key.verifying_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("signature"), "got: {err}");
    }

    /// Slice 43P-LR (adversarial, payload-type replay): a
    /// signature valid over a *different* DSSE artifact (e.g.
    /// an ABI attestation) cannot be replayed against the ops
    /// surface. The verifier's payload-type allow-list catches
    /// this even when the underlying ed25519 signature itself
    /// is mathematically valid.
    #[test]
    fn ops_snapshot_refuses_envelope_with_wrong_payload_type() {
        let key = fresh_key();
        // Sign the same bytes as a NON-ops payload-type.
        let wrong_type_envelope = sign_envelope(
            b"some other artifact",
            "application/vnd.corvid.abi.attestation+json; version=1",
            &key,
            "k",
        );
        let envelope_json = serde_json::to_vec(&wrong_type_envelope).unwrap();
        let err = verify_ops_snapshot(&envelope_json, &key.verifying_key())
            .unwrap_err()
            .to_string();
        assert!(err.contains("payload type"), "got: {err}");
    }

    /// Two snapshots with the same observable state produce
    /// byte-identical canonical payloads — a useful invariant
    /// for golden-fixture tests + diff-friendly review.
    #[test]
    fn canonical_snapshot_bytes_deterministic_over_identical_state() {
        let a = sample_snapshot();
        let b = sample_snapshot();
        assert_eq!(
            canonical_snapshot_bytes(&a).unwrap(),
            canonical_snapshot_bytes(&b).unwrap()
        );
    }
}
