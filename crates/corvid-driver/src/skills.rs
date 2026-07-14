//! The skill capability surface: manifest, audit, verification, and
//! vendoring for effect-audited skill packages.
//!
//! A skill is a directory of Corvid source plus a `skill.toml`
//! manifest declaring the skill's CAPABILITY LABEL — the ceiling on
//! what the code inside may do (which stdlib capability groups it
//! uses, its maximum trust tier and per-effect cost, which data
//! classes it touches) plus its declared external reach and required
//! configuration. The label is enforced twice:
//!
//! 1. At add time (`corvid add skill <path>`): the audit is COMPUTED
//!    from the skill's actual source and verified against the label
//!    before any code lands in the project; the rendered "nutrition
//!    label" is what the user consents to.
//! 2. At every check/run: [`verify_project_skills`] recomputes the
//!    audit for each vendored skill so a skill edited past its label
//!    fails loudly, naming the exceeded dimension.
//!
//! Capability detection deliberately OVER-approximates: any mention
//! of a stdlib executing tool's name in the skill's token stream
//! counts as a use of that capability group. Over-approximation can
//! only over-report (require a broader label), never let a use slip
//! under the label — the conservative direction for a security
//! audit. Effect dimensions (trust / cost / data) come from the
//! skill's parsed `effect` declarations, which is where the checker
//! reads them too.

use corvid_syntax::{lex, parse_file, TokKind};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Trust tiers in ascending order of required oversight.
const TRUST_TIERS: &[&str] = &["autonomous", "supervisor_required", "human_required"];

fn trust_rank(tier: &str) -> Option<usize> {
    TRUST_TIERS.iter().position(|t| *t == tier)
}

/// Map a stdlib executing tool name to its capability group.
fn capability_group(tool: &str) -> Option<&'static str> {
    let group = if tool.starts_with("io_") {
        "io"
    } else if tool.starts_with("http_") {
        "http"
    } else if tool.starts_with("db_") {
        "db"
    } else if tool.starts_with("json_") {
        "json"
    } else if tool.starts_with("time_") {
        "time"
    } else if tool.starts_with("random_") {
        "random"
    } else if tool.starts_with("rag_") {
        "rag"
    } else if tool == "mcp_call" {
        "mcp"
    } else if tool == "secret_read" {
        "secrets"
    } else if tool.starts_with("cache_") {
        "cache"
    } else {
        return None;
    };
    Some(group)
}

/// The parsed `skill.toml`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SkillManifest {
    pub skill: SkillMeta,
    #[serde(default)]
    pub capabilities: SkillCapabilities,
    #[serde(default)]
    pub reach: SkillReach,
    #[serde(default)]
    pub requires: SkillRequires,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
}

/// The capability label — the enforced ceiling.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SkillCapabilities {
    /// Allowed capability groups (io / http / db / json / time /
    /// random / rag / mcp / secrets / cache / llm). Empty = the
    /// skill claims to be pure vocabulary.
    #[serde(default)]
    pub uses: Vec<String>,
    /// Maximum trust tier any effect in the skill may declare.
    /// Absent = `autonomous` (the strictest ceiling).
    #[serde(default)]
    pub max_trust: Option<String>,
    /// Maximum per-effect declared cost in USD. Absent = 0.0 (no
    /// paid effects unless the label raises the ceiling).
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Allowed `data:` dimension values. Empty = no data claims
    /// allowed beyond `none`.
    #[serde(default)]
    pub data: Vec<String>,
}

/// Declared external reach. NOT statically enforced (hosts and
/// paths are runtime values confined by `[http] allow` and
/// `[io] root`); rendered on the label so the user knows what the
/// skill SAYS it needs.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SkillReach {
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct SkillRequires {
    /// Environment variables (usually secrets) the skill reads.
    #[serde(default)]
    pub env: Vec<String>,
}

/// What the skill's source ACTUALLY does, computed from code.
#[derive(Debug, Default)]
pub struct SkillAudit {
    /// Capability groups whose tools the source mentions.
    pub capabilities: BTreeSet<String>,
    /// Highest trust tier among the skill's effect declarations.
    pub max_trust: Option<String>,
    /// Highest per-effect declared cost.
    pub max_cost_usd: f64,
    /// `data:` dimension values declared by the skill's effects.
    pub data: BTreeSet<String>,
    /// Names of tools declared `dangerous` inside the skill.
    pub dangerous_tools: Vec<String>,
    /// Exported (public) tool/agent/prompt names.
    pub exports: Vec<String>,
    /// Files that failed to parse (audit refuses in that case).
    pub unparsable: Vec<String>,
}

/// Compute the audit for every `.cor` file under `skill_dir`
/// (non-recursive top level plus one nested level — skills stay
/// flat by convention).
pub fn compute_skill_audit(skill_dir: &Path) -> anyhow::Result<SkillAudit> {
    let mut audit = SkillAudit::default();
    for source_path in cor_files(skill_dir)? {
        let source = std::fs::read_to_string(&source_path)?;
        let rel = source_path
            .strip_prefix(skill_dir)
            .unwrap_or(&source_path)
            .display()
            .to_string();
        let Ok(tokens) = lex(&source) else {
            audit.unparsable.push(rel);
            continue;
        };
        // Capability scan: token-level, over-approximating.
        for tok in &tokens {
            if let TokKind::Ident(name) = &tok.kind {
                if let Some(group) = capability_group(name) {
                    audit.capabilities.insert(group.to_string());
                }
                if name == "call_llm" {
                    audit.capabilities.insert("llm".to_string());
                }
            }
        }
        let (file, parse_errors) = parse_file(&tokens);
        if !parse_errors.is_empty() {
            audit.unparsable.push(rel);
            continue;
        }
        for decl in &file.decls {
            match decl {
                corvid_ast::Decl::Effect(effect) => {
                    for dim in &effect.dimensions {
                        match (dim.name.name.as_str(), &dim.value) {
                            ("trust", corvid_ast::DimensionValue::Name(tier)) => {
                                let new_rank = trust_rank(tier);
                                let old_rank =
                                    audit.max_trust.as_deref().and_then(trust_rank);
                                if new_rank > old_rank {
                                    audit.max_trust = Some(tier.clone());
                                }
                            }
                            ("cost", corvid_ast::DimensionValue::Cost(usd)) => {
                                if *usd > audit.max_cost_usd {
                                    audit.max_cost_usd = *usd;
                                }
                            }
                            ("data", corvid_ast::DimensionValue::Name(class)) => {
                                if class != "none" {
                                    audit.data.insert(class.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                corvid_ast::Decl::Tool(tool) => {
                    if tool.effect == corvid_ast::Effect::Dangerous {
                        audit.dangerous_tools.push(tool.name.name.clone());
                    }
                    if tool.visibility.is_callable_from_outside_file() {
                        audit.exports.push(format!("tool {}", tool.name.name));
                    }
                }
                corvid_ast::Decl::Agent(agent) => {
                    if agent.visibility.is_callable_from_outside_file() {
                        audit.exports.push(format!("agent {}", agent.name.name));
                    }
                }
                corvid_ast::Decl::Prompt(prompt) => {
                    audit.capabilities.insert("llm".to_string());
                    if prompt.visibility.is_callable_from_outside_file() {
                        audit.exports.push(format!("prompt {}", prompt.name.name));
                    }
                }
                _ => {}
            }
        }
    }
    Ok(audit)
}

fn cor_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "cor") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Verify the computed audit fits the declared label. Every
/// violation names the exceeded dimension and both sides.
pub fn verify_label(manifest: &SkillManifest, audit: &SkillAudit) -> Vec<String> {
    let mut violations = Vec::new();
    let name = &manifest.skill.name;

    for file in &audit.unparsable {
        violations.push(format!(
            "skill `{name}`: `{file}` does not parse — the audit cannot vouch for code it \
             cannot read"
        ));
    }

    let allowed: BTreeSet<&str> = manifest
        .capabilities
        .uses
        .iter()
        .map(String::as_str)
        .collect();
    for capability in &audit.capabilities {
        if !allowed.contains(capability.as_str()) {
            violations.push(format!(
                "skill `{name}`: capability `{capability}` is used by the source but the \
                 label's `capabilities.uses` allows only [{}]",
                manifest.capabilities.uses.join(", ")
            ));
        }
    }

    let label_trust = manifest
        .capabilities
        .max_trust
        .as_deref()
        .unwrap_or("autonomous");
    let Some(label_trust_rank) = trust_rank(label_trust) else {
        violations.push(format!(
            "skill `{name}`: label max_trust `{label_trust}` is not a trust tier \
             (expected one of [{}])",
            TRUST_TIERS.join(", ")
        ));
        return violations;
    };
    if let Some(actual_trust) = &audit.max_trust {
        if trust_rank(actual_trust) > Some(label_trust_rank) {
            violations.push(format!(
                "skill `{name}`: trust dimension exceeds the label — an effect declares \
                 `trust: {actual_trust}` but the label allows at most `{label_trust}`"
            ));
        }
    }

    let label_cost = manifest.capabilities.max_cost_usd.unwrap_or(0.0);
    if audit.max_cost_usd > label_cost {
        violations.push(format!(
            "skill `{name}`: cost dimension exceeds the label — an effect declares \
             `cost: ${:.4}` but the label allows at most ${label_cost:.4}",
            audit.max_cost_usd
        ));
    }

    let allowed_data: BTreeSet<&str> = manifest
        .capabilities
        .data
        .iter()
        .map(String::as_str)
        .collect();
    for class in &audit.data {
        if !allowed_data.contains(class.as_str()) {
            violations.push(format!(
                "skill `{name}`: data dimension exceeds the label — an effect declares \
                 `data: {class}` but the label's `capabilities.data` allows only [{}]",
                manifest.capabilities.data.join(", ")
            ));
        }
    }

    violations
}

/// Render the consent audit — the "nutrition label".
pub fn render_label(manifest: &SkillManifest, audit: &SkillAudit, signed: bool) -> String {
    let mut out = String::new();
    let meta = &manifest.skill;
    let _ = writeln!(out, "skill: {} v{}", meta.name, meta.version);
    if !meta.description.is_empty() {
        let _ = writeln!(out, "  {}", meta.description);
    }
    if !signed {
        let _ = writeln!(
            out,
            "\n  !! UNSIGNED — no publisher attestation; you are trusting the source \
             you fetched it from."
        );
    }
    let _ = writeln!(out, "\ncapability label (verified against the source):");
    let caps = if audit.capabilities.is_empty() {
        "none (pure vocabulary)".to_string()
    } else {
        audit
            .capabilities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let _ = writeln!(out, "  uses:       {caps}");
    let _ = writeln!(
        out,
        "  max trust:  {}",
        audit.max_trust.as_deref().unwrap_or("autonomous")
    );
    let _ = writeln!(out, "  max cost:   ${:.4} per call", audit.max_cost_usd);
    if !audit.data.is_empty() {
        let _ = writeln!(
            out,
            "  data:       {}",
            audit.data.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    if !audit.dangerous_tools.is_empty() {
        let _ = writeln!(
            out,
            "  dangerous:  {} tool(s) requiring `approve` at call sites: {}",
            audit.dangerous_tools.len(),
            audit.dangerous_tools.join(", ")
        );
    }
    if !manifest.reach.hosts.is_empty() {
        let _ = writeln!(
            out,
            "  reach:      hosts {} (enforced at runtime by [http] allow)",
            manifest.reach.hosts.join(", ")
        );
    }
    if !manifest.reach.paths.is_empty() {
        let _ = writeln!(
            out,
            "  reach:      paths {} (confined at runtime by [io] root)",
            manifest.reach.paths.join(", ")
        );
    }
    if !manifest.requires.env.is_empty() {
        let _ = writeln!(
            out,
            "  requires:   env {}",
            manifest.requires.env.join(", ")
        );
    }
    if !audit.exports.is_empty() {
        let _ = writeln!(out, "\nexports:");
        for export in &audit.exports {
            let _ = writeln!(out, "  {export}");
        }
    }
    out
}

/// Load and validate a skill directory's manifest.
pub fn load_manifest(skill_dir: &Path) -> anyhow::Result<SkillManifest> {
    let manifest_path = skill_dir.join("skill.toml");
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| {
        anyhow::anyhow!(
            "no readable skill.toml at `{}`: {e}",
            manifest_path.display()
        )
    })?;
    let manifest: SkillManifest = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("invalid skill.toml at `{}`: {e}", manifest_path.display()))?;
    if manifest.skill.name.trim().is_empty() {
        anyhow::bail!("skill.toml: `skill.name` must not be empty");
    }
    if !manifest
        .skill
        .name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "skill.toml: `skill.name` `{}` must be lowercase kebab/snake case (it becomes \
             the src/skills/<name>/ directory)",
            manifest.skill.name
        );
    }
    Ok(manifest)
}

/// The outcome `corvid add skill` reports.
#[derive(Debug)]
pub struct SkillAddPlan {
    pub manifest: SkillManifest,
    pub rendered_label: String,
    pub destination: PathBuf,
}

/// Validate a skill source dir against its own label and prepare the
/// vendor plan. Refuses (Err) on label violations — a skill whose
/// label is dishonest must not be installable.
pub fn plan_add_skill(project_root: &Path, source_dir: &Path) -> anyhow::Result<SkillAddPlan> {
    let manifest = load_manifest(source_dir)?;
    let audit = compute_skill_audit(source_dir)?;
    let violations = verify_label(&manifest, &audit);
    if !violations.is_empty() {
        anyhow::bail!(
            "the skill's label does not cover what its source does:\n  {}",
            violations.join("\n  ")
        );
    }
    let destination = project_root
        .join("src")
        .join("skills")
        .join(&manifest.skill.name);
    if destination.exists() {
        anyhow::bail!(
            "`{}` already exists — updating an installed skill is `corvid skill update` \
             (hash-pinned; rides slice 49b)",
            destination.display()
        );
    }
    let rendered_label = render_label(&manifest, &audit, false);
    Ok(SkillAddPlan {
        manifest,
        rendered_label,
        destination,
    })
}

/// Execute a validated plan: copy the skill source into the project.
pub fn vendor_skill(plan: &SkillAddPlan, source_dir: &Path) -> anyhow::Result<()> {
    copy_dir(source_dir, &plan.destination)
}

fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Re-verify every vendored skill in a project — called on every
/// check/run so an edited skill cannot silently outgrow its label.
/// Returns human-readable violations; empty = all labels hold.
pub fn verify_project_skills(project_root: &Path) -> Vec<String> {
    let skills_dir = project_root.join("src").join("skills");
    let Ok(entries) = std::fs::read_dir(&skills_dir) else {
        return Vec::new();
    };
    let mut violations = Vec::new();
    for entry in entries.flatten() {
        let skill_dir = entry.path();
        if !skill_dir.is_dir() || !skill_dir.join("skill.toml").exists() {
            continue;
        }
        match (load_manifest(&skill_dir), compute_skill_audit(&skill_dir)) {
            (Ok(manifest), Ok(audit)) => {
                violations.extend(verify_label(&manifest, &audit));
            }
            (Err(e), _) | (_, Err(e)) => {
                violations.push(format!(
                    "skill at `{}`: {e}",
                    skill_dir.display()
                ));
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, manifest: &str, source: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("skill.toml"), manifest).unwrap();
        std::fs::write(dir.join("main.cor"), source).unwrap();
    }

    const HONEST_MANIFEST: &str = r#"
[skill]
name = "repo-summarizer"
version = "0.1.0"
description = "Summarizes repository activity."

[capabilities]
uses = ["http", "llm"]
max_trust = "supervisor_required"
max_cost_usd = 0.5
data = ["external"]

[reach]
hosts = ["api.github.com"]

[requires]
env = ["GITHUB_TOKEN"]
"#;

    const HONEST_SOURCE: &str = "\
effect repo_read:
    cost: $0.25
    trust: supervisor_required
    data: external

public tool fetch_activity(repo: String) -> String uses repo_read

public agent summarize(repo: String) -> Result<String, String>:
    activity = http_get(repo)
    return Ok(\"summary\")
";

    #[test]
    fn honest_skill_passes_its_label() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), HONEST_MANIFEST, HONEST_SOURCE);
        let manifest = load_manifest(dir.path()).unwrap();
        let audit = compute_skill_audit(dir.path()).unwrap();
        assert!(audit.capabilities.contains("http"));
        assert_eq!(audit.max_trust.as_deref(), Some("supervisor_required"));
        assert_eq!(audit.max_cost_usd, 0.25);
        let violations = verify_label(&manifest, &audit);
        assert!(violations.is_empty(), "got: {violations:?}");
    }

    #[test]
    fn capability_outside_label_is_named() {
        let dir = tempfile::tempdir().unwrap();
        // Source calls io_read_text but the label only allows http/llm.
        let source = HONEST_SOURCE
            .replace("activity = http_get(repo)", "activity = io_read_text(repo)");
        write_skill(dir.path(), HONEST_MANIFEST, &source);
        let manifest = load_manifest(dir.path()).unwrap();
        let audit = compute_skill_audit(dir.path()).unwrap();
        let violations = verify_label(&manifest, &audit);
        assert!(
            violations.iter().any(|v| v.contains("capability `io`")),
            "the violation must name the excess capability; got {violations:?}"
        );
    }

    #[test]
    fn trust_above_label_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let source = HONEST_SOURCE.replace("trust: supervisor_required", "trust: human_required");
        write_skill(dir.path(), HONEST_MANIFEST, &source);
        let manifest = load_manifest(dir.path()).unwrap();
        let audit = compute_skill_audit(dir.path()).unwrap();
        let violations = verify_label(&manifest, &audit);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("trust dimension exceeds the label")),
            "got {violations:?}"
        );
    }

    #[test]
    fn cost_above_label_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let source = HONEST_SOURCE.replace("cost: $0.25", "cost: $2.00");
        write_skill(dir.path(), HONEST_MANIFEST, &source);
        let manifest = load_manifest(dir.path()).unwrap();
        let audit = compute_skill_audit(dir.path()).unwrap();
        let violations = verify_label(&manifest, &audit);
        assert!(
            violations
                .iter()
                .any(|v| v.contains("cost dimension exceeds the label")),
            "got {violations:?}"
        );
    }

    #[test]
    fn edited_vendored_skill_fails_project_verification() {
        // The load-bearing loop: add an honest skill, then edit the
        // VENDORED copy past its label — verify_project_skills must
        // catch it.
        let project = tempfile::tempdir().unwrap();
        let source_dir = project.path().join("incoming");
        write_skill(&source_dir, HONEST_MANIFEST, HONEST_SOURCE);

        let plan = plan_add_skill(project.path(), &source_dir).unwrap();
        vendor_skill(&plan, &source_dir).unwrap();
        assert!(verify_project_skills(project.path()).is_empty());

        // Edit the vendored copy to call the db surface.
        let vendored_main = plan.destination.join("main.cor");
        let edited = std::fs::read_to_string(&vendored_main)
            .unwrap()
            .replace("activity = http_get(repo)", "activity = db_open(repo)");
        std::fs::write(&vendored_main, edited).unwrap();

        let violations = verify_project_skills(project.path());
        assert!(
            violations.iter().any(|v| v.contains("capability `db`")),
            "the edited skill must fail its label; got {violations:?}"
        );
    }

    #[test]
    fn dishonest_label_refuses_install() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = HONEST_MANIFEST.replace("uses = [\"http\", \"llm\"]", "uses = [\"llm\"]");
        write_skill(dir.path(), &manifest, HONEST_SOURCE);
        let project = tempfile::tempdir().unwrap();
        let err = plan_add_skill(project.path(), dir.path())
            .expect_err("a label that does not cover the source must refuse");
        assert!(
            err.to_string().contains("capability `http`"),
            "got: {err}"
        );
    }
}
