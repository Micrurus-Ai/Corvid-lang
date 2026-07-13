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

/// Slice 47b: refresh the vendored `src/std/` from the current
/// install's stdlib and report what changed.
pub fn run_refresh_std(root: &Path) -> Result<u8> {
    let report = corvid_driver::refresh_vendored_std(root)?;
    println!("corvid upgrade refresh-std");
    println!("source: {}", report.source.display());
    if report.added.is_empty() && report.updated.is_empty() {
        println!(
            "vendored stdlib is up to date ({} modules unchanged)",
            report.unchanged.len()
        );
        return Ok(0);
    }
    for name in &report.added {
        println!("  added   src/std/{name}");
    }
    for name in &report.updated {
        println!("  updated src/std/{name}");
    }
    println!(
        "{} added, {} updated, {} unchanged",
        report.added.len(),
        report.updated.len(),
        report.unchanged.len()
    );
    Ok(0)
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

// ---------- Slice 33Q13e — `corvid upgrade assist` ----------
//
// Distinct from `corvid upgrade check` (mechanical rewrites that
// `apply` can automate): `assist` audits source for patterns that
// require operator judgment to migrate, with structured
// recommendations + per-finding source citations.

/// Severity ordering for assist findings. Critical = must address
/// before the next strict-typecheck pass would reject the source;
/// warn = will compile today but at boundary risk; info = forward-
/// looking suggestion that improves robustness.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssistSeverity {
    Critical,
    Warn,
    Info,
}

/// One actionable upgrade-readiness recommendation. Every finding
/// cites the source file + 1-indexed line that triggered it —
/// mirrors the 33Q13a synthesize-feedback groundedness contract.
#[derive(Debug, Clone, Serialize)]
pub struct AssistFinding {
    pub severity: AssistSeverity,
    /// Stable id of the detection rule (e.g. `trust.custom_value`,
    /// `extern_c.struct_boundary`, `agent.llm_no_budget`).
    pub rule_id: &'static str,
    /// Brief title naming what to do.
    pub title: String,
    /// Source file the finding was detected in.
    pub file: PathBuf,
    /// 1-indexed line number of the triggering token.
    pub line: usize,
    /// Detailed rationale + the actionable upgrade path.
    pub rationale: String,
}

/// The full assist report.
#[derive(Debug, Clone, Serialize)]
pub struct AssistReport {
    pub scanned_files: usize,
    pub findings: Vec<AssistFinding>,
}

/// Canonical trust values from `docs/internals/effect-spec/04-builtin-dimensions.md` § 4.2 +
/// the autonomous_if_confident gate. Mirrors the 33Q7a drift gate's
/// constant. Any `trust:` value outside this set is a `Name(String)`
/// extension that 33Q7b will require an explicit `corvid.toml`
/// declaration for.
const CANONICAL_TRUST_VALUES: &[&str] = &[
    "autonomous",
    "supervisor_required",
    "human_required",
    "autonomous_if_confident",
];

/// Canonical data values from § 4.4. Same drift-gate logic as trust.
const CANONICAL_DATA_VALUES: &[&str] = &[
    "none", "public", "pii", "financial", "medical", "grounded",
];

pub fn run_assist(root: &Path, json: bool) -> Result<u8> {
    let sources = corvid_sources(root)?;
    let mut findings: Vec<AssistFinding> = Vec::new();
    for path in &sources {
        let text =
            fs::read_to_string(path).with_context(|| format!("read `{}`", path.display()))?;
        findings.extend(scan_for_assist_findings(path, &text));
    }
    let report = AssistReport {
        scanned_files: sources.len(),
        findings,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("serialize assist report")?
        );
    } else {
        print!("{}", render_assist_markdown(&report));
    }
    Ok(0)
}

/// Walk one source file's lines for the detection patterns 33Q13e
/// ships with. Each pattern is a small per-line regex-style check
/// — no IR walk needed because the patterns are syntactic. (A
/// future LLM-promote slice can add IR-level checks that the
/// LLM layer can reason about.)
fn scan_for_assist_findings(path: &Path, text: &str) -> Vec<AssistFinding> {
    let mut findings = Vec::new();
    let mut prev_line: Option<&str> = None;
    for (idx, raw_line) in text.lines().enumerate() {
        let line_number = idx + 1;
        let line = raw_line.trim_start();

        // Pattern 1 — `trust:` custom value (33Q7b migration).
        // Matches `trust: <value>` on a line; flag if value isn't
        // canonical. The 33Q7a drift gate catches this in the
        // reference-app corpus; this rule extends the same check
        // to user code.
        if let Some(value) = parse_dimension_value(line, "trust") {
            if !CANONICAL_TRUST_VALUES.iter().any(|c| *c == value) {
                findings.push(AssistFinding {
                    severity: AssistSeverity::Warn,
                    rule_id: "trust.custom_value",
                    title: format!(
                        "Non-canonical `trust: {value}` will require corvid.toml declaration in v1.1"
                    ),
                    file: path.to_path_buf(),
                    line: line_number,
                    rationale: format!(
                        "The v1.0 typechecker accepts any string as a `trust:` value via \
                         `DimensionValue::Name(String)`. Slice 33Q7b (post-v1.0) tightens \
                         this — non-canonical values will require an explicit \
                         `[effect-system.dimensions.{value}]` block in `corvid.toml`. The \
                         canonical lattice is `autonomous < supervisor_required < \
                         human_required` (plus `autonomous_if_confident(<t>)`). See \
                         `docs/internals/effect-spec/reference-app-dimensions.md` for the \
                         shipped reference-app extensions and how they will migrate."
                    ),
                });
            }
        }

        // Pattern 2 — `data:` custom value (33Q7b migration, parallel
        // to trust).
        if let Some(value) = parse_dimension_value(line, "data") {
            if !CANONICAL_DATA_VALUES.iter().any(|c| *c == value) {
                findings.push(AssistFinding {
                    severity: AssistSeverity::Warn,
                    rule_id: "data.custom_value",
                    title: format!(
                        "Non-canonical `data: {value}` will require corvid.toml declaration in v1.1"
                    ),
                    file: path.to_path_buf(),
                    line: line_number,
                    rationale: format!(
                        "Same 33Q7b migration as `trust:`. Canonical `data:` values are \
                         `none`, `public`, `pii`, `financial`, `medical`, `grounded`. \
                         `{value}` parses today but will require an explicit \
                         `[effect-system.dimensions.{value}]` block in `corvid.toml` after \
                         the strict-typecheck promotion."
                    ),
                });
            }
        }

        // Pattern 3 — `pub extern "c"` agent on the prior line +
        // current line declares an agent with struct boundary types.
        // This is 33Q8 territory — won't compile today, will after
        // post-v1.0 lift. Flag as `info` so reviewers know the
        // restriction is tracked.
        if let Some(prev) = prev_line {
            if prev.trim_start().starts_with("pub extern \"c\"")
                && line.starts_with("agent ")
                && line_signals_struct_boundary(line)
            {
                findings.push(AssistFinding {
                    severity: AssistSeverity::Info,
                    rule_id: "extern_c.struct_boundary",
                    title: "`pub extern \"c\"` agent has struct-shaped boundary — 33Q8 lift pending"
                        .to_string(),
                    file: path.to_path_buf(),
                    line: line_number,
                    rationale: "v1.0 rejects struct parameters / returns on `pub extern \"c\"` \
                                agents (scalar-only boundary). Slice 33Q8 (post-v1.0) lifts \
                                this; today's workaround is to decompose the struct into \
                                scalars at the boundary OR pass JSON-through-String. See \
                                `docs/reference/exported-abi.md` for the full v1.0 ABI \
                                surface + 33Q8 plan."
                        .to_string(),
                });
            }
        }

        // Pattern 4 — agent declaration that USES llm-shaped effects
        // but has no `@budget` attribute in the preceding header
        // block. This isn't perfect (we'd need IR for that) but the
        // heuristic `uses llm` / `uses *_ai` without a `@budget`
        // above is a reasonable lint.
        // Conservative: only fires on `agent <name>...uses <effect>:`
        // patterns where the effect name contains `llm` or `ai`.
        if line.starts_with("agent ") && line.contains(" uses ") {
            let uses_llm = line
                .split(" uses ")
                .nth(1)
                .map(|s| s.contains("llm") || s.contains("_ai"))
                .unwrap_or(false);
            let prev_has_budget = prev_line.map_or(false, |p| p.contains("@budget("));
            if uses_llm && !prev_has_budget {
                findings.push(AssistFinding {
                    severity: AssistSeverity::Warn,
                    rule_id: "agent.llm_no_budget",
                    title: "LLM-using agent has no `@budget` constraint — runaway cost risk"
                        .to_string(),
                    file: path.to_path_buf(),
                    line: line_number,
                    rationale: "An agent declared with `uses <llm-shaped-effect>` is reachable \
                                by code paths whose cost the compiler can't bound without an \
                                `@budget($X)` annotation. Without a budget the moat's \
                                compile-time-cost-ceiling guarantee (`budget.compile_time_ceiling` \
                                in `corvid-guarantees`) doesn't apply — the agent can drift to \
                                unbounded spend at runtime. Add `@budget($X)` on the line above \
                                the agent declaration to opt into the ceiling."
                        .to_string(),
                });
            }
        }

        prev_line = Some(raw_line);
    }
    findings
}

/// Parse `<dim>: <value>` on a single line, returning the value if
/// the line matches the shape. Used by the trust and data
/// dimension-extension lints. Lines starting with `#` are comments
/// and are skipped.
///
/// **False-positive guard**: also returns `None` when the value
/// starts with an uppercase letter. Effect-dimension VALUES are
/// always lowercase identifiers (`autonomous`, `external`, `pii`,
/// etc.) — uppercase tokens after `trust:` or `data:` are TYPE NAMES
/// in struct field declarations like:
///
/// ```corvid
/// public type EffectTag:
///     trust: String
///     data: String
/// ```
///
/// Surfaced when the assist scanner hit `src/std/effects.cor` and
/// false-positived `trust: String` / `data: String` as non-canonical
/// dimension values. Without this guard, every Corvid source that
/// declares a struct with a `trust:` or `data:` field would trip
/// the lint.
fn parse_dimension_value(line: &str, dimension: &str) -> Option<String> {
    if line.starts_with('#') {
        return None;
    }
    let needle = format!("{dimension}:");
    let idx = line.find(&needle)?;
    // The dim must appear at the start of a trimmed line (effect
    // body) — otherwise `data:` inside a comment or a string would
    // match. Conservative: require column 0 after trim.
    if !line.starts_with(&needle) {
        return None;
    }
    let after = &line[idx + needle.len()..];
    let trimmed = after.trim_start();
    let value: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if value.is_empty() {
        return None;
    }
    // Effect-dimension values are lowercase. Uppercase first char
    // means this is a type name in a struct field declaration, not
    // a dimension value — skip.
    if value.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return None;
    }
    Some(value)
}

/// Heuristic: does the agent declaration line signal struct-shaped
/// parameters or return? We look for `(<name>: <Title-cased type>)`
/// or `-> <Title-cased type>` where the type is NOT one of the
/// scalar v1.0 boundary types. This is a syntactic approximation —
/// false positives on type aliases are acceptable for an `info`-
/// severity finding (the reviewer reads the line and decides).
fn line_signals_struct_boundary(line: &str) -> bool {
    const SCALAR_TYPES: &[&str] = &["Int", "Float", "Bool", "String", "Nothing"];
    // Crude: look for a `: SomethingCapital` token where Something
    // isn't a scalar; same for `-> SomethingCapital`. If we find
    // one we flag the line.
    for needle in [": ", "-> "] {
        for chunk in line.split(needle).skip(1) {
            let ident: String = chunk
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if ident.is_empty() {
                continue;
            }
            let first = ident.chars().next().unwrap();
            if first.is_ascii_uppercase() && !SCALAR_TYPES.contains(&ident.as_str()) {
                // Also exclude Grounded<T> + Option<T> + List<T>
                // header-prefixes which would parse with a capital
                // but aren't struct shapes per se.
                if matches!(ident.as_str(), "Grounded" | "Option" | "List") {
                    continue;
                }
                return true;
            }
        }
    }
    false
}

pub fn render_assist_markdown(report: &AssistReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Upgrade-readiness assist — {} file(s) scanned, {} finding(s)",
        report.scanned_files,
        report.findings.len()
    );
    let _ = writeln!(out);
    if report.findings.is_empty() {
        let _ = writeln!(out, "_(no upgrade-readiness findings — your source is in good shape against the v1.0 → v1.1 strict-typecheck migration path.)_");
        return out;
    }
    for sev in [
        AssistSeverity::Critical,
        AssistSeverity::Warn,
        AssistSeverity::Info,
    ] {
        let bucket: Vec<&AssistFinding> = report
            .findings
            .iter()
            .filter(|f| f.severity == sev)
            .collect();
        if bucket.is_empty() {
            continue;
        }
        let _ = writeln!(out, "## {:?} ({})", sev, bucket.len());
        let _ = writeln!(out);
        for f in bucket {
            let _ = writeln!(
                out,
                "- **{}** _(rule: `{}`, source: `{}:{}`)_",
                f.title,
                f.rule_id,
                f.file.display(),
                f.line
            );
            let _ = writeln!(out, "  - {}", f.rationale);
        }
        let _ = writeln!(out);
    }
    out
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
