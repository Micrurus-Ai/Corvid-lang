//! Type warning kind definitions and user-facing diagnostics.

use corvid_ast::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeWarningKind {
    /// The cost explorer could not prove a static upper bound.
    UnboundedCostAnalysis { agent: String, message: String },
    /// An agent declared `Stream<T>` but never actually yielded.
    StreamReturnWithoutYield { agent: String },
    /// A replay arm duplicates an earlier arm's pattern; the later
    /// arm can never match. Phase 21 slice 21-inv-E-3.
    ReplayUnreachableArm {
        pattern: String,
        first_arm_span: Span,
    },
    /// `effects: unsafe` on a Python import is explicit but should be reviewed.
    UnsafePythonImport { module: String, message: String },
    /// A `schedule` declaration is parsed and typechecked but the
    /// scheduler runner that actually fires the cron isn't part of
    /// the v1.0 runtime — the declaration is silently dropped at IR
    /// lowering today. Without this warning, a reviewer writing
    /// `schedule "0 9 * * *" zone "America/New_York" -> ...` would
    /// expect the cron to fire and have no signal that it won't
    /// (surfaced in self-trial round 4 against `/tmp/job_coordinator`).
    /// Filed alongside the eventual scheduler-runner slice as the
    /// load-bearing user-visible diagnostic that pins the gap.
    ScheduleNotExecutable { agent: String, cron: String },
}

impl TypeWarningKind {
    pub fn message(&self) -> String {
        match self {
            Self::UnboundedCostAnalysis { agent, message } => {
                format!("cost analysis warning in agent `{agent}`: {message}")
            }
            Self::StreamReturnWithoutYield { agent } => {
                format!("W0270: agent `{agent}` declares `Stream<T>` return but never yields")
            }
            Self::ReplayUnreachableArm {
                pattern,
                first_arm_span,
            } => {
                format!(
                    "replay arm `{pattern}` is unreachable: an earlier arm at [{}..{}] already matches the same recorded events",
                    first_arm_span.start, first_arm_span.end
                )
            }
            Self::UnsafePythonImport { module, message } => {
                format!("python import `{module}` declares `effects: unsafe`: {message}")
            }
            Self::ScheduleNotExecutable { agent, cron } => {
                format!(
                    "W0280: `schedule \"{cron}\" -> {agent}(...)` parses + typechecks but the \
                     v1.0 scheduler runner does not yet fire scheduled jobs — the cron will \
                     NOT execute. The declaration is preserved in the IR for the post-v1.0 \
                     runner slice that will wire it up"
                )
            }
        }
    }

    pub fn hint(&self) -> Option<String> {
        match self {
            Self::UnboundedCostAnalysis { .. } => Some(
                "use a statically bounded loop or inspect `:cost <agent>` for the partial tree".into(),
            ),
            Self::StreamReturnWithoutYield { .. } => Some(
                "either add at least one `yield` or change the return type to a non-stream value".into(),
            ),
            Self::ReplayUnreachableArm { .. } => Some(
                "remove the duplicate arm or make its pattern distinct (different prompt / tool / label)".into(),
            ),
            Self::UnsafePythonImport { .. } => Some(
                "replace `unsafe` with narrower effects such as `network`, `filesystem`, `subprocess`, `environment`, or `native_extension` when possible".into(),
            ),
            Self::ScheduleNotExecutable { .. } => Some(
                "until the scheduler runner ships, drive scheduled work from an external cron / k8s CronJob that POSTs to a Corvid HTTP route (the `server` block), OR call the agent directly via `corvid run` from your own scheduler".into(),
            ),
        }
    }
}
