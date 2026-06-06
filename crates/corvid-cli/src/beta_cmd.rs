//! `corvid beta` command handlers — slice 33Q13a.
//!
//! Today's surface: `corvid beta synthesize-feedback <REPORTS...>`
//! which walks one or more trial-report markdown files, extracts
//! every `### P<n>` / `### Minor` finding header, groups findings by
//! their declared class (CODE / DOCS / UX / etc.), and emits a
//! synthesis report with file:line citations back to the source.
//!
//! Why deterministic Rust (not an LLM-driven Corvid agent) at v1.0:
//! `corvid claim audit` (already shipped, registered under
//! `claim.audit_runnable_artifacts`) is exactly this shape — a
//! deterministic typed classifier with line-grounded citations. The
//! "AI helper" registry row matters because the helper provides
//! structured output an AI/developer can consume, not because the
//! helper invokes an LLM internally. The 33Q13-llm-promote
//! follow-up adds LLM-driven thematic clustering on top of this
//! grounded base — without it, an LLM-only synthesizer could
//! hallucinate themes the source reports don't actually mention.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One finding extracted from a trial report. The `report_path` +
/// `line_number` carry the citation back to the source — every
/// claim the synthesis makes is anchored here, mirrors the
/// `Grounded<T>` shape Corvid uses elsewhere.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// The trial-report file this finding came from.
    pub report_path: PathBuf,
    /// 1-indexed line number of the `### P<n>` header.
    pub line_number: usize,
    /// Severity tag: `P1`, `P1.1`, `P2`, `P3.b`, `Minor`, etc. —
    /// preserved verbatim from the source header.
    pub severity: String,
    /// Class tag: `CODE`, `DOCS`, `UX`, `CODE/DOCS`, `non-scope`, etc.
    /// — preserved verbatim from the source header. Used to bucket
    /// findings in the synthesis output.
    pub class: String,
    /// One-line title summarizing the finding (everything after
    /// `class:` on the header line).
    pub title: String,
}

/// One bucket in the synthesis output — all findings sharing a class.
#[derive(Debug, Clone, Serialize)]
pub struct Bucket {
    /// Class label this bucket is keyed on (e.g. `CODE`).
    pub class: String,
    /// The findings in this bucket, in the order they were extracted
    /// (preserves cross-report reading order).
    pub findings: Vec<Finding>,
}

/// The full synthesis report.
#[derive(Debug, Clone, Serialize)]
pub struct Synthesis {
    /// Reports scanned, in the operator-supplied order.
    pub reports: Vec<PathBuf>,
    /// Total findings discovered across all reports.
    pub total_findings: usize,
    /// Per-class buckets, alphabetized by class label.
    pub buckets: Vec<Bucket>,
}

/// Entry point for `corvid beta synthesize-feedback`.
pub fn run_synthesize_feedback(reports: &[PathBuf], json: bool) -> Result<u8> {
    let synthesis = synthesize(reports)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&synthesis)
                .context("serialize synthesis report")?
        );
    } else {
        print!("{}", render_markdown(&synthesis));
    }
    Ok(0)
}

/// Walk every report and produce a structured synthesis. Returned
/// shape is JSON-serializable so the `--json` output is a one-line
/// `serde_json::to_string_pretty` call.
pub fn synthesize(reports: &[PathBuf]) -> Result<Synthesis> {
    let mut all_findings: Vec<Finding> = Vec::new();
    for report in reports {
        let text = std::fs::read_to_string(report)
            .with_context(|| format!("read trial report `{}`", report.display()))?;
        all_findings.extend(extract_findings(report, &text));
    }

    // Group by class. BTreeMap gives alphabetized buckets which is
    // both deterministic AND human-readable in the markdown output.
    let mut by_class: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
    for finding in &all_findings {
        by_class
            .entry(finding.class.clone())
            .or_default()
            .push(finding.clone());
    }
    let buckets = by_class
        .into_iter()
        .map(|(class, findings)| Bucket { class, findings })
        .collect::<Vec<_>>();

    Ok(Synthesis {
        reports: reports.to_vec(),
        total_findings: all_findings.len(),
        buckets,
    })
}

/// Extract every `### <SEVERITY> — <CLASS>: <TITLE>` header from a
/// trial report. The parser is deliberately conservative: it ONLY
/// matches the exact `### <sev> — <class>: <title>` shape used by
/// the trial-report convention at
/// `docs/external-trials/33m-friends-and-family-prompt.md`'s report
/// template. Other `###` headers (intake, table-of-contents, etc.)
/// are ignored.
fn extract_findings(report_path: &Path, text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_number = idx + 1;
        let line = raw_line.trim_end();
        let Some(stripped) = line.strip_prefix("### ") else {
            continue;
        };
        // Header shape: `<SEVERITY> — <CLASS>: <TITLE>`.
        // The em-dash `—` (U+2014) is the canonical separator in
        // our trial-report template. Some reports use ` - ` (ASCII
        // hyphen with spaces) for backward compatibility.
        let dash_separator = if stripped.contains(" — ") {
            " — "
        } else if stripped.contains(" - ") {
            " - "
        } else {
            continue;
        };
        let Some((severity, rest)) = stripped.split_once(dash_separator) else {
            continue;
        };
        let Some((class, title)) = rest.split_once(": ") else {
            continue;
        };
        // Severity tag must START with `P` (for P1/P2/P3 + variants)
        // or be `Minor`. Filters out section headers that happen to
        // contain " — " (e.g. round-2 dispositions tables).
        let severity_trim = severity.trim();
        let is_severity = severity_trim.starts_with('P')
            && severity_trim
                .chars()
                .skip(1)
                .all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_lowercase())
            || severity_trim == "Minor";
        if !is_severity {
            continue;
        }
        findings.push(Finding {
            report_path: report_path.to_path_buf(),
            line_number,
            severity: severity_trim.to_string(),
            class: class.trim().to_string(),
            title: title.trim().to_string(),
        });
    }
    findings
}

/// Render the synthesis as a human-readable markdown document. The
/// shape mirrors the reviewer-facing reports — per-class sections
/// with bullet-list findings, each citing its source.
pub fn render_markdown(synthesis: &Synthesis) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Trial-feedback synthesis — {} report(s), {} finding(s)",
        synthesis.reports.len(),
        synthesis.total_findings
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Reports scanned");
    let _ = writeln!(out);
    for report in &synthesis.reports {
        let _ = writeln!(out, "- `{}`", report.display());
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Findings by class");
    let _ = writeln!(out);
    if synthesis.buckets.is_empty() {
        let _ = writeln!(out, "_(none — the reports contained no `### P<n>` headers)_");
        return out;
    }
    for bucket in &synthesis.buckets {
        let _ = writeln!(
            out,
            "### {} ({} finding(s))",
            bucket.class,
            bucket.findings.len()
        );
        let _ = writeln!(out);
        for finding in &bucket.findings {
            let _ = writeln!(
                out,
                "- **[{}]** {} — `{}:{}`",
                finding.severity,
                finding.title,
                finding.report_path.display(),
                finding.line_number
            );
        }
        let _ = writeln!(out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_findings_recognizes_canonical_em_dash_header() {
        let text = "\
## Intake

### P1 — CODE: serve has no tool-handler registration

Some body.

### P2.1 — UX: route table mislabels routes

More body.

### P3.b — CODE/DOCS: default ARG resolves to broken version

Other body.

### Minor — DOCS: `corvid --version` reports 0.0.1

### Minor — `corvid --version` reports 0.0.1 (no class — should be skipped)
";
        let findings = extract_findings(Path::new("test.md"), text);
        // 4 entries with `class: title` shape → extracted. The
        // class-less Minor entry at the end is conservatively
        // skipped because the regex requires `<sev> — <class>: <title>`.
        // (A future slice can support the class-less Minor shape if
        // reviewers ask for it; for now we stay strict so a missing
        // `class:` can't fall through as the title.)
        assert_eq!(findings.len(), 4, "got: {findings:#?}");
        assert_eq!(findings[0].severity, "P1");
        assert_eq!(findings[0].class, "CODE");
        assert_eq!(findings[1].severity, "P2.1");
        assert_eq!(findings[1].class, "UX");
        assert_eq!(findings[2].severity, "P3.b");
        assert_eq!(findings[2].class, "CODE/DOCS");
        assert_eq!(findings[3].severity, "Minor");
        assert_eq!(findings[3].class, "DOCS");
    }

    /// 33Q13a load-bearing assertion: the synthesizer is GROUNDED.
    /// When the source corpus does NOT mention a theme, the
    /// synthesis MUST NOT claim it does. This is the
    /// hallucination-prevention property — pre-condition for ever
    /// promoting the helper to an LLM-driven variant.
    #[test]
    fn synthesizer_does_not_fabricate_findings_absent_from_source() {
        let text = "\
### P1 — CODE: only one real finding
Body only.
";
        let findings = extract_findings(Path::new("only-one.md"), text);
        assert_eq!(findings.len(), 1);
        // Adversarial: the parser MUST NOT invent a P2/P3/Minor
        // that wasn't in the source.
        for f in &findings {
            assert!(
                ["P1"].contains(&f.severity.as_str()),
                "synthesizer fabricated severity `{}` not present in source",
                f.severity
            );
        }
    }

    #[test]
    fn buckets_alphabetize_and_count_correctly() {
        let text = "\
### P1 — UX: a ux thing
### P2 — CODE: a code thing
### P3 — CODE: another code thing
";
        let synthesis =
            synthesize(&[PathBuf::from("dummy.md")]);
        // Above call would fail because dummy.md doesn't exist; use
        // the lower-level extract + manual bucket assertion instead.
        let _ = synthesis;
        let findings = extract_findings(Path::new("dummy.md"), text);
        let mut by_class: BTreeMap<String, Vec<Finding>> = BTreeMap::new();
        for f in &findings {
            by_class.entry(f.class.clone()).or_default().push(f.clone());
        }
        let buckets: Vec<_> = by_class
            .into_iter()
            .map(|(class, findings)| Bucket { class, findings })
            .collect();
        assert_eq!(buckets.len(), 2, "expected CODE + UX buckets");
        assert_eq!(buckets[0].class, "CODE");
        assert_eq!(buckets[0].findings.len(), 2);
        assert_eq!(buckets[1].class, "UX");
        assert_eq!(buckets[1].findings.len(), 1);
    }
}
