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

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `narrate_drift_report` below.
#[allow(dead_code)]
pub const GUARANTEE_ID_DRIFT_NARRATION_GROUNDED: &str = "connector.drift_narration_grounded";

/// One human-readable explanation of a single drift site,
/// paired with the back-references that grounded the
/// narration. The CLI renders this for an operator who is
/// reviewing why CI flagged a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriftNarration {
    /// JSON path of the drift site (e.g. `$.result.thread_id`).
    pub path: String,
    /// Drift kind: `removed`, `added`, or `type_changed`. The
    /// CLI uses this to colour/group output.
    pub kind: String,
    /// One-line explanation naming the operational consequence
    /// in concrete terms ("connector code that consumed this
    /// field is now broken at parse time").
    pub consequence: String,
    /// Severity for triage. `breaking` for sites that fail a
    /// running connector immediately; `compatible` for additive
    /// changes the connector can choose to consume.
    pub severity: String,
    /// Grounded<T> shape: back-references to the structured
    /// drift evidence the narration summarised. Today the
    /// sources name the original report fields; downstream
    /// they could carry trace ids or audit-event ids when the
    /// narrator is wired to a live system.
    pub sources: Vec<String>,
}

/// Narrate every drift site in `report`. The narration order
/// matches the report's sorted order so the output is
/// deterministic + diff-friendly in CI. Empty reports produce
/// an empty narration vec — caller decides whether the
/// no-drift case warrants a "no drift detected" notice.
///
/// Each `DriftNarration::sources` entry names the report bucket
/// + the drift path it was synthesised from, so an auditor can
/// trace every claim in the narration back to a structural
/// evidence row. The narrator is deterministic + LLM-free; the
/// "RAG-grounded" framing in the slice description means the
/// output cites which detection evidence supports the claim,
/// not that an LLM round-trip is involved.
pub fn narrate_drift_report(report: &ContractDriftReport) -> Vec<DriftNarration> {
    let mut narrations = Vec::with_capacity(report.total());
    for path in &report.removed_paths {
        narrations.push(DriftNarration {
            path: path.clone(),
            kind: "removed".to_string(),
            consequence: format!(
                "field `{path}` is missing from the observed response \
                 — connector code that consumed it now fails at \
                 deserialization"
            ),
            severity: "breaking".to_string(),
            sources: vec![format!("removed_paths::{path}")],
        });
    }
    for change in &report.type_changed_paths {
        narrations.push(DriftNarration {
            path: change.path.clone(),
            kind: "type_changed".to_string(),
            consequence: format!(
                "field `{}` changed type from `{}` to `{}` — \
                 connector deserialization will fail until the \
                 expected type is updated to match",
                change.path, change.baseline_type, change.observed_type
            ),
            severity: "breaking".to_string(),
            sources: vec![format!(
                "type_changed_paths::{}::baseline={}::observed={}",
                change.path, change.baseline_type, change.observed_type
            )],
        });
    }
    for path in &report.added_paths {
        narrations.push(DriftNarration {
            path: path.clone(),
            kind: "added".to_string(),
            consequence: format!(
                "field `{path}` is new in the observed response \
                 — existing connector code is unaffected, but \
                 the connector may want to start consuming it"
            ),
            severity: "compatible".to_string(),
            sources: vec![format!("added_paths::{path}")],
        });
    }
    narrations
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

    /// Slice 35V2-P41-H-LR (positive, empty input): an empty
    /// drift report yields an empty narration vec — the
    /// narrator is a pure projection.
    #[test]
    fn drift_narration_for_empty_report_is_empty() {
        let report = ContractDriftReport::default();
        let narrations = narrate_drift_report(&report);
        assert!(narrations.is_empty());
    }

    /// Slice 35V2-P41-H-LR (positive, Grounded<T> contract):
    /// every narration cell has a non-empty `sources` array
    /// that back-references the structural evidence the
    /// narration summarised. The grounding property is the
    /// central guarantee — operators can audit-trail every
    /// claim back to a detector row.
    #[test]
    fn every_drift_narration_carries_grounded_sources() {
        let report = detect_contract_drift(
            &json!({"alpha": 1, "beta": {"x": 1, "y": 2}, "gamma": "g"}),
            &json!({"alpha": "1", "beta": {"x": 1}, "delta": "added"}),
        );
        assert!(!report.is_empty());
        let narrations = narrate_drift_report(&report);
        // Every drift site has exactly one narration; nothing
        // gets dropped silently.
        assert_eq!(narrations.len(), report.total());
        // Every narration is grounded.
        for narration in &narrations {
            assert!(!narration.sources.is_empty());
            // The source row names the bucket + the path so an
            // auditor can find the evidence in the report.
            let source = &narration.sources[0];
            assert!(
                source.starts_with("removed_paths::")
                    || source.starts_with("added_paths::")
                    || source.starts_with("type_changed_paths::")
            );
            assert!(
                source.contains(&narration.path),
                "narration source `{source}` does not back-reference path `{}`",
                narration.path
            );
        }
    }

    /// Slice 35V2-P41-H-LR (adversarial, breaking-severity
    /// classification): removed fields and type changes both
    /// surface as `breaking` severity. Added fields are
    /// `compatible`. An operator triaging the report needs the
    /// severity to be deterministic on the drift kind, not
    /// reordered or mis-labelled.
    #[test]
    fn drift_narration_classifies_breaking_versus_compatible() {
        let report = detect_contract_drift(
            &json!({"a": 1, "b": "x"}),
            &json!({"a": "1", "c": "added"}),
        );
        let narrations = narrate_drift_report(&report);
        let breakings: Vec<&DriftNarration> = narrations
            .iter()
            .filter(|n| n.severity == "breaking")
            .collect();
        let compatibles: Vec<&DriftNarration> = narrations
            .iter()
            .filter(|n| n.severity == "compatible")
            .collect();
        // type_change on $.a + removed $.b → 2 breaking
        assert_eq!(breakings.len(), 2);
        // added $.c → 1 compatible
        assert_eq!(compatibles.len(), 1);
        assert_eq!(compatibles[0].path, "$.c");
        assert_eq!(compatibles[0].kind, "added");
    }

    /// Slice 35V2-P41-H-LR (adversarial, removed-field
    /// consequence narration): the narration for a removed
    /// field names "deserialization" so the operator knows
    /// exactly what breaks. Vague wording ("something
    /// changed") would defeat the helper's purpose.
    #[test]
    fn removed_field_narration_names_deserialization_consequence() {
        let report = detect_contract_drift(
            &json!({"id": "m", "thread_id": "t-1"}),
            &json!({"id": "m"}),
        );
        let narrations = narrate_drift_report(&report);
        assert_eq!(narrations.len(), 1);
        let n = &narrations[0];
        assert_eq!(n.kind, "removed");
        assert_eq!(n.severity, "breaking");
        assert!(
            n.consequence.contains("deserialization"),
            "consequence should name deserialization: {}",
            n.consequence
        );
        assert!(n.consequence.contains("thread_id"));
    }

    /// Slice 35V2-P41-H-LR (positive, ordering invariant):
    /// the narration order is removed → type_changed → added
    /// (breaking first, compatible last). Operators read the
    /// most severe items first.
    #[test]
    fn drift_narration_orders_breaking_before_compatible() {
        let report = detect_contract_drift(
            &json!({"removed_field": 1, "changed_field": 1}),
            &json!({"changed_field": "1", "added_field": "new"}),
        );
        let narrations = narrate_drift_report(&report);
        // Find indices to assert ordering.
        let removed_idx = narrations
            .iter()
            .position(|n| n.kind == "removed")
            .unwrap();
        let changed_idx = narrations
            .iter()
            .position(|n| n.kind == "type_changed")
            .unwrap();
        let added_idx = narrations
            .iter()
            .position(|n| n.kind == "added")
            .unwrap();
        assert!(
            removed_idx < changed_idx && changed_idx < added_idx,
            "expected removed < type_changed < added, got: {narrations:?}"
        );
    }
}
