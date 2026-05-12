//! The `Approver` trait — what every approval-flow implementation
//! satisfies.
//!
//! The interpreter calls into an `Approver` whenever it encounters
//! a dangerous tool call after an `approve` statement. The approver
//! returns an `ApprovalDecision` (Approve / Deny). Lives in
//! `corvid-runtime-core` so wasm-clean callers (the browser
//! playground in particular) can declare browser-native approver
//! impls — JS-side dialog, an in-page confirm() flow, a custom UI
//! component — that satisfy the same contract as the native
//! `StdinApprover` and `ProgrammaticApprover`.
//!
//! Implementations:
//! - `corvid_runtime::approvals::StdinApprover` — interactive
//!   stdin/stdout approval, the default for `corvid run`. Uses
//!   `tokio::task::spawn_blocking`, native-only.
//! - `corvid_runtime::approvals::ProgrammaticApprover` — closure-
//!   wrapped, used by tests, CI, and embedding hosts. Native-only
//!   today because of bench-latency instrumentation; a future slice
//!   may extract a wasm-friendly closure approver if the playground
//!   needs one separate from its browser-native dialog approver.

use futures::future::BoxFuture;

use crate::approval_request::{ApprovalDecision, ApprovalRequest};
use crate::errors::RuntimeError;

/// Trait every approver implements.
pub trait Approver: Send + Sync {
    fn approve<'a>(
        &'a self,
        req: &'a ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, RuntimeError>>;
}
