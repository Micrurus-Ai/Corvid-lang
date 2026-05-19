//! Schema-agnostic JSON contract-drift detector.
//!
//! `detect_contract_drift(baseline, observed)` walks both JSON
//! trees in lockstep and reports every path where the observed
//! response differs from the baseline:
//!
//!   - `added_paths` — fields present in `observed` that are not
//!     in `baseline`. Compatible additions from the provider's
//!     perspective; an integrating connector may still want to
//!     adopt them, but no caller breaks.
//!   - `removed_paths` — fields present in `baseline` that are
//!     missing from `observed`. A connector that consumed this
//!     field is now broken — the central drift threat.
//!   - `type_changed_paths` — fields present in both trees but
//!     whose JSON type differs (e.g. `number` → `string` is a
//!     classic provider-side breakage).
//!
//! The detector is pure (no I/O, no env reads) and deterministic
//! over the JSON-path order of its outputs. The caller picks
//! what "baseline" means: a recorded mock fixture, the previous
//! successful live run's response, a hand-authored expected
//! shape, etc.
//!
//! This is the unit of correctness behind `corvid connectors
//! check --baseline <file> --observed <file>`. The live-HTTP
//! fetch path that would compute `observed` from a real
//! provider call is operational gating — see
//! `35V2-P41-E-LR-live-provider-ci-matrix` for that operational
//! slice. Today the detector ships + the CLI wires it for the
//! file-input flow, which is enough to enforce the contract
//! against any pre-recorded payload pair in CI.

use serde_json::Value;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `detect_contract_drift` below.
#[allow(dead_code)]
pub const GUARANTEE_ID_CONTRACT_DRIFT_DETECTED: &str = "connector.contract_drift_detected";

/// Drift report. `added`, `removed`, and `type_changed` paths
/// are each sorted lexicographically so two runs against the
/// same input produce byte-identical output (useful for golden
/// fixtures + diff-friendly review).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractDriftReport {
    pub added_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub type_changed_paths: Vec<TypeChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeChange {
    pub path: String,
    pub baseline_type: String,
    pub observed_type: String,
}

impl ContractDriftReport {
    /// True iff every drift bucket is empty. The caller uses
    /// this to decide the exit code of `corvid connectors
    /// check --baseline ... --observed ...`.
    pub fn is_empty(&self) -> bool {
        self.added_paths.is_empty()
            && self.removed_paths.is_empty()
            && self.type_changed_paths.is_empty()
    }

    /// Total number of drift sites — sum of all three buckets.
    pub fn total(&self) -> usize {
        self.added_paths.len() + self.removed_paths.len() + self.type_changed_paths.len()
    }
}

/// Compute the structural drift between `baseline` and
/// `observed`. See module doc for the three drift buckets.
pub fn detect_contract_drift(baseline: &Value, observed: &Value) -> ContractDriftReport {
    let mut report = ContractDriftReport::default();
    walk(&mut report, "$", baseline, observed);
    report.added_paths.sort();
    report.removed_paths.sort();
    report.type_changed_paths.sort_by(|a, b| a.path.cmp(&b.path));
    report
}

fn walk(report: &mut ContractDriftReport, path: &str, baseline: &Value, observed: &Value) {
    if !same_kind(baseline, observed) {
        report.type_changed_paths.push(TypeChange {
            path: path.to_string(),
            baseline_type: json_type_name(baseline).to_string(),
            observed_type: json_type_name(observed).to_string(),
        });
        return;
    }
    match (baseline, observed) {
        (Value::Object(b), Value::Object(o)) => {
            for (key, b_val) in b {
                let child_path = format!("{path}.{key}");
                match o.get(key) {
                    Some(o_val) => walk(report, &child_path, b_val, o_val),
                    None => report.removed_paths.push(child_path),
                }
            }
            for key in o.keys() {
                if !b.contains_key(key) {
                    report.added_paths.push(format!("{path}.{key}"));
                }
            }
        }
        (Value::Array(b), Value::Array(o)) => {
            // For drift purposes, an array's *shape* is the
            // shape of its first element (the typical
            // collection-of-records pattern). Empty arrays on
            // either side are treated as "no shape evidence"
            // and skipped — the caller can pick whether
            // emptiness itself is meaningful at the row level.
            if let (Some(b_first), Some(o_first)) = (b.first(), o.first()) {
                walk(report, &format!("{path}[0]"), b_first, o_first);
            }
        }
        _ => {
            // Primitive types matched on `same_kind`. We do not
            // compare values — the detector reports
            // *structural* drift, not data drift.
        }
    }
}

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!(
        (a, b),
        (Value::Null, Value::Null)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
    )
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn identical_shapes_produce_empty_drift_report() {
        let baseline = json!({"id": "msg-1", "subject": "hi", "count": 3});
        let observed = json!({"id": "msg-2", "subject": "yo", "count": 7});
        let report = detect_contract_drift(&baseline, &observed);
        assert!(report.is_empty());
        assert_eq!(report.total(), 0);
    }

    /// Slice 35V2-P41-D-LR (positive, provider-added field):
    /// the provider has added a field that wasn't in the
    /// baseline. The detector surfaces it under `added_paths` so
    /// the connector author can decide whether to consume it,
    /// but the run does not have to fail.
    #[test]
    fn provider_added_field_appears_in_added_paths() {
        let baseline = json!({"id": "m", "subject": "hi"});
        let observed = json!({"id": "m", "subject": "hi", "snippet": "new"});
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.added_paths, vec!["$.snippet".to_string()]);
        assert!(report.removed_paths.is_empty());
        assert!(report.type_changed_paths.is_empty());
    }

    /// Slice 35V2-P41-D-LR (adversarial, provider-removed
    /// field): the provider has removed a field the connector
    /// consumes. The detector surfaces it under `removed_paths`
    /// — the central drift threat. CI runs that pipe through
    /// `corvid connectors check --baseline ... --observed ...`
    /// exit non-zero and fail the build.
    #[test]
    fn provider_removed_field_appears_in_removed_paths_central_threat() {
        let baseline = json!({"id": "m", "subject": "hi", "thread_id": "t-1"});
        let observed = json!({"id": "m", "subject": "hi"});
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.removed_paths, vec!["$.thread_id".to_string()]);
        assert!(!report.is_empty());
    }

    /// Slice 35V2-P41-D-LR (adversarial, type change):
    /// the provider changed a field's type. Classic case: a
    /// numeric id silently becomes a string. The connector that
    /// consumed it with the wrong type now breaks at parse
    /// time; the detector catches it before deployment.
    #[test]
    fn provider_type_change_appears_in_type_changed_paths() {
        let baseline = json!({"id": 42, "subject": "hi"});
        let observed = json!({"id": "42", "subject": "hi"});
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.type_changed_paths.len(), 1);
        let change = &report.type_changed_paths[0];
        assert_eq!(change.path, "$.id");
        assert_eq!(change.baseline_type, "number");
        assert_eq!(change.observed_type, "string");
        assert!(!report.is_empty());
    }

    /// Nested-object drift: the detector walks recursively and
    /// reports every drift site as a dotted JSON path so an
    /// auditor can navigate straight to the broken field.
    #[test]
    fn nested_object_drift_reports_dotted_json_paths() {
        let baseline = json!({
            "result": {"id": "m", "headers": {"from": "a", "to": "b"}},
            "next_page": "p1",
        });
        let observed = json!({
            "result": {"id": "m", "headers": {"from": "a"}, "extra": 1},
            "next_page": "p1",
        });
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.removed_paths, vec!["$.result.headers.to".to_string()]);
        assert_eq!(report.added_paths, vec!["$.result.extra".to_string()]);
    }

    /// Array of records: the detector uses the first element's
    /// shape as the shape evidence (the typical "list of typed
    /// records" pattern). Drift in the first element shape
    /// surfaces under an indexed path.
    #[test]
    fn array_of_records_walks_first_element_shape() {
        let baseline = json!({"messages": [{"id": "a", "snippet": "x"}, {"id": "b", "snippet": "y"}]});
        let observed = json!({"messages": [{"id": "a"}, {"id": "b"}]});
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.removed_paths, vec!["$.messages[0].snippet".to_string()]);
    }

    /// Empty arrays on either side are skipped — the caller
    /// decides whether emptiness itself is meaningful. This
    /// prevents a transient empty result-set from looking like
    /// "every field was removed."
    #[test]
    fn empty_arrays_skip_shape_walking() {
        let baseline = json!({"messages": [{"id": "a"}]});
        let observed = json!({"messages": []});
        let report = detect_contract_drift(&baseline, &observed);
        assert!(report.is_empty());
    }

    /// Slice 35V2-P41-D-LR (adversarial, multiple drift sites
    /// in one diff): all three drift buckets fire in one
    /// comparison. The report sorts every bucket so the output
    /// is deterministic over insertion order — useful for
    /// golden-fixture diffs in CI.
    #[test]
    fn multiple_drift_sites_sorted_for_deterministic_output() {
        let baseline = json!({
            "alpha": 1,
            "beta": {"x": 1, "y": 2},
            "gamma": "g",
        });
        let observed = json!({
            "alpha": "1",          // type change
            "beta": {"x": 1},      // removed: beta.y
            "delta": "added",      // added: delta
        });
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.added_paths, vec!["$.delta".to_string()]);
        // `$.gamma` removed at root + `$.beta.y` removed nested
        // — both surface, sorted.
        assert_eq!(
            report.removed_paths,
            vec!["$.beta.y".to_string(), "$.gamma".to_string()]
        );
        assert_eq!(report.type_changed_paths.len(), 1);
        assert_eq!(report.type_changed_paths[0].path, "$.alpha");
        assert_eq!(report.total(), 4);
    }

    #[test]
    fn null_vs_non_null_is_a_type_change_not_a_missing_field() {
        let baseline = json!({"id": "m", "subject": null});
        let observed = json!({"id": "m", "subject": "hi"});
        let report = detect_contract_drift(&baseline, &observed);
        assert_eq!(report.type_changed_paths.len(), 1);
        assert_eq!(report.type_changed_paths[0].path, "$.subject");
        assert_eq!(report.type_changed_paths[0].baseline_type, "null");
        assert_eq!(report.type_changed_paths[0].observed_type, "string");
    }
}
