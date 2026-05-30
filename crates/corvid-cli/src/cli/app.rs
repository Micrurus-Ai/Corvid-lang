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
    /// Walk the app's ABI surface and suggest the canonical
    /// adversarial fixtures every surface element should have.
    ///
    /// Lowers the supplied `.cor` source through the standard
    /// pipeline, builds the ABI descriptor in-process, and emits
    /// one `AdversarialSuggestion` per (surface_element,
    /// threat_category) pair: cross-tenant variants for every
    /// dangerous tool and writeable store, role-bypass + expired
    /// reuse for every approval site, replay-without-token for
    /// every `@replayable` agent, malformed-payload + role-bypass
    /// for every `pub extern "c"` agent. Each suggestion carries
    /// Grounded<T>-shaped `sources` back-referencing the
    /// descriptor element it came from.
    ///
    /// Deterministic + replay-stable. Promotes
    /// `app.adversarial_refresh_grounded` to runtime-checked.
    AdversarialRefresh {
        /// Corvid source file to walk.
        file: PathBuf,
    },
    /// Diff two Corvid app surfaces and render a typed PR
    /// description. Lowers both sources to ABI descriptors
    /// in-process and emits typed sections (Breaking, Additive,
    /// Informational) covering agents, tools, approval gates,
    /// types, stores, claim guarantees, and ABI / compiler
    /// versions. Every bullet carries Grounded<T>-shaped
    /// `sources` back-referencing the descriptor field that
    /// diverged. Reviewer reads Breaking sections first.
    ///
    /// Deterministic + replay-stable. Promotes
    /// `app.pr_describe_grounded` to runtime-checked.
    PrDescribe {
        /// The base-side Corvid source (typically the
        /// merge target — `main`).
        #[arg(long, value_name = "FILE")]
        base: PathBuf,
        /// The head-side Corvid source (typically the
        /// branch being merged).
        #[arg(long, value_name = "FILE")]
        head: PathBuf,
    },
}
