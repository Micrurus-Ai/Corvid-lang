//! Runtime errors raised by Corvid's deterministic core.
//!
//! Distinct from `corvid-vm::InterpError` — those are interpreter-level
//! (type mismatch, division by zero). These cover the runtime boundary:
//! tool dispatch, approval flow, LLM adapters, tracing.
//!
//! The interpreter wraps `RuntimeError` into `InterpError::Runtime(...)`
//! when it bubbles up to user code; downstream renderers can pattern-match
//! either form.
//!
//! Lives in `corvid-runtime-core` so wasm-clean callers (the browser
//! playground, future agent-execution paths) can surface the same
//! error vocabulary as native CLI users. `corvid-runtime` re-exports
//! through its `errors` module so existing native consumers see no API
//! change.

use crate::replay_divergence::ReplayDivergence;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// A tool name was called that has no registered handler.
    UnknownTool(String),

    /// A tool handler returned an error.
    ToolFailed { tool: String, message: String },

    /// A prompt name was called that has no registered template / handler.
    UnknownPrompt(String),

    /// No LLM adapter is registered that can handle the requested model.
    NoAdapter(String),

    /// An LLM adapter returned an error (HTTP, parse, etc.).
    AdapterFailed { adapter: String, message: String },

    /// A Python FFI import/call failed.
    PythonFailed {
        module: String,
        function: String,
        traceback: String,
    },

    /// A Python FFI call requested a host capability not declared by policy.
    PythonPolicyDenied {
        module: String,
        function: String,
        required_effect: String,
    },

    /// Approval was denied (user said no, programmatic approver returned deny).
    ApprovalDenied { action: String },

    /// Approval flow returned no synchronous decision — the approver
    /// queued the request for later out-of-band review and the agent
    /// must not proceed in this run. The `approval_id` is the queued
    /// item's identifier; callers route on it (e.g. `corvid serve`
    /// answers HTTP 202 carrying this id so the client can poll
    /// `/__approvals/<id>` for the eventual decision). Distinct from
    /// `ApprovalDenied` because there is no decision yet — a future
    /// reviewer or queue runtime decides; that decision lives in the
    /// approval queue store rather than in this RuntimeError.
    ApprovalQueued { approval_id: String },

    /// Approval flow failed for a non-deny reason (timeout, IO).
    ApprovalFailed(String),

    /// Wire-format conversion failed (Value <-> JSON marshalling).
    Marshal(String),

    /// A store write was based on a stale record revision.
    StoreConflict {
        kind: String,
        store: String,
        key: String,
        expected_revision: u64,
        actual_revision: Option<u64>,
    },

    /// A store operation violated a declared store policy.
    StorePolicyViolation {
        kind: String,
        store: String,
        key: String,
        policy: String,
        message: String,
    },

    /// No model is configured for an LLM call. Hint to set `CORVID_MODEL`
    /// or pass `model=` per call.
    NoModelConfigured,

    /// The model catalog in `corvid.toml` was present but malformed.
    ModelCatalogParse { path: PathBuf, message: String },

    /// Capability-based routing could not find any registered model
    /// strong enough for the prompt's requirement.
    NoEligibleModel {
        required_capability: String,
        required_output_format: Option<String>,
        available_models: Vec<String>,
    },

    ModelOutputFormatMismatch {
        prompt: String,
        model: String,
        required_output_format: String,
        model_output_format: Option<String>,
    },

    /// A `route:` prompt evaluated every arm and found no match.
    NoMatchingRoute {
        prompt: String,
    },

    /// An adversarial pipeline's adjudicator returned a value that
    /// does not satisfy the runtime contradiction contract.
    InvalidAdversarialVerdict {
        prompt: String,
        message: String,
    },

    ReplayTraceLoad {
        path: PathBuf,
        message: String,
    },

    ReplayDivergence(ReplayDivergence),

    InvalidReplayMutation {
        step: usize,
        message: String,
    },

    CrossTierReplayUnsupported {
        recorded_writer: String,
        replay_writer: String,
    },

    /// A replay-mode runtime caught a side-effect call that bypassed the
    /// recorded-event substitution path. `surface` names the quarantined
    /// component (`"llm"` for slice `35V2-P38-C-4`; `"http"`, `"store"`,
    /// `"io"` follow in `35V2-P38-C-5`). `detail` carries human-readable
    /// context (adapter name, model, URL, file path) so the operator can
    /// trace the violation back to source. Surfaces this rather than
    /// `Other(...)` so the bypass case is testable and distinguishable
    /// from a `ReplayDivergence` (the substitution-mismatch case the
    /// interpreter path already produces today).
    QuarantineViolation {
        surface: String,
        detail: String,
    },

    /// Phase 33S0: an executing I/O surface (file / http / SQLite)
    /// was called before its per-surface slice (33S1 / 33S2 / 33S3)
    /// wired the actual runtime side. The ffi_bridge stubs return
    /// this variant so a program that tries to call e.g.
    /// `io.read_text(...)` against a partially-built toolchain gets
    /// a precise diagnostic naming the surface, not a generic crash.
    /// Cleared from existence by the per-surface slice landing the
    /// real implementation — once 33S1 ships, the `io` arm of any
    /// ffi_bridge function should never return this variant.
    SurfaceNotImplemented {
        surface: String,
        function: String,
    },

    /// Catch-all. Prefer adding a dedicated variant.
    Other(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "no handler registered for tool `{name}`"),
            Self::ToolFailed { tool, message } => {
                write!(f, "tool `{tool}` failed: {message}")
            }
            Self::UnknownPrompt(name) => write!(f, "no prompt named `{name}`"),
            Self::NoAdapter(model) => {
                write!(f, "no LLM adapter registered for model `{model}`")
            }
            Self::AdapterFailed { adapter, message } => {
                write!(f, "LLM adapter `{adapter}` failed: {message}")
            }
            Self::PythonFailed {
                module,
                function,
                traceback,
            } => {
                write!(f, "python call `{module}.{function}` failed: {traceback}")
            }
            Self::PythonPolicyDenied {
                module,
                function,
                required_effect,
            } => write!(
                f,
                "python call `{module}.{function}` requires undeclared effect `{required_effect}`"
            ),
            Self::ApprovalDenied { action } => {
                write!(f, "approval denied for `{action}`")
            }
            Self::ApprovalQueued { approval_id } => {
                write!(f, "approval queued for review (id `{approval_id}`)")
            }
            Self::ApprovalFailed(msg) => write!(f, "approval flow failed: {msg}"),
            Self::Marshal(msg) => write!(f, "value marshalling failed: {msg}"),
            Self::StoreConflict {
                kind,
                store,
                key,
                expected_revision,
                actual_revision,
            } => write!(
                f,
                "store conflict for {kind} `{store}` key `{key}`: expected revision {expected_revision}, found {}",
                actual_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "missing record".to_string())
            ),
            Self::StorePolicyViolation {
                kind,
                store,
                key,
                policy,
                message,
            } => write!(
                f,
                "store policy `{policy}` rejected {kind} `{store}` key `{key}`: {message}"
            ),
            Self::NoModelConfigured => write!(
                f,
                "no LLM model configured. Set CORVID_MODEL, add `default_model = \"...\"` to corvid.toml, or pass `model=` per call."
            ),
            Self::ModelCatalogParse { path, message } => write!(
                f,
                "failed to parse model catalog in `{}`: {message}",
                path.display()
            ),
            Self::NoEligibleModel {
                required_capability,
                required_output_format,
                available_models,
            } => write!(
                f,
                "no eligible model for capability `{required_capability}`{}; available models: {}",
                required_output_format
                    .as_deref()
                    .map(|format| format!(" and output format `{format}`"))
                    .unwrap_or_default(),
                if available_models.is_empty() {
                    "none".to_string()
                } else {
                    available_models.join(", ")
                }
            ),
            Self::ModelOutputFormatMismatch {
                prompt,
                model,
                required_output_format,
                model_output_format,
            } => write!(
                f,
                "prompt `{prompt}` requires output format `{required_output_format}`, but model `{model}` advertises {}",
                model_output_format
                    .as_deref()
                    .map(|format| format!("`{format}`"))
                    .unwrap_or_else(|| "no output format".to_string())
            ),
            Self::NoMatchingRoute { prompt } => {
                write!(f, "no matching route arm for prompt `{prompt}`")
            }
            Self::InvalidAdversarialVerdict { prompt, message } => {
                write!(
                    f,
                    "invalid adversarial verdict for prompt `{prompt}`: {message}"
                )
            }
            Self::ReplayTraceLoad { path, message } => {
                write!(f, "failed to load replay trace `{}`: {message}", path.display())
            }
            Self::ReplayDivergence(err) => err.fmt(f),
            Self::InvalidReplayMutation { step, message } => write!(
                f,
                "invalid replay mutation at substitutable step {step}: {message}"
            ),
            Self::CrossTierReplayUnsupported {
                recorded_writer,
                replay_writer,
            } => write!(
                f,
                "cross-tier replay is not supported in v1: trace writer `{recorded_writer}` cannot replay on `{replay_writer}`"
            ),
            Self::QuarantineViolation { surface, detail } => write!(
                f,
                "replay-mode quarantine ({surface}) blocked an unrecorded side-effect: {detail}"
            ),
            Self::SurfaceNotImplemented { surface, function } => write!(
                f,
                "executing surface `{surface}` is registered but its implementation has not yet been wired — \
                 `{function}` is reachable today only as a ffi_bridge stub. The runtime side ships in the \
                 33S1/33S2/33S3 per-surface slices; calling this function before those land returns \
                 SurfaceNotImplemented so the program fails closed rather than silently no-op."
            ),
            Self::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slice 33S0 — the SurfaceNotImplemented variant renders a
    /// diagnostic that names both the surface and the function so
    /// an early caller (one reaching an ffi_bridge stub before
    /// 33S1/2/3 lands the real impl) sees a precise error rather
    /// than a generic "feature missing" sentinel.
    #[test]
    fn surface_not_implemented_display_names_surface_and_function() {
        let err = RuntimeError::SurfaceNotImplemented {
            surface: "io".to_string(),
            function: "io.read_text".to_string(),
        };
        let rendered = format!("{err}");
        assert!(
            rendered.contains("io"),
            "Display must mention the surface; got {rendered}"
        );
        assert!(
            rendered.contains("io.read_text"),
            "Display must mention the function name; got {rendered}"
        );
        assert!(
            rendered.contains("33S1") || rendered.contains("33S2") || rendered.contains("33S3"),
            "Display should reference the slice number that will wire the impl; got {rendered}"
        );
    }

    /// Slice 33S0 — `SurfaceNotImplemented` is distinguishable
    /// from `QuarantineViolation` at the variant level. Pre-fix
    /// the ffi_bridge stubs would have had to return a generic
    /// `Other(...)` which can't be matched cleanly; the new
    /// variant lets call sites pattern-match precisely.
    #[test]
    fn surface_not_implemented_is_a_distinct_variant_from_quarantine_violation() {
        let surface_err = RuntimeError::SurfaceNotImplemented {
            surface: "http".to_string(),
            function: "http.post_json".to_string(),
        };
        let quarantine_err = RuntimeError::QuarantineViolation {
            surface: "http".to_string(),
            detail: "POST to api.example.com".to_string(),
        };

        assert!(matches!(
            surface_err,
            RuntimeError::SurfaceNotImplemented { .. }
        ));
        assert!(matches!(
            quarantine_err,
            RuntimeError::QuarantineViolation { .. }
        ));
        assert!(!matches!(
            surface_err,
            RuntimeError::QuarantineViolation { .. }
        ));
    }
}
