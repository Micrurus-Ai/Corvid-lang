use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct UpgradeFinding {
    path: String,
    rule_id: &'static str,
    kind: &'static str,
    message: &'static str,
    replacement: &'static str,
    occurrences: usize,
}

struct RewriteRule {
    id: &'static str,
    kind: &'static str,
    from: &'static str,
    to: &'static str,
    message: &'static str,
}

const RULES: &[RewriteRule] = &[
    RewriteRule {
        id: "syntax.pub_extern_agent_single_line",
        kind: "syntax",
        from: "pub extern \"c\"\nagent ",
        to: "pub extern \"c\" agent ",
        message: "`pub extern \"c\" agent` is the stable v1 spelling; split-line legacy form is migrated automatically",
    },
    RewriteRule {
        id: "stdlib.llm_complete_to_agent_run",
        kind: "stdlib",
        from: "std.llm.complete(",
        to: "std.agent.run(",
        message: "`std.llm.complete` is replaced by the policy-aware `std.agent.run` entrypoint",
    },
    RewriteRule {
        id: "stdlib.cache_get_or_create_to_remember",
        kind: "stdlib",
        from: "std.cache.get_or_create(",
        to: "std.cache.remember(",
        message: "`std.cache.get_or_create` is replaced by `std.cache.remember` with the same key/value contract",
    },
    RewriteRule {
        id: "schema.migration_state_v1",
        kind: "schema",
        from: "\"schema\":\"corvid.migration_state.v0\"",
        to: "\"schema\":\"corvid.migration_state.v1\"",
        message: "migration state files must declare the v1 schema before stable release tooling consumes them",
    },
    RewriteRule {
        id: "trace.format_v1",
        kind: "trace",
        from: "\"schema\":\"corvid.trace.v0\"",
        to: "\"schema\":\"corvid.trace.v1\"",
        message: "trace envelopes must use the v1 trace schema for stable replay and claim audit",
    },
    RewriteRule {
        id: "connector.manifest_v1",
        kind: "connector",
        from: "\"manifest_version\":\"0.1\"",
        to: "\"manifest_version\":\"1.0\"",
        message: "connector manifests must use manifest_version 1.0 for stable scope, replay, and approval checks",
    },
];

pub fn run_check(
    root: &Path,
    json: bool,
    claims_current: Option<&Path>,
    claims_target: Option<&Path>,
) -> Result<u8> {
    let findings = collect_findings(root)?;

    // 43Q: claim-regression check. When both `--claims-current`
    // and `--claims-target` are provided, compare the two claim
    // manifests and refuse if any registered guarantee id would
    // be removed or downgraded.
    let claim_regressions = match (claims_current, claims_target) {
        (None, None) => Vec::new(),
        (Some(current), Some(target)) => check_claim_regression(current, target)?,
        _ => bail!(
            "`--claims-current` and `--claims-target` must be supplied together (one without the other is ambiguous)"
        ),
    };

    if json {
        // Output shape contract: when no `--claims-current` /
        // `--claims-target` flags are supplied, emit just the
        // findings array. The launch-readiness integration
        // tests (`upgrade_command_reports_and_applies_*` in
        // `crates/corvid-cli/tests/reference_apps.rs`) consume
        // this shape directly. When claim-regression flags ARE
        // supplied, emit the full report object so the claim
        // regressions are surfaced alongside the findings.
        if claims_current.is_none() && claims_target.is_none() {
            println!(
                "{}",
                serde_json::to_string_pretty(&findings)
                    .context("serialize upgrade findings array")?
            );
        } else {
            let report = UpgradeCheckReport {
                findings: &findings,
                claim_regressions: &claim_regressions,
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&report).context("serialize upgrade findings")?
            );
        }
    } else {
        render_findings(&findings);
        render_claim_regressions(&claim_regressions);
    }
    let exit = if !findings.is_empty() || !claim_regressions.is_empty() {
        1
    } else {
        0
    };
    Ok(exit)
}

/// One entry in a claim manifest JSON file — produced (eventually)
/// by `corvid claim --explain --json <cdylib>`. The minimal shape
/// is `{id, class}` per row; extra fields are tolerated for
/// forward compatibility.
#[derive(Debug, Clone, Deserialize)]
struct ClaimManifestRow {
    id: String,
    class: String,
}

/// A regression flagged by the claim-regression check.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimRegression {
    pub id: String,
    pub kind: &'static str,
    pub from: String,
    pub to: String,
    pub message: String,
}

#[derive(Serialize)]
struct UpgradeCheckReport<'a> {
    findings: &'a [UpgradeFinding],
    claim_regressions: &'a [ClaimRegression],
}

fn parse_claim_manifest(path: &Path) -> Result<HashMap<String, String>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("read claim manifest `{}`", path.display()))?;
    let rows: Vec<ClaimManifestRow> = serde_json::from_str(&text).with_context(|| {
        format!("parse claim manifest `{}` as JSON array of {{id, class}} objects", path.display())
    })?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        out.insert(row.id, row.class);
    }
    Ok(out)
}

/// Compare two claim manifests + report removals or class
/// downgrades.
///
/// 43Q: returns a non-empty vec when the upgrade target REMOVES
/// any id from the current manifest, OR when an id's class
/// downgrades (Static → RuntimeChecked, Static → OutOfScope, or
/// RuntimeChecked → OutOfScope). A class upgrade
/// (OutOfScope → RuntimeChecked, etc.) is fine — those go to the
/// findings list as informational, not as regressions.
fn check_claim_regression(
    current_path: &Path,
    target_path: &Path,
) -> Result<Vec<ClaimRegression>> {
    let current = parse_claim_manifest(current_path)?;
    let target = parse_claim_manifest(target_path)?;
    let mut regressions = Vec::new();
    for (id, current_class) in &current {
        match target.get(id) {
            None => regressions.push(ClaimRegression {
                id: id.clone(),
                kind: "removed",
                from: current_class.clone(),
                to: "absent".into(),
                message: format!(
                    "upgrade would REMOVE guarantee `{id}` (current class `{current_class}`)"
                ),
            }),
            Some(target_class) if class_rank(target_class) < class_rank(current_class) => {
                regressions.push(ClaimRegression {
                    id: id.clone(),
                    kind: "downgraded",
                    from: current_class.clone(),
                    to: target_class.clone(),
                    message: format!(
                        "upgrade would DOWNGRADE guarantee `{id}` from `{current_class}` to `{target_class}`"
                    ),
                });
            }
            _ => {}
        }
    }
    regressions.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(regressions)
}

/// Rank a `class` string for the downgrade comparison.
/// Static (2) > RuntimeChecked (1) > OutOfScope (0). Unknown
/// strings rank as 0 (treated as the weakest class — a typo
/// shouldn't silently look like a downgrade).
fn class_rank(class: &str) -> u8 {
    match class.to_ascii_lowercase().as_str() {
        "static" => 2,
        "runtimechecked" | "runtime_checked" => 1,
        "outofscope" | "out_of_scope" => 0,
        _ => 0,
    }
}

fn render_claim_regressions(regressions: &[ClaimRegression]) {
    if regressions.is_empty() {
        return;
    }
    println!();
    println!("claim regressions ({} found):", regressions.len());
    for reg in regressions {
        println!("  - {}", reg.message);
    }
    println!();
    println!(
        "REFUSED. The upgrade target removes or downgrades {} \
         registered guarantee(s). Run `corvid claim --explain --json \
         <new.cdylib>` and verify the claim surface before \
         re-running with the corrected target.",
        regressions.len()
    );
}

pub fn run_apply(root: &Path, json: bool) -> Result<u8> {
    let mut findings = Vec::new();
    for path in corvid_sources(root)? {
        let original =
            fs::read_to_string(&path).with_context(|| format!("read `{}`", path.display()))?;
        let mut rewritten = original.clone();
        for rule in RULES {
            let occurrences = rewritten.matches(rule.from).count();
            if occurrences == 0 {
                continue;
            }
            findings.push(finding_for(&path, rule, occurrences));
            rewritten = rewritten.replace(rule.from, rule.to);
        }
        if rewritten != original {
            fs::write(&path, rewritten).with_context(|| format!("write `{}`", path.display()))?;
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&findings).context("serialize applied findings")?
        );
    } else {
        render_findings(&findings);
    }
    Ok(0)
}

fn collect_findings(root: &Path) -> Result<Vec<UpgradeFinding>> {
    let mut findings = Vec::new();
    for path in corvid_sources(root)? {
        let source =
            fs::read_to_string(&path).with_context(|| format!("read `{}`", path.display()))?;
        for rule in RULES {
            let occurrences = source.matches(rule.from).count();
            if occurrences > 0 {
                findings.push(finding_for(&path, rule, occurrences));
            }
        }
    }
    Ok(findings)
}

fn corvid_sources(root: &Path) -> Result<Vec<PathBuf>> {
    if root.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }

    let mut files = Vec::new();
    collect_corvid_sources(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_corvid_sources(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read dir `{}`", dir.display()))? {
        let entry = entry.with_context(|| format!("read dir entry in `{}`", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_corvid_sources(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext == "cor" || ext == "json")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn finding_for(path: &Path, rule: &RewriteRule, occurrences: usize) -> UpgradeFinding {
    UpgradeFinding {
        path: path.display().to_string(),
        rule_id: rule.id,
        kind: rule.kind,
        message: rule.message,
        replacement: rule.to,
        occurrences,
    }
}

fn render_findings(findings: &[UpgradeFinding]) {
    println!("corvid upgrade report");
    println!("finding_count: {}", findings.len());
    for finding in findings {
        println!(
            "{} [{}] {} occurrences={} replacement={}",
            finding.path,
            finding.kind,
            finding.rule_id,
            finding.occurrences,
            finding.replacement
        );
        println!("  {}", finding.message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_manifest(dir: &Path, name: &str, rows: &[(&str, &str)]) -> PathBuf {
        let path = dir.join(name);
        let body: Vec<_> = rows
            .iter()
            .map(|(id, class)| serde_json::json!({"id": id, "class": class}))
            .collect();
        fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).unwrap();
        path
    }

    /// 43Q: the claim-regression check is silent (no regressions
    /// flagged) when the upgrade target preserves every guarantee
    /// id with its current class.
    #[test]
    fn claim_regression_check_passes_when_manifests_match() {
        let dir = tempdir().unwrap();
        let current = write_manifest(
            dir.path(),
            "current.json",
            &[
                ("approval.dangerous_call_requires_token", "Static"),
                ("jobs.cron_schedule_durable", "RuntimeChecked"),
            ],
        );
        let target = write_manifest(
            dir.path(),
            "target.json",
            &[
                ("approval.dangerous_call_requires_token", "Static"),
                ("jobs.cron_schedule_durable", "RuntimeChecked"),
            ],
        );
        let regressions = check_claim_regression(&current, &target).unwrap();
        assert!(
            regressions.is_empty(),
            "expected zero regressions, got {regressions:?}"
        );
    }

    /// 43Q: a removed guarantee id is flagged as a regression.
    /// Catches the "upgrade silently drops a Static guarantee"
    /// failure mode that the v1.0 claim-stability promise forbids.
    #[test]
    fn claim_regression_check_flags_removed_guarantee() {
        let dir = tempdir().unwrap();
        let current = write_manifest(
            dir.path(),
            "current.json",
            &[("approval.dangerous_call_requires_token", "Static")],
        );
        let target = write_manifest(dir.path(), "target.json", &[]);
        let regressions = check_claim_regression(&current, &target).unwrap();
        assert_eq!(regressions.len(), 1);
        let r = &regressions[0];
        assert_eq!(r.id, "approval.dangerous_call_requires_token");
        assert_eq!(r.kind, "removed");
        assert_eq!(r.from, "Static");
        assert_eq!(r.to, "absent");
    }

    /// 43Q: a downgrade (Static → RuntimeChecked, Static →
    /// OutOfScope, RuntimeChecked → OutOfScope) is flagged. An
    /// upgrade (OutOfScope → RuntimeChecked, etc.) is NOT a
    /// regression and produces no entry.
    #[test]
    fn claim_regression_check_flags_class_downgrades_only() {
        let dir = tempdir().unwrap();
        let current = write_manifest(
            dir.path(),
            "current.json",
            &[
                ("a.static_to_runtime", "Static"),
                ("b.static_to_oos", "Static"),
                ("c.runtime_to_oos", "RuntimeChecked"),
                ("d.oos_to_runtime", "OutOfScope"), // upgrade, not regression
                ("e.unchanged", "Static"),
            ],
        );
        let target = write_manifest(
            dir.path(),
            "target.json",
            &[
                ("a.static_to_runtime", "RuntimeChecked"),
                ("b.static_to_oos", "OutOfScope"),
                ("c.runtime_to_oos", "OutOfScope"),
                ("d.oos_to_runtime", "RuntimeChecked"),
                ("e.unchanged", "Static"),
            ],
        );
        let regressions = check_claim_regression(&current, &target).unwrap();
        let ids: Vec<&str> = regressions.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "a.static_to_runtime",
                "b.static_to_oos",
                "c.runtime_to_oos",
            ],
            "downgrades should be flagged in sorted-id order; \
             oos_to_runtime should NOT appear (it's an upgrade); \
             unchanged should NOT appear"
        );
    }

    /// 43Q: providing only one of `--claims-current` /
    /// `--claims-target` is ambiguous and refuses with an error.
    #[test]
    fn upgrade_check_refuses_unpaired_claim_manifest_flag() {
        let dir = tempdir().unwrap();
        let current = write_manifest(dir.path(), "current.json", &[]);
        let project_dir = dir.path().to_path_buf();
        let err = run_check(&project_dir, false, Some(&current), None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("must be supplied together"),
            "error should explain pairing requirement; got: {msg}"
        );
    }
}
