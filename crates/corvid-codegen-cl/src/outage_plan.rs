//! Which native calls can raise an outage, and which `try` forms native
//! code catches exactly (recoverable failures TE-3).
//!
//! A tool call and a prompt call are the native calls that can fail with
//! an outage: the runtime holds the outage instead of stopping (see the
//! runtime's `ffi_bridge/outage.rs`). An agent can fail when it can reach
//! one. The expansion pass (`dup_drop/propagation.rs`) hoists each call
//! that can fail into its own statement, followed by a test that unwinds
//! by an ordinary `return`, so the ownership pass releases what is live.
//!
//! A failing call inside a `match` arm or guard is lifted into an agent
//! of its own (`outage_lift.rs`), and its call is checked after the
//! `match`. Some calls still cannot be hoisted: one in a lambda body or a
//! `parallel` statement runs inside a construct the IR has no statement
//! form for. Such a call is
//! followed by a test that stops the program on an outage, which is what
//! native code always did. A `try` that can reach one would then stop
//! where the interpreter catches, so native lowering refuses that `try`
//! by name, and so does the tier picker. Both use this module, so they
//! cannot disagree.

use corvid_ir::{IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrStmt};
use corvid_resolve::DefId;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone)]
pub struct OutagePlan {
    /// Agents that can raise an outage to their caller.
    pub fallible: HashSet<DefId>,
    /// Agents in which an outage cannot unwind exactly, with the reason,
    /// for the refusal of a `try` that reaches them.
    blocked: HashMap<DefId, String>,
    names: HashMap<DefId, String>,
    /// Whether some `try` in the program catches outages natively, so
    /// the runtime must hold them instead of stopping at the call.
    pub catches: bool,
    /// Why each `try ... timeout` that native code does not lower is
    /// refused, keyed by the span of its body (TE-3e).
    nested_timeouts: HashMap<corvid_ast::Span, String>,
}

impl OutagePlan {
    pub fn for_file(ir: &IrFile) -> OutagePlan {
        let mut plan = OutagePlan {
            names: ir.agents.iter().map(|agent| (agent.id, agent.name.clone())).collect(),
            ..OutagePlan::default()
        };
        // Both sets only grow, so this reaches a fixed point.
        loop {
            let mut changed = false;
            for agent in &ir.agents {
                if !plan.fallible.contains(&agent.id) && plan.block_can_fail(&agent.body) {
                    plan.fallible.insert(agent.id);
                    changed = true;
                }
            }
            for agent in &ir.agents {
                if !plan.blocked.contains_key(&agent.id) {
                    if let Some(reason) = plan.block_reason(&agent.body) {
                        plan.blocked.insert(agent.id, reason);
                        changed = true;
                    }
                }
            }
            if !changed {
                plan.nested_timeouts = nested_timeouts(ir, &plan);
                let mut catches = false;
                corvid_ir::for_each_expr(ir, &mut |_, e| catches |= plan.catches_natively(e));
                plan.catches = catches;
                return plan;
            }
        }
    }

    /// Whether this call can raise an outage.
    pub fn call_can_fail(&self, kind: &IrCallKind) -> bool {
        match kind {
            IrCallKind::Tool { .. } | IrCallKind::Prompt { .. } => true,
            IrCallKind::Agent { def_id } => self.fallible.contains(def_id),
            _ => false,
        }
    }

    /// Whether evaluating `expr` can raise an outage anywhere inside it.
    pub fn can_fail(&self, expr: &IrExpr) -> bool {
        let mut found = false;
        corvid_ir::for_each_subexpr(expr, &mut |e| {
            if let IrExprKind::Call { kind, .. } = &e.kind {
                found |= self.call_can_fail(kind);
            }
        });
        found
    }

    /// Whether `expr` is a `try` the expansion pass turns into native
    /// outage handling.
    pub fn catches_natively(&self, expr: &IrExpr) -> bool {
        match &expr.kind {
            IrExprKind::TryRetry { body, timeout_ms, fallback, .. } => {
                (fallback.is_some() || timeout_ms.is_some() || self.can_fail(body))
                    && self.try_refusal_with(body, *timeout_ms).is_none()
            }
            _ => false,
        }
    }

    /// Why a `try` around `body` cannot be lowered natively, if it cannot.
    pub fn try_refusal(&self, body: &IrExpr) -> Option<String> {
        self.expr_reason(body, true)
    }

    /// [`Self::try_refusal`] for a `try` that may carry a `timeout`. A
    /// timed body containing `?` is refused: `?` returns from the agent
    /// from inside the attempt, past the end of its deadline.
    pub fn try_refusal_with(&self, body: &IrExpr, timeout_ms: Option<u64>) -> Option<String> {
        if timeout_ms.is_some() {
            if let Some(reason) = self.nested_timeouts.get(&body.span) {
                return Some(reason.clone());
            }
            let mut propagates = false;
            corvid_ir::for_each_subexpr(body, &mut |e| propagates |= matches!(e.kind, IrExprKind::TryPropagate { .. }));
            if propagates {
                return Some("its body uses `?`, which would leave the attempt with its deadline still set".to_string());
            }
        }
        self.try_refusal(body)
    }

    fn block_can_fail(&self, block: &IrBlock) -> bool {
        block.stmts.iter().any(|stmt| {
            let mut found = false;
            corvid_ir::for_each_stmt_expr(stmt, &mut |e| {
                if let IrExprKind::Call { kind, .. } = &e.kind {
                    found |= self.call_can_fail(kind);
                }
            });
            found
        })
    }

    fn block_reason(&self, block: &IrBlock) -> Option<String> {
        block.stmts.iter().find_map(|stmt| self.stmt_reason(stmt))
    }

    /// The statements the expansion pass hoists from, as it does.
    fn stmt_reason(&self, stmt: &IrStmt) -> Option<String> {
        match stmt {
            IrStmt::Let { value, .. }
            | IrStmt::Expr { expr: value, .. }
            | IrStmt::Return { value: Some(value), .. }
            | IrStmt::Destructure { value, .. } => self.expr_reason(value, true),
            IrStmt::Approve { args, .. } => args.iter().find_map(|arg| self.expr_reason(arg, true)),
            IrStmt::Assign { value, path, .. } => path
                .iter()
                .filter_map(|part| match part {
                    corvid_ir::IrPathSeg::Index(index) => Some(index),
                    corvid_ir::IrPathSeg::Field(_) => None,
                })
                .chain(std::iter::once(value))
                .find_map(|e| self.expr_reason(e, true)),
            IrStmt::If { cond, then_block, else_block, .. } => self
                .expr_reason(cond, true)
                .or_else(|| self.block_reason(then_block))
                .or_else(|| else_block.as_ref().and_then(|b| self.block_reason(b))),
            IrStmt::For { iter, body, .. } => self.expr_reason(iter, true).or_else(|| self.block_reason(body)),
            IrStmt::While { cond, body, .. } => self.expr_reason(cond, true).or_else(|| self.block_reason(body)),
            other => {
                // `parallel`, `atomic` and `yield`: nothing in them is
                // hoisted from.
                let mut reason = None;
                corvid_ir::for_each_stmt_expr(other, &mut |e| {
                    if reason.is_none() {
                        reason = self.node_reason(e, false);
                    }
                });
                reason
            }
        }
    }

    /// The first reason an outage inside `expr` would not unwind exactly
    /// to an enclosing `try`. `hoisted` says whether `expr` is in a
    /// position the expansion pass hoists from.
    fn expr_reason(&self, expr: &IrExpr, hoisted: bool) -> Option<String> {
        if let Some(reason) = self.node_reason(expr, hoisted) {
            return Some(reason);
        }
        match hoisted_children(expr) {
            Some(children) => children.into_iter().find_map(|(child, child_hoisted)| {
                self.expr_reason(child, hoisted && child_hoisted)
            }),
            None => {
                // Nothing inside this form is hoisted from.
                let mut reason = None;
                corvid_ir::for_each_subexpr(expr, &mut |e| {
                    if reason.is_none() && !std::ptr::eq(e, expr) {
                        reason = self.node_reason(e, false);
                    }
                });
                reason
            }
        }
    }

    /// The reason this one node blocks exact unwinding, ignoring its
    /// children.
    fn node_reason(&self, expr: &IrExpr, hoisted: bool) -> Option<String> {
        let IrExprKind::Call { kind, callee_name, .. } = &expr.kind else {
            return None;
        };
        match kind {
            IrCallKind::Agent { def_id } if self.blocked.contains_key(def_id) => {
                let name = self.names.get(def_id).map(String::as_str).unwrap_or(callee_name);
                Some(format!("it calls agent `{name}`, and {}", self.blocked[def_id]))
            }
            _ if !hoisted && self.call_can_fail(kind) => Some(format!(
                "it calls `{callee_name}` inside a lambda or a `parallel` statement, where native code cannot yet unwind an outage"
            )),
            _ => None,
        }
    }
}

/// The direct children of `expr` the expansion pass descends into, each
/// with whether a failing call there is hoisted, or `None` for a form it
/// does not descend into at all. This must match
/// `dup_drop::propagation::expand_expr`; a test holds the two together.
pub(crate) fn hoisted_children(expr: &IrExpr) -> Option<Vec<(&IrExpr, bool)>> {
    use IrExprKind::*;
    Some(match &expr.kind {
        TryPropagate { inner }
        | OptionSome { inner }
        | ResultOk { inner }
        | ResultErr { inner }
        | UnOp { operand: inner, .. }
        | WrappingUnOp { operand: inner, .. }
        | FieldAccess { target: inner, .. }
        | TrustBoundary { inner } => vec![(inner.as_ref(), true)],
        Match { scrutinee, arms } => {
            // An arm body or a guard that is a call is checked after the
            // `match` (failing ones are lifted into such calls by
            // `outage_lift`); anything else in them is not hoisted from.
            let mut out = vec![(scrutinee.as_ref(), true)];
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    out.push((guard, matches!(guard.kind, Call { .. })));
                }
                out.push((&arm.body, matches!(arm.body.kind, Call { .. })));
            }
            out
        }
        BinOp { left, right, .. } | WrappingBinOp { left, right, .. } | Index { target: left, index: right } => {
            vec![(left.as_ref(), true), (right.as_ref(), true)]
        }
        Call { args, .. } | List { items: args } => args.iter().map(|a| (a, true)).collect(),
        MapLiteral { keys, values } => keys.iter().zip(values).flat_map(|(k, v)| [(k, true), (v, true)]).collect(),
        BuiltinMethod { receiver, args, .. } => {
            std::iter::once((receiver.as_ref(), true)).chain(args.iter().map(|a| (a, true))).collect()
        }
        TryRetry { body, fallback, .. } => {
            std::iter::once((body.as_ref(), true)).chain(fallback.iter().map(|f| (f.as_ref(), true))).collect()
        }
        Literal(_) | Local { .. } | Decl { .. } | OptionNone => Vec::new(),
        _ => return None,
    })
}

/// A `try` that catches outages, whatever its clauses.
fn is_catching_try(plan: &OutagePlan, expr: &IrExpr) -> bool {
    matches!(&expr.kind, IrExprKind::TryRetry { body, timeout_ms, fallback, .. }
        if fallback.is_some() || timeout_ms.is_some() || plan.can_fail(body))
}

/// The timed `try`s native code refuses, by the span of their body.
///
/// Nested tries' timeouts are told apart on the interpreter: an outer
/// try's timeout fires as the outer try's, whatever is inside it. Native
/// code keeps one deadline stack and cannot yet say whose deadline a
/// failed call met, so a timed `try` is lowered only where no other
/// `try` can be around it or inside it: its agent is never called from
/// inside a `try`, no `try` encloses it in its agent, and its body
/// reaches no `try`.
fn nested_timeouts(ir: &IrFile, plan: &OutagePlan) -> HashMap<corvid_ast::Span, String> {
    let calls = |expr: &IrExpr, out: &mut HashSet<DefId>| {
        corvid_ir::for_each_subexpr(expr, &mut |e| {
            if let IrExprKind::Call { kind: IrCallKind::Agent { def_id }, .. } = &e.kind {
                out.insert(*def_id);
            }
        });
    };
    let callees: HashMap<DefId, HashSet<DefId>> = ir
        .agents
        .iter()
        .map(|agent| {
            let mut out = HashSet::new();
            corvid_ir::for_each_agent_expr(agent, &mut |e| calls(e, &mut out));
            (agent.id, out)
        })
        .collect();
    let close = |mut set: HashSet<DefId>| {
        loop {
            let more: HashSet<DefId> = set.iter().flat_map(|id| callees.get(id).into_iter().flatten().copied()).collect();
            if more.is_subset(&set) {
                return set;
            }
            set.extend(more);
        }
    };
    // Agents that can run inside some `try`'s body.
    let mut inside = HashSet::new();
    corvid_ir::for_each_expr(ir, &mut |_, e| {
        if is_catching_try(plan, e) {
            // A fallback runs after its `try` is done, so only the body
            // counts as inside it.
            if let IrExprKind::TryRetry { body, .. } = &e.kind {
                calls(body, &mut inside);
            }
        }
    });
    let inside = close(inside);
    // Agents whose body holds a catching `try`, and those that reach one.
    let holds_try: HashSet<DefId> = ir
        .agents
        .iter()
        .filter(|agent| {
            let mut found = false;
            corvid_ir::for_each_agent_expr(agent, &mut |e| found |= is_catching_try(plan, e));
            found
        })
        .map(|agent| agent.id)
        .collect();
    let reaches_try = |body: &IrExpr| {
        let mut called = HashSet::new();
        calls(body, &mut called);
        close(called).iter().any(|id| holds_try.contains(id))
    };
    let mut refused = HashMap::new();
    for agent in &ir.agents {
        corvid_ir::for_each_agent_expr(agent, &mut |e| {
            if let IrExprKind::TryRetry { body, timeout_ms: Some(_), .. } = &e.kind {
                let mut nested_inside = false;
                corvid_ir::for_each_subexpr(body, &mut |inner| {
                    nested_inside |= !std::ptr::eq(inner, body.as_ref()) && is_catching_try(plan, inner)
                });
                let reason = if inside.contains(&agent.id) {
                    Some(format!("agent `{}` can run inside another `try`", agent.name))
                } else if nested_inside || reaches_try(body) {
                    Some("its body can reach another `try`".to_string())
                } else {
                    None
                };
                if let Some(reason) = reason {
                    refused.insert(body.span, format!("{reason}, and native code cannot yet tell nested timeouts apart"));
                }
            }
        });
    }
    // A timed `try` inside another `try`'s body in the same agent.
    corvid_ir::for_each_expr(ir, &mut |_, e| {
        if let IrExprKind::TryRetry { body: outer, .. } = &e.kind {
            if is_catching_try(plan, e) {
                corvid_ir::for_each_subexpr(outer, &mut |inner| {
                    if let IrExprKind::TryRetry { body, timeout_ms: Some(_), .. } = &inner.kind {
                        refused.entry(body.span).or_insert_with(|| {
                            "it sits inside another `try`, and native code cannot yet tell nested timeouts apart".to_string()
                        });
                    }
                });
            }
        }
    });
    refused
}

