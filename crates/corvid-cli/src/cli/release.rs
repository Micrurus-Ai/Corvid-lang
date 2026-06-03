use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ReleaseCommand {
    /// Produce signed release-channel artifacts (binary + SBOM +
    /// SHA256SUMS + signed manifest). The pre-existing positional
    /// `corvid release <channel> <version>` shape became
    /// `corvid release build <channel> <version>` in slice
    /// `35V2-P43-T-LR-release-notes` to make room for sibling
    /// subcommands. The direct-channel forms `corvid release
    /// nightly|beta|stable <version>` are aliases of
    /// `corvid release build <channel> <version>` — they match
    /// the launch-rehearsal smoke-command shape published in
    /// `docs/launch-rehearsal.md` and the reference_apps
    /// integration test that gates Phase 43.
    Build {
        /// Release channel: nightly, beta, or stable.
        channel: String,
        /// Explicit version. Nightly requires `-nightly.`, beta
        /// requires `-beta.`, stable is plain SemVer.
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
