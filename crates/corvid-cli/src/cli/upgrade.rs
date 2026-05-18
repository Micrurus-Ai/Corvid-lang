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
}
