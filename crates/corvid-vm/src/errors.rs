//! Runtime errors raised by the interpreter.
//!
//! These are distinct from the compile-time errors in `corvid-types`. A
//! program that passes the type checker can still raise these at runtime
//! (division by zero, unapproved action at a bypassed boundary, etc.).

use corvid_ast::Span;
use corvid_resolve::LocalId;
use corvid_runtime::RuntimeError;
use std::fmt;

#[derive(Debug, Clone)]
pub struct InterpError {
    pub kind: InterpErrorKind,
    pub span: Span,
}

impl InterpError {
    pub fn new(kind: InterpErrorKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Whether this failure is the outside world's (see
    /// [`InterpErrorKind::is_outage`]).
    pub fn is_outage(&self) -> bool {
        self.kind.is_outage()
    }
}

impl InterpErrorKind {
    /// The one rule `try ... retry` and `try ... else` use: a timeout,
    /// an unusable reply, or a runtime outage (a tool, model, database
    /// or provider that failed). A bug (arithmetic, an index, a
    /// refinement, a type mismatch) and a decision or a limit (a denied
    /// approval, a budget, a confidence floor, a token ceiling) are never
    /// outages.
    pub fn is_outage(&self) -> bool {
        match self {
            Self::Timeout { .. } | Self::UnusableReply(_) => true,
            Self::Runtime(error) => error.is_outage(),
            Self::UndefinedLocal(_)
            | Self::TypeMismatch { .. }
            | Self::UnknownField { .. }
            | Self::Arithmetic(_)
            | Self::IndexOutOfBounds { .. }
            | Self::RefinementViolated { .. }
            | Self::ArgumentOutOfRange { .. }
            | Self::BudgetExceeded { .. }
            | Self::ParallelArmCancelled
            | Self::ConfidenceFloorBreached { .. }
            | Self::TokenLimitExceeded { .. }
            | Self::NotImplemented(_)
            | Self::MissingReturn
            | Self::ApprovalDenied(_)
            | Self::CallerLacksAuthority { .. }
            | Self::DispatchFailed(_)
            | Self::Marshal(_)
            | Self::Other(_) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InterpErrorKind {
    Timeout { milliseconds: u64 },
    /// A local was referenced that has no binding in the current env.
    /// Reaching this typically means the resolver / IR lowering are out
    /// of sync with the interpreter.
    UndefinedLocal(LocalId),

    /// An operation received a value whose type it can't handle.
    /// `got` is the dynamic type name; `expected` is a short description.
    TypeMismatch { expected: String, got: String },

    /// Field access targeted a struct, but the field doesn't exist on it.
    UnknownField { struct_name: String, field: String },

    /// Arithmetic failure (overflow, division by zero, etc.).
    Arithmetic(String),

    /// Indexing a list with an out-of-range index.
    IndexOutOfBounds { len: usize, index: i64 },

    /// A value built or assigned in code breaks its field's refinement
    /// (B09): `Search(q, n)` with `n` outside `between(1, 50)`.
    RefinementViolated { type_name: String, field: String, refinement: String, value: String },
    /// An argument outside its parameter's refinement, found when the
    /// call starts: a bug in the caller, never an outage.
    ArgumentOutOfRange { callable: String, param: String, refinement: String, value: String },

    /// A streaming computation crossed the active cost budget.
    BudgetExceeded { budget: f64, used: f64 },

    /// A `parallel` arm was cooperatively cancelled after a sibling
    /// arm failed fast (slice 52d-2). This is a scheduler sentinel, not
    /// a program error: the arm stopped at a tool-dispatch boundary
    /// because it had NOT crossed a non-reversible effect boundary (the
    /// cancellation×reversibility rule). It never escapes the
    /// `parallel` block — the block reports the sibling's real error.
    ParallelArmCancelled,

    /// A streaming prompt fell below its declared confidence floor.
    ConfidenceFloorBreached { floor: f64, actual: f64 },

    /// A streaming prompt exceeded its token ceiling.
    TokenLimitExceeded { limit: u64, used: u64 },

    /// The interpreter encountered a construct it doesn't implement yet.
    /// Expected only during staged rollout — should never fire in shipped code.
    NotImplemented(String),

    /// An agent or tool returned without producing a value, but a value was expected.
    MissingReturn,

    /// An approval action was denied or failed at runtime.
    ApprovalDenied(String),

    /// Code called an action, passing a caller (`Caller`) that lacks what
    /// the action's `requires` names. A decision, never an outage.
    CallerLacksAuthority { action: String, caller: String, missing: String },

    /// A tool or prompt couldn't be dispatched.
    DispatchFailed(String),

    /// An error bubbled up from `corvid-runtime` (tool/LLM/approval/IO).
    Runtime(RuntimeError),

    /// Marshalling between `Value` and `serde_json::Value` failed at the
    /// runtime boundary: the program's own value could not be sent, or a
    /// developer's override did not fit. A bug, never an outage.
    Marshal(String),

    /// A model's reply, a tool's result or a provider's response did not
    /// fit the declared type, or a judged guard found the reply below its
    /// floor. The outside world answered unusably: an outage, which
    /// `try` can retry or catch, and which a prompt's `with repair`
    /// answers by asking again.
    UnusableReply(String),

    /// Catch-all with message. Prefer adding a dedicated variant over this.
    Other(String),
}

impl fmt::Display for InterpErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UndefinedLocal(id) => write!(f, "local binding #{} is unbound", id.0),
            Self::TypeMismatch { expected, got } => {
                write!(f, "type mismatch: expected `{expected}`, got `{got}`")
            }
            Self::UnknownField { struct_name, field } => {
                write!(f, "no field `{field}` on type `{struct_name}`")
            }
            Self::Arithmetic(msg) => write!(f, "arithmetic error: {msg}"),
            Self::IndexOutOfBounds { len, index } => {
                write!(f, "index {index} out of bounds for list of length {len}")
            }
            Self::RefinementViolated { type_name, field, refinement, value } => {
                write!(f, "`{type_name}.{field}` must be {refinement}, but this value is {value}")
            }
            Self::ArgumentOutOfRange { callable, param, refinement, value } => {
                write!(f, "`{param}` is {value}, outside `{refinement}`, in this call to `{callable}`")
            }
            Self::BudgetExceeded { budget, used } => {
                write!(f, "stream budget exceeded: used ${used:.4} over budget ${budget:.4}")
            }
            Self::ParallelArmCancelled => {
                write!(f, "parallel arm cancelled after a sibling failed (reversible boundary)")
            }
            Self::ConfidenceFloorBreached { floor, actual } => {
                write!(
                    f,
                    "stream confidence floor breached: actual {actual:.3} below required {floor:.3}"
                )
            }
            Self::TokenLimitExceeded { limit, used } => {
                write!(f, "stream token limit exceeded: used {used} over limit {limit}")
            }
            Self::NotImplemented(what) => {
                write!(f, "interpreter does not yet support: {what}")
            }
            Self::MissingReturn => write!(f, "function ended without returning a value"),
            Self::ApprovalDenied(action) => {
                write!(f, "approval denied for action `{action}`")
            }
            Self::Timeout { milliseconds } => write!(f, "timed out after {milliseconds}ms"),
            Self::CallerLacksAuthority { action, caller, missing } => {
                write!(f, "action `{action}` requires {missing}, which caller `{caller}` does not hold")
            }
            Self::DispatchFailed(msg) => write!(f, "call dispatch failed: {msg}"),
            Self::Runtime(err) => write!(f, "{err}"),
            Self::Marshal(msg) => write!(f, "marshalling error: {msg}"),
            Self::UnusableReply(msg) => write!(f, "unusable reply: {msg}"),
            Self::Other(m) => f.write_str(m),
        }
    }
}

impl fmt::Display for InterpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}..{}] {}", self.span.start, self.span.end, self.kind)
    }
}
