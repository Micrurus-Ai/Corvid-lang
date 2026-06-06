//! `corvid beta` subcommand surface — slice 33Q13a.
//!
//! Operator-facing helpers for running the friends-and-family
//! (33M) beta round. Today the surface is just
//! `synthesize-feedback` (deterministic synthesis across N trial
//! reports); other helpers (`recruit`, `track`, `summarize-round`)
//! land as the round scales.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum BetaCommand {
    /// Synthesize findings across one or more trial-report markdown
    /// files into a single themed report grouped by class (CODE /
    /// DOCS / UX / etc.) with file:line citations back to the source
    /// reports.
    ///
    /// Each finding is grounded — the synthesizer cites the file
    /// and line it was extracted from, and the test corpus pins
    /// the no-fabrication property (when a report doesn't mention
    /// a theme, the synthesis MUST NOT claim it does). The v1.0
    /// implementation is deterministic Rust (mirrors `corvid claim
    /// audit`); a post-v1.0 follow-up slice
    /// (33Q13-llm-promote-synthesize-feedback) adds LLM-driven
    /// thematic clustering on top of the same grounded base.
    SynthesizeFeedback {
        /// Paths to trial-report markdown files. Provide one or more.
        /// Each file is parsed for `### P<n>` / `### Minor` finding
        /// headers and grouped by their declared class.
        #[arg(required = true, value_name = "REPORT", num_args = 1..)]
        reports: Vec<PathBuf>,
        /// Emit the synthesis as JSON instead of the default
        /// markdown rendering. Useful for downstream tooling that
        /// wants to consume the structured shape directly.
        #[arg(long)]
        json: bool,
    },
}
