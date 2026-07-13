use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum UpgradeCommand {
    /// Report syntax and stdlib migrations without modifying files.
    ///
    /// 43Q: also supports a claim-regression check that refuses
    /// to recommend an upgrade if any registered guarantee would
    /// be removed or downgraded (Static→RuntimeChecked, or
    /// RuntimeChecked→OutOfScope). The two claim manifests are
    /// JSON arrays of `{id, class}` pairs the operator produces
    /// via `corvid claim --explain --json <cdylib>`.
    Check {
        /// Source file or project directory to scan.
        path: PathBuf,
        /// Emit JSON findings.
        #[arg(long)]
        json: bool,
        /// Current binary's claim manifest (JSON file). Produces
        /// via `corvid claim --explain --json <current.cdylib>`.
        #[arg(long, value_name = "PATH")]
        claims_current: Option<PathBuf>,
        /// Upgrade target's claim manifest (JSON file). Produces
        /// via `corvid claim --explain --json <target.cdylib>`.
        /// Required when `--claims-current` is set.
        #[arg(long, value_name = "PATH")]
        claims_target: Option<PathBuf>,
    },
    /// Apply safe syntax and stdlib migrations.
    Apply {
        /// Source file or project directory to rewrite.
        path: PathBuf,
        /// Emit JSON findings.
        #[arg(long)]
        json: bool,
    },
    /// Refresh the project's vendored `src/std/` from the current
    /// install's stdlib (slice 47b). Projects vendored from an
    /// older install pick up new modules (std/json, std/time,
    /// std/mcp, ...) without manual copying. Local edits under
    /// `src/std/` are overwritten for modules that changed
    /// upstream — the vendored stdlib is not a user-edit surface.
    RefreshStd {
        /// Project root (the directory containing `src/`).
        path: PathBuf,
    },
    /// Audit source for patterns that will need attention at the
    /// next strict-typecheck or feature-boundary upgrade — slice
    /// 33Q13e (third and last of the remaining AI helpers under
    /// `35V2-P43-T-LR-phase-43-ai-helpers`).
    ///
    /// Distinct from `corvid upgrade check`: `check` reports
    /// mechanical syntax/stdlib substitutions that `apply` can
    /// rewrite automatically. `assist` reports patterns that
    /// require operator judgment — e.g. custom `trust:`/`data:`
    /// values that 33Q7b will require `corvid.toml` declarations
    /// for, `pub extern "c"` agents with struct boundaries that
    /// 33Q8 will lift, LLM-tool-using agents with no `@budget`
    /// constraint (cost-overrun risk). Output is a structured
    /// recommendations list with severity buckets + per-pattern
    /// citations of the source line that triggered the finding.
    ///
    /// v1.0 implementation is deterministic Rust (same shape as
    /// `corvid claim audit`, `corvid beta synthesize-feedback`,
    /// `corvid deploy tailor`). A post-v1.0 follow-up
    /// (33Q13f-upgrade-assist-llm-promote) adds LLM-driven
    /// refinement anchored to the same deterministic signals.
    Assist {
        /// Source file or project directory to audit.
        path: PathBuf,
        /// Emit JSON findings instead of the default markdown
        /// rendering. Useful for downstream tooling that wants to
        /// consume the structured shape directly.
        #[arg(long)]
        json: bool,
    },
}
