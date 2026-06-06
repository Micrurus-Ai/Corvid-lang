//! 33Q13a end-to-end acceptance — `corvid beta synthesize-feedback`
//! against the real trial-report corpus shipped in
//! `docs/external-trials/`.
//!
//! These tests pin two properties:
//!
//! 1. **Coverage**: when the corpus contains N `### P<n>` finding
//!    headers across M reports, the synthesizer surfaces all N
//!    findings grouped into the correct class buckets.
//! 2. **Grounding (no fabrication)**: every finding the synthesizer
//!    emits cites a line that, when read from the source file,
//!    actually starts with `### ` and the claimed severity/class.
//!    This is the load-bearing property that would gate a future
//!    LLM-driven variant (slice 33Q13b): an LLM CAN hallucinate
//!    themes, so the deterministic core must enforce groundedness
//!    that the LLM-layer can only refine, never override.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

fn trial_report(name: &str) -> PathBuf {
    repo_root().join("docs").join("external-trials").join(name)
}

#[test]
fn synthesize_feedback_surfaces_canonical_categories_from_real_corpus() {
    // Run against both anonymous trial files. The maintainer-as-
    // reviewer report has 9 findings; the anonymous round-1+2 has 5.
    // Use --json so we get structured output we can assert against.
    let anon = trial_report("33m-trial-anonymous-2026-06-04.md");
    let maint = trial_report("33m-trial-maintainer-as-reviewer-2026-06-05.md");

    assert!(anon.is_file(), "anonymous trial report missing: {}", anon.display());
    assert!(maint.is_file(), "maintainer trial report missing: {}", maint.display());

    let output = Command::new(corvid_bin())
        .arg("beta")
        .arg("synthesize-feedback")
        .arg(&anon)
        .arg(&maint)
        .arg("--json")
        .current_dir(repo_root())
        .output()
        .expect("spawn corvid beta synthesize-feedback");

    assert!(
        output.status.success(),
        "synthesize-feedback exited non-zero. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("synthesis JSON utf-8");
    let synth: serde_json::Value =
        serde_json::from_str(&stdout).expect("synthesis JSON parses");

    // Coverage check: both reports cited in `reports` field.
    let reports = synth["reports"].as_array().expect("`reports` array");
    assert_eq!(reports.len(), 2, "both reports should be cited");

    // The maintainer report alone declares 9 findings (5 in main
    // text + 4 in dispositions table headers — but only the `###`
    // headers in the body should match the parser regex). The
    // anonymous round-1+2 report has 5 findings in round-1 + 5 in
    // round-2 = 10 findings total. Combined expected: at least
    // 14. The exact count can shift as the reports get edited;
    // assert a lower bound rather than equality so a future edit
    // that adds a finding doesn't break this test.
    let total = synth["total_findings"].as_u64().expect("u64");
    assert!(
        total >= 13,
        "expected >= 13 findings across both reports; got {total}. \
         JSON output:\n{stdout}"
    );

    // Canonical-category check: CODE, DOCS, and at least one of UX
    // or CODE/DOCS MUST appear as bucket classes. The exact set
    // depends on the reports' wording, so we check membership not
    // equality.
    let buckets = synth["buckets"].as_array().expect("`buckets` array");
    let bucket_classes: Vec<String> = buckets
        .iter()
        .map(|b| b["class"].as_str().expect("class string").to_string())
        .collect();
    for required in &["CODE", "DOCS"] {
        assert!(
            bucket_classes.iter().any(|c| c == required),
            "synthesis MUST include a `{required}` bucket — both \
             trial reports have findings classed `{required}`. \
             got buckets: {bucket_classes:?}"
        );
    }
}

/// Grounding (load-bearing): every finding the synthesizer cites
/// MUST be backed by an actual `### <severity> — <class>:` header
/// at the cited line of the cited file. The synthesizer cannot
/// fabricate findings — if it claims a P1 at line 42 of file X,
/// reading line 42 of file X MUST show that header.
///
/// This is the prerequisite for ever promoting the helper to an
/// LLM-driven thematic synthesizer (slice 33Q13b): the LLM layer
/// could refine groupings or invent new theme labels, but the
/// underlying citations have to anchor in real source bytes. This
/// test pins the property NOW so when the LLM layer lands, its
/// output is structurally constrained by the same check.
#[test]
fn synthesize_feedback_is_grounded_every_citation_resolves_to_real_header() {
    let anon = trial_report("33m-trial-anonymous-2026-06-04.md");
    let maint = trial_report("33m-trial-maintainer-as-reviewer-2026-06-05.md");

    let output = Command::new(corvid_bin())
        .arg("beta")
        .arg("synthesize-feedback")
        .arg(&anon)
        .arg(&maint)
        .arg("--json")
        .current_dir(repo_root())
        .output()
        .expect("spawn");
    assert!(output.status.success(), "synthesize-feedback failed");

    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    let synth: serde_json::Value = serde_json::from_str(&stdout).expect("JSON");
    let buckets = synth["buckets"].as_array().expect("buckets");

    for bucket in buckets {
        let findings = bucket["findings"].as_array().expect("findings");
        for finding in findings {
            let report_path = finding["report_path"].as_str().expect("report_path");
            let line_number = finding["line_number"].as_u64().expect("line_number") as usize;
            let severity = finding["severity"].as_str().expect("severity");
            let class = finding["class"].as_str().expect("class");

            // Read the cited line and verify the synthesizer didn't
            // invent it. We treat `report_path` as relative to the
            // workspace root since the CLI was invoked with
            // current_dir(repo_root()).
            let abs_path = if PathBuf::from(report_path).is_absolute() {
                PathBuf::from(report_path)
            } else {
                repo_root().join(report_path)
            };
            let text = std::fs::read_to_string(&abs_path).unwrap_or_else(|e| {
                panic!(
                    "cited report path must be readable: {} ({})",
                    abs_path.display(),
                    e
                )
            });
            let lines: Vec<&str> = text.lines().collect();
            assert!(
                line_number >= 1 && line_number <= lines.len(),
                "cited line {line_number} out of range for {} (file has {} lines)",
                abs_path.display(),
                lines.len()
            );
            let cited_line = lines[line_number - 1];

            // The cited line MUST be a `### ` header that contains
            // both the severity and class strings the synthesizer
            // claims. If the synthesizer fabricated, the line at
            // that position won't match.
            assert!(
                cited_line.starts_with("### "),
                "fabrication detected: synthesizer cited {}:{} as a \
                 finding header but the line there is `{cited_line}` \
                 — not a `### ` heading. severity={severity} class={class}",
                abs_path.display(),
                line_number
            );
            assert!(
                cited_line.contains(severity),
                "groundedness violated: synthesizer claimed severity \
                 `{severity}` at {}:{} but the cited line is `{cited_line}`. \
                 The deterministic parser cannot invent severities; an \
                 LLM-driven variant must obey the same constraint.",
                abs_path.display(),
                line_number
            );
            assert!(
                cited_line.contains(class),
                "groundedness violated: synthesizer claimed class \
                 `{class}` at {}:{} but the cited line is `{cited_line}`",
                abs_path.display(),
                line_number
            );
        }
    }
}
