//! Lexical environment: `LocalId` → `Value`.
//!
//! v0.5 treats each function (agent/prompt) body as a single flat scope —
//! matching the resolver's current model. When we introduce closures,
//! this type gains a frame stack.

use crate::value::Value;
use corvid_resolve::LocalId;
use std::collections::HashMap;

// Parallel arms clone their environment. Share the consumed flag so one
// decision cannot authorize two dispatches of the same reviewed arguments.
#[derive(Debug)]
struct PythonApproval {
    label: String,
    args: Vec<serde_json::Value>,
    consumed: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone, Default)]
pub struct Env {
    locals: HashMap<LocalId, Value>,
    python_approvals: Vec<std::sync::Arc<PythonApproval>>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn approval_depth(&self) -> usize {
        self.python_approvals.len()
    }

    pub(crate) fn leave_approval_scope(&mut self, depth: usize) {
        self.python_approvals.truncate(depth);
    }

    pub(crate) fn record_python_approval(&mut self, label: &str, args: Vec<serde_json::Value>) {
        if label.starts_with("python:") {
            self.python_approvals.push(std::sync::Arc::new(PythonApproval {
                label: label.into(), args, consumed: false.into(),
            }));
        }
    }

    pub(crate) fn consume_python_approval(&self, label: &str, args: &[serde_json::Value]) -> bool {
        self.python_approvals.iter().rev().any(|approval| {
            approval.label == label && approval.args == args
                && !approval.consumed.swap(true, std::sync::atomic::Ordering::AcqRel)
        })
    }

    /// Bind `id` to `value`. Shadows any prior binding at the same id
    /// (consistent with Corvid's assignment semantics).
    pub fn bind(&mut self, id: LocalId, value: Value) {
        self.locals.insert(id, value);
    }

    /// Return a clone of the value bound at `id`, or `None` if unbound.
    pub fn lookup(&self, id: LocalId) -> Option<Value> {
        self.locals.get(&id).cloned()
    }

    /// Snapshot every binding — the BY-VALUE capture set for a
    /// closure (slice 45j). Values clone; heap cells share, so a
    /// captured list still observes later mutations through any
    /// handle.
    pub fn entries_snapshot(&self) -> Vec<(LocalId, Value)> {
        self.locals.iter().map(|(k, v)| (*k, v.clone())).collect()
    }
}
