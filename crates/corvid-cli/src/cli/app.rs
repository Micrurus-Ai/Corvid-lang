//! Clap arg tree for `corvid app *` — per-app assistive helpers.
//!
//! `corvid app *` houses the per-app AI helpers filed under the
//! Phase-42 launch-readiness umbrella `35V2-P42-H-LR-per-app-ai-helpers`.
//! Each subcommand is a deterministic typed classifier over the
//! app's ABI descriptor — mirroring the drift-narrator posture
//! (`connector.drift_narration_grounded`) where every derived
//! value carries Grounded<T>-shaped sources.

use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AppCommand {
    /// Render the operator-facing boot summary for a Corvid app.
    ///
    /// Lowers the supplied `.cor` source through the standard
    /// pipeline, builds the ABI descriptor in-process, and prints
    /// a typed `BootSummary` (surface counts, flagship `pub extern
    /// "c"` entrypoints, approval gates, enforced guarantees,
    /// dangerous-surface counts) whose every derived field is
    /// paired with a descriptor-field source.
    ///
    /// Replay-stable: two invocations on the same source produce
    /// byte-identical output. Promotes `app.boot_summary_grounded`
    /// to runtime-checked.
    BootSummary {
        /// Corvid source file to summarise.
        file: PathBuf,
    },
}
