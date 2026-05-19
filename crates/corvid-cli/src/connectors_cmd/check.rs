//! `corvid connectors check [--live]` — manifest validation.
//!
//! Default mode validates every shipped manifest against the
//! manifest schema and reports per-connector diagnostics. The
//! `--live` mode is reserved for the live drift narrator that
//! compares manifest schema to a real provider response shape;
//! until that lands, `--live` returns an explicit `Err` directing
//! the caller to rerun without `--live` (per the Phase 20j roadmap
//! audit-correction track).

use anyhow::{anyhow, Context, Result};
use corvid_connector_runtime::{
    detect_contract_drift, validate_connector_manifest, ContractDriftReport,
};
use std::path::Path;

use super::support::shipped_manifests;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `run_contract_drift` below (delegating
/// to `corvid_connector_runtime::detect_contract_drift`).
#[allow(dead_code)]
pub const GUARANTEE_ID_CONTRACT_DRIFT_DETECTED: &str = "connector.contract_drift_detected";

/// Hermetic file-input mode for `corvid connectors check
/// --baseline <file> --observed <file>`. Loads both JSON
/// payloads and runs the schema-agnostic structural drift
/// detector. The CLI exits non-zero on any drift site; the
/// caller wires this into CI to fail builds when a captured
/// live response diverges from the recorded baseline.
pub fn run_contract_drift(
    baseline_path: &Path,
    observed_path: &Path,
) -> Result<ContractDriftReport> {
    let baseline_raw = std::fs::read_to_string(baseline_path)
        .with_context(|| format!("read baseline file `{}`", baseline_path.display()))?;
    let observed_raw = std::fs::read_to_string(observed_path)
        .with_context(|| format!("read observed file `{}`", observed_path.display()))?;
    let baseline: serde_json::Value = serde_json::from_str(&baseline_raw)
        .with_context(|| format!("parse baseline JSON in `{}`", baseline_path.display()))?;
    let observed: serde_json::Value = serde_json::from_str(&observed_raw)
        .with_context(|| format!("parse observed JSON in `{}`", observed_path.display()))?;
    Ok(detect_contract_drift(&baseline, &observed))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorCheckEntry {
    pub name: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

/// Validates every shipped connector manifest and returns one
/// `ConnectorCheckEntry` per connector. With `live = true` the
/// caller indicates real-provider drift detection should run; this
/// slice flags it as a deferred bounty-extension behaviour and the
/// per-connector live drift narrator lands in slice 41M alongside
/// the webhook receive end-to-end.
pub fn run_check(live: bool) -> Result<Vec<ConnectorCheckEntry>> {
    let manifests = shipped_manifests()?;
    let mut entries = Vec::with_capacity(manifests.len());
    for (name, manifest) in manifests {
        let report = validate_connector_manifest(&manifest);
        let diagnostics = report
            .diagnostics
            .iter()
            .map(|d| format!("{d}"))
            .collect::<Vec<_>>();
        entries.push(ConnectorCheckEntry {
            name: name.to_string(),
            valid: report.valid,
            diagnostics,
        });
    }
    if live && std::env::var("CORVID_PROVIDER_LIVE").as_deref() != Ok("1") {
        return Err(anyhow!(
            "`--live` requires `CORVID_PROVIDER_LIVE=1` plus per-provider \
             credentials — refusing to issue live drift probes without \
             explicit opt-in. For hermetic CI runs, capture the live \
             response separately and pass it as \
             `--baseline <recorded.json> --observed <captured.json>`; \
             the structural drift detector runs without any network call."
        ));
    }
    // Live-HTTP-fetch wiring (actually contact the real provider
    // + compute `observed` from the live response) is operational
    // gating, filed at `35V2-P41-E-LR-live-provider-ci-matrix`.
    // The structural drift detector ships today via the
    // `--baseline`/`--observed` file-input flow + the registry
    // row `connector.contract_drift_detected` is RuntimeChecked.
    if live {
        return Err(anyhow!(
            "Live-HTTP drift detection requires the per-provider CI matrix \
             in `35V2-P41-E-LR-live-provider-ci-matrix` (provider creds \
             must live in CI secrets). Today rerun with \
             `--baseline <file> --observed <file>` to exercise the \
             structural drift detector against captured payloads, or \
             omit `--live` for the manifest schema validation report."
        ));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice 41L: `corvid connectors check` flags every shipped
    /// manifest as valid (manifests are static and CI-tested
    /// elsewhere). With `--live` the command refuses without
    /// `CORVID_PROVIDER_LIVE=1`.
    #[test]
    fn check_passes_for_shipped_manifests() {
        let entries = run_check(false).expect("check");
        for entry in &entries {
            assert!(entry.valid, "{}: {:?}", entry.name, entry.diagnostics);
        }
    }

    /// Slice 35V2-P41-D-LR (positive, file-input flow):
    /// identical baseline + observed JSON files produce an
    /// empty drift report. The CLI exits 0 in this case.
    #[test]
    fn contract_drift_identical_files_report_no_drift() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = dir.path().join("baseline.json");
        let observed = dir.path().join("observed.json");
        let payload = r#"{"id":"m1","subject":"hi","count":3}"#;
        std::fs::write(&baseline, payload).unwrap();
        std::fs::write(&observed, payload).unwrap();
        let report = run_contract_drift(&baseline, &observed).unwrap();
        assert!(report.is_empty());
    }

    /// Slice 35V2-P41-D-LR (adversarial, removed field):
    /// the observed response is missing a field the baseline
    /// declared. The detector surfaces it under `removed_paths`
    /// and the report is non-empty — the CLI exits non-zero in
    /// production usage.
    #[test]
    fn contract_drift_removed_field_surfaces_with_non_empty_report() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = dir.path().join("baseline.json");
        let observed = dir.path().join("observed.json");
        std::fs::write(
            &baseline,
            r#"{"id":"m1","subject":"hi","thread_id":"t-1"}"#,
        )
        .unwrap();
        std::fs::write(&observed, r#"{"id":"m1","subject":"hi"}"#).unwrap();
        let report = run_contract_drift(&baseline, &observed).unwrap();
        assert!(!report.is_empty());
        assert_eq!(report.removed_paths, vec!["$.thread_id".to_string()]);
    }

    /// Slice 35V2-P41-D-LR (adversarial, malformed input): a
    /// non-JSON baseline file produces a typed error naming the
    /// offending file rather than a silent empty drift report.
    #[test]
    fn contract_drift_malformed_baseline_file_surfaces_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let baseline = dir.path().join("baseline.json");
        let observed = dir.path().join("observed.json");
        std::fs::write(&baseline, "not json at all").unwrap();
        std::fs::write(&observed, r#"{"id":"m1"}"#).unwrap();
        let err = run_contract_drift(&baseline, &observed)
            .unwrap_err()
            .to_string();
        assert!(err.contains("parse baseline JSON"), "got: {err}");
    }
}
