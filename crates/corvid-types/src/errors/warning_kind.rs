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
    /// An `identity` block explicitly weakened a session safe-default
    /// via `insecure_opt_out: true` (slice 51g). Allowed, but never
    /// silent: the weakened posture surfaces in review and audit.
    IdentityInsecureSession { identity: String, reasons: String },
    /// A type the file declares or imports has the name of a built-in
    /// type, so in this file the name means the program's type and the
    /// built-in is hidden. Allowed, so a built-in added later never breaks
    /// a program, but said, because a value a built-in returns (such as
    /// `from_json`'s `JsonError`) is still the built-in type.
    BuiltinTypeHidden { name: String },
}

impl TypeWarningKind {
    /// This warning's code: `W` and the range of its area.
    /// Declared, never inferred from the message; codes are unique across
    /// every diagnostic kind ([diagnostic codes](../../../docs/proposals/diagnostic-codes.md)).
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnboundedCostAnalysis { .. } => "W0251",
            Self::StreamReturnWithoutYield { .. } => "W0901",
            Self::ReplayUnreachableArm { .. } => "W0101",
            Self::UnsafePythonImport { .. } => "W0201",
            Self::ScheduleNotExecutable { .. } => "W0902",
            Self::InferenceNeedsAnnotation { .. } => "W0202",
            Self::IdentityInsecureSession { .. } => "W0701",
            Self::BuiltinTypeHidden { .. } => "W0301",
        }
    }
}

impl TypeWarningKind {
    pub fn message(&self) -> String {
        match self {
            Self::UnboundedCostAnalysis { agent, message } => {
                format!("cost analysis warning in agent `{agent}`: {message}")
            }
            Self::StreamReturnWithoutYield { agent } => {
                format!("W0270: agent `{agent}` declares `Stream<T>` return but never yields a value or returns a stream")
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
            Self::IdentityInsecureSession { identity, reasons } => {
                format!(
                    "W0300: `identity {identity}` weakens a session safe-default ({reasons}) via `insecure_opt_out: true` — allowed, but ship this only for local development"
                )
            }
            Self::BuiltinTypeHidden { name } => {
                format!("`{name}` in this file is the type the program declares, which hides the built-in `{name}`")
            }
        }
    }

    pub fn hint(&self) -> Option<String> {
        match self {
            Self::UnboundedCostAnalysis { .. } => Some(
                "use a statically bounded loop or inspect `:cost <agent>` for the partial tree".into(),
            ),
            Self::StreamReturnWithoutYield { .. } => Some(
                "yield an element, return a stream, or change the declared return type".into(),
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
            Self::IdentityInsecureSession { .. } => Some(
                "remove `insecure_opt_out` (and the weakened options) before production — restore `secure`, `http_only`, a non-`none` SameSite, and session rotation".into(),
            ),
            Self::BuiltinTypeHidden { name } => Some(format!(
                "a value a built-in gives you is still the built-in `{name}`, so it will not match a `{name}` annotation here; rename your type to use both"
            )),
        }
    }
}
