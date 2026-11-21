use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Copy the running CLI and write its checksum, release manifest and signed attestation.
    ///
    /// Set CORVID_RELEASE_SIGNING_KEY to a hex-encoded Ed25519 seed.
    /// The output also includes installer scripts, a changelog and release notes for operators.
    Build {
        /// Release channel: nightly, beta, or stable.
        channel: String,
        /// Release version, such as 1.0.0-nightly.local, 1.0.0-beta.2, or 1.0.0.
        version: Option<String>,
        /// Output directory for generated release artifacts.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Alias for `corvid release build nightly <version>`.
    Nightly {
        version: Option<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Alias for `corvid release build beta <version>`.
    Beta {
        version: Option<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Alias for `corvid release build stable <version>`.
    Stable {
        version: Option<String>,
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Render structured release notes between two git refs.
    /// Deterministic: walks `git log <from>..<to>`, categorises by
    /// conventional-commit prefix (feat / fix / docs / refactor /
    /// test / chore / perf), and emits markdown grouped by
    /// category. No LLM round-trip — the slice's "RAG-grounded"
    /// framing in the 43T umbrella refers to commit-history
    /// citation (every line traces back to a SHA), not a
    /// generative pipeline.
    Notes {
        /// Lower-bound git ref (exclusive). Typically the
        /// previous release tag.
        from: String,
        /// Upper-bound git ref (inclusive). Typically the new
        /// release tag, or `HEAD`.
        to: String,
        /// Output file. When omitted, prints to stdout — useful
        /// for piping into a release-issue body or attaching to
        /// a `corvid release build` run.
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
}
