//! Shared approval-request + decision data shapes.
//!
//! `ApprovalRequest` is the authorization payload the runtime hands
//! to an `Approver`: the label from the `approve Label(args)`
//! statement plus the marshalled args. `ApprovalDecision` is the
//! Approve/Deny outcome the approver returns.
//!
//! Lives in `corvid-runtime-core` so both halves of the wasm/native
//! split can construct + reason about an approval round-trip without
//! pulling the native runtime's tokio / DB / OAuth surface. The
//! `Approver` trait itself stays in `corvid-runtime` until slice
//! 33J7b-3d untangles `RuntimeError` from `replay::ReplayDivergence`
//! (the trait's error type currently routes through the runtime's
//! errors module). `corvid-runtime` re-exports both types through
//! its `approvals` module so existing native consumers see no API
//! change.

use crate::approval_card::ApprovalCard;

/// What the runtime asks the approver to authorize.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    /// Label from the `approve Label(args)` statement.
    pub label: String,
    /// Args from the `approve` statement, marshalled to JSON.
    pub args: Vec<serde_json::Value>,
}

impl ApprovalRequest {
    pub fn card(&self) -> ApprovalCard {
        ApprovalCard::from_request(self)
    }
}

/// The approver's decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}
