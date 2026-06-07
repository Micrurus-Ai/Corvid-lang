//! 33Q13e end-to-end acceptance — `corvid upgrade assist`.
//!
//! Pins three properties of the upgrade-readiness audit:
//!
//! 1. **Canonical-source clean**: a Corvid source using ONLY
//!    canonical trust + data values and proper @budget annotations
//!    produces zero `assist` findings. The audit cannot false-
//!    positive — same groundedness contract 33Q13a + 33Q13c pin.
//! 2. **Non-canonical detection**: a source using a non-canonical
//!    `trust:` or `data:` value MUST surface the 33Q7b warn
//!    finding with the correct `rule_id` + line citation.
//! 3. **No-false-positive on struct field declarations**: a Corvid
//!    source declaring `type Foo: trust: String` (struct field of
//!    type `String`, NOT a dimension value) MUST NOT trip the
//!    non-canonical-trust rule. This was the load-bearing false
//!    positive surfaced during live verification — fixed by the
//!    `parse_dimension_value` uppercase-skip guard.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

fn assist_json(source_path: &PathBuf) -> serde_json::Value {
    let output = Command::new(corvid_bin())
        .arg("upgrade")
        .arg("assist")
        .arg(source_path)
        .arg("--json")
        .current_dir(repo_root())
        .output()
        .expect("spawn corvid upgrade assist");
    assert!(
        output.status.success(),
        "upgrade assist exited non-zero. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("assist JSON utf-8");
    serde_json::from_str(&stdout)
        .unwrap_or_else(|err| panic!("JSON parse failed: {err}\n--- stdout ---\n{stdout}"))
}

/// Property 1: a Corvid source using only canonical values produces
/// zero `assist` findings. This is the no-false-positive baseline.
#[test]
fn upgrade_assist_produces_zero_findings_for_canonical_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("main.cor");
    let source = r#"effect refund_money:
    cost: $0.50
    trust: human_required
    data: financial

tool refund(user_id: String, amount: Float) -> Bool dangerous uses refund_money

@budget($1.00)
@trust(human_required)
agent issue_refund(user_id: String, amount: Float) -> Bool uses refund_money:
    approve Refund(user_id, amount)
    return refund(user_id, amount)
"#;
    std::fs::write(&src, source).unwrap();

    let report = assist_json(&src);
    let findings = report["findings"].as_array().expect("findings array");
    assert!(
        findings.is_empty(),
        "canonical source MUST produce 0 findings (no false positives); got {} findings: {findings:?}",
        findings.len()
    );
    assert_eq!(
        report["scanned_files"].as_u64().expect("u64"),
        1,
        "expected exactly 1 scanned file"
    );
}

/// Property 2: a source using non-canonical trust/data values
/// surfaces 33Q7b warnings with the correct rule_id + line.
#[test]
fn upgrade_assist_detects_non_canonical_trust_and_data_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("main.cor");
    // `trust: bounded` is one of the 33Q7a-cataloged reference-app
    // extensions — non-canonical but accepted today. `data: customer`
    // is the same pattern for data dimension.
    let source = r#"effect customer_lookup:
    cost: $0.01
    trust: bounded
    data: customer

tool lookup(user_id: String) -> String uses customer_lookup

agent fetch(user_id: String) -> String uses customer_lookup:
    return lookup(user_id)
"#;
    std::fs::write(&src, source).unwrap();

    let report = assist_json(&src);
    let findings = report["findings"].as_array().expect("findings array");

    let trust_finding = findings.iter().find(|f| {
        f["rule_id"].as_str() == Some("trust.custom_value")
    });
    let data_finding = findings.iter().find(|f| {
        f["rule_id"].as_str() == Some("data.custom_value")
    });

    assert!(
        trust_finding.is_some(),
        "MUST surface trust.custom_value for `trust: bounded` (one of the \
         33Q7a-cataloged reference-app extensions). findings={findings:?}"
    );
    assert!(
        data_finding.is_some(),
        "MUST surface data.custom_value for `data: customer`. \
         findings={findings:?}"
    );

    // Citation correctness: the line numbers in the findings must
    // match the actual lines in the source (3 for trust, 4 for data).
    let trust = trust_finding.unwrap();
    assert_eq!(
        trust["line"].as_u64().expect("u64"),
        3,
        "trust finding must cite line 3 (where `trust: bounded` lives). \
         finding={trust:?}"
    );
    let data = data_finding.unwrap();
    assert_eq!(
        data["line"].as_u64().expect("u64"),
        4,
        "data finding must cite line 4. finding={data:?}"
    );

    // Severity is `warn` (not `critical`) because today it
    // compiles; the migration is post-v1.0.
    assert_eq!(trust["severity"].as_str(), Some("warn"));
    assert_eq!(data["severity"].as_str(), Some("warn"));
}

/// Property 3 (LOAD-BEARING — the false positive caught during
/// live verification): a struct field declaration with type
/// `String` MUST NOT trip the non-canonical-trust rule. The
/// `parse_dimension_value` uppercase-skip guard is the structural
/// fix; this test pins it.
#[test]
fn upgrade_assist_does_not_false_positive_on_struct_field_declarations() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("main.cor");
    // This is the exact shape `std/effects.cor` declares — struct
    // fields with type `String` whose name happens to be `trust`
    // or `data`. Pre-fix, the assist analyzer matched these as
    // non-canonical dimension values and emitted false findings
    // like `trust: String will require corvid.toml`.
    let source = r#"public type EffectTag:
    name: String
    trust: String
    data: String
    replay_policy: String

public type EffectEnvelope:
    effect_name: String
    trust: String
    data: String
"#;
    std::fs::write(&src, source).unwrap();

    let report = assist_json(&src);
    let findings = report["findings"].as_array().expect("findings array");

    for f in findings {
        let rule_id = f["rule_id"].as_str().expect("rule_id");
        let title = f["title"].as_str().expect("title");
        assert!(
            !["trust.custom_value", "data.custom_value"].contains(&rule_id),
            "struct field declaration MUST NOT trip dimension-value lints. \
             False finding: rule_id=`{rule_id}` title=`{title}`. The \
             `parse_dimension_value` uppercase-skip guard is the structural \
             fix; if this test fails, that guard regressed."
        );
    }
}
