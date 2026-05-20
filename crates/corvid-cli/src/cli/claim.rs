use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ClaimCommand {
    /// Audit launch-facing claims for runnable evidence or
    /// explicit non-scope status. With `--explain-failures`,
    /// each finding is paired with a typed `kind` + a
    /// `suggested_fix` describing the concrete remediation;
    /// every fix back-references the inventory line so an
    /// operator can navigate straight to the source row
    /// (Grounded<T> shape at the claim-audit layer).
    Audit {
        /// Claim inventory markdown table.
        #[arg(
            long,
            value_name = "PATH",
            default_value = "docs/meta/launch-claim-audit.md"
        )]
        inventory: PathBuf,
        /// Emit JSON report.
        #[arg(long)]
        json: bool,
        /// Pair every finding with a typed `kind` + a
        /// `suggested_fix` describing the concrete remediation.
        /// Operators triaging a CI claim-audit failure read the
        /// explanations first; the machine-readable JSON output
        /// is available via `--json --explain-failures`.
        #[arg(long)]
        explain_failures: bool,
    },
}
