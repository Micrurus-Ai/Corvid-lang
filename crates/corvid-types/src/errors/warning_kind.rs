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
    /// A `schedule` declaration fires only under the governed-cron
    /// scheduler runner (`corvid schedule run`), never during a plain
    /// `corvid run` — which executes `main` once. Without this note,
    /// a reviewer writing `schedule "0 9 * * *" zone
    /// "America/New_York" -> ...` and testing via `corvid run` would
    /// expect the cron to fire and have no signal about which command
    /// starts it (the original gap surfaced in self-trial round 4
    /// against a job-coordinator project, before the runner shipped).
    ScheduleNotExecutable { agent: String, cron: String },
    /// An unannotated binding whose initializer cannot determine
    /// its full type (slice 45q): `x = []` (element type Unknown)
    /// or `x = None` (payload type Unknown). Downstream checking
    /// weakens to Unknown-accepts-everything; an annotation
    /// restores full checking.
    InferenceNeedsAnnotation {
        binding: String,
        construct: String,
        hint: String,
    },
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
            Self::InferenceNeedsAnnotation {
                binding,
                construct,
                hint,
            } => {
                format!(
                    "W0290: `{binding}` is bound to {construct}, so its full type is unknown and later checks weaken — annotate it (`{hint}`) to keep full type checking"
                )
            }
            Self::ScheduleNotExecutable { agent, cron } => {
                format!(
                    "W0280: `schedule \"{cron}\" -> {agent}(...)` fires only under the \
                     scheduler runner (`corvid schedule run --source <this file>`) — a plain \
                     `corvid run` executes `main` once and does not start the cron"
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
            Self::InferenceNeedsAnnotation { hint, .. } => {
                Some(format!("write `{hint}` with the real inner type"))
            }
            Self::ScheduleNotExecutable { .. } => Some(
                "start the governed-cron runner with `corvid schedule run --source <file>` — scheduled agents inherit tracing, retries, dead-letters, and replay from the durable-jobs queue".into(),
            ),
        }
    }
}
