//! Effect-aware `parallel` scheduling profiles (slice 52d-1).
//!
//! Before a `parallel:` block runs, the runtime computes each arm's
//! **effect profile** — the transitive worst-case cost of every tool /
//! prompt the arm can reach, and whether all of them are reversible —
//! plus the combined profile of the whole block. This is the awareness
//! that effect-aware scheduling (52d) is built on: the combined cost is
//! recorded for observability, and the per-arm reversibility feeds the
//! cancellation×reversibility rule (52d-2) — a branch that has reached a
//! non-reversible tool is past a boundary that must not be cancelled.
//!
//! The per-tool `effect_cost` / `effect_reversible` are pre-computed in
//! the IR (from the effect registry at lower time), so this walk needs
//! no registry access at runtime — it just sums and ANDs.

use corvid_ir::{IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrStmt};
use std::collections::HashSet;

/// The composed effect profile of one `parallel` arm (or a whole
/// block): the summed worst-case cost of the tools/prompts it can reach
/// and whether every one of them is reversible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArmEffectProfile {
    pub cost: f64,
    pub reversible: bool,
}

impl ArmEffectProfile {
    pub fn zero() -> Self {
        Self {
            cost: 0.0,
            reversible: true,
        }
    }

    /// Combine sibling arms: costs SUM (every arm runs — the `parallel`
    /// operator's Sum semantics), reversibility ANDs (the block is
    /// reversible only if every arm is).
    pub fn combine(self, other: Self) -> Self {
        Self {
            cost: self.cost + other.cost,
            reversible: self.reversible && other.reversible,
        }
    }
}

/// Compute one arm's transitive effect profile by walking its call
/// expression. Every reachable tool contributes its pre-computed
/// `effect_cost` (summed) and `effect_reversible` (AND-ed); prompts
/// contribute their `effect_cost` (reversible — an LLM call has no
/// external side effect beyond its cost); agent calls recurse into the
/// callee body. A `visited` set of agent names bounds recursion.
pub fn arm_effect_profile(ir: &IrFile, call: &IrExpr) -> ArmEffectProfile {
    let mut visited = HashSet::new();
    expr_profile(ir, call, &mut visited)
}

/// The combined profile of every arm in a `parallel` block, plus each
/// arm's individual profile in arm order.
pub fn block_effect_profile(ir: &IrFile, calls: &[&IrExpr]) -> (ArmEffectProfile, Vec<ArmEffectProfile>) {
    let per_arm: Vec<ArmEffectProfile> = calls.iter().map(|c| arm_effect_profile(ir, c)).collect();
    let combined = per_arm
        .iter()
        .copied()
        .fold(ArmEffectProfile::zero(), ArmEffectProfile::combine);
    (combined, per_arm)
}

fn expr_profile(ir: &IrFile, expr: &IrExpr, visited: &mut HashSet<String>) -> ArmEffectProfile {
    let mut acc = ArmEffectProfile::zero();
    match &expr.kind {
        IrExprKind::Call {
            kind,
            callee_name,
            args,
        } => {
            match kind {
                IrCallKind::Tool { .. } => {
                    if let Some(tool) = ir.tools.iter().find(|t| &t.name == callee_name) {
                        acc = acc.combine(ArmEffectProfile {
                            cost: tool.effect_cost,
                            reversible: tool.effect_reversible,
                        });
                    }
                }
                IrCallKind::Prompt { .. } => {
                    if let Some(prompt) = ir.prompts.iter().find(|p| &p.name == callee_name) {
                        acc = acc.combine(ArmEffectProfile {
                            cost: prompt.effect_cost,
                            reversible: true,
                        });
                    }
                }
                IrCallKind::Agent { .. } => {
                    if visited.insert(callee_name.clone()) {
                        if let Some(agent) = ir.agents.iter().find(|a| &a.name == callee_name) {
                            acc = acc.combine(block_profile(ir, &agent.body, visited));
                        }
                    }
                }
                _ => {}
            }
            for a in args {
                acc = acc.combine(expr_profile(ir, a, visited));
            }
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            acc = acc.combine(expr_profile(ir, receiver, visited));
            for a in args {
                acc = acc.combine(expr_profile(ir, a, visited));
            }
        }
        IrExprKind::FieldAccess { target, .. } => acc = acc.combine(expr_profile(ir, target, visited)),
        IrExprKind::Index { target, index } => {
            acc = acc.combine(expr_profile(ir, target, visited));
            acc = acc.combine(expr_profile(ir, index, visited));
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            acc = acc.combine(expr_profile(ir, left, visited));
            acc = acc.combine(expr_profile(ir, right, visited));
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            acc = acc.combine(expr_profile(ir, operand, visited));
        }
        IrExprKind::Match { scrutinee, arms } => {
            acc = acc.combine(expr_profile(ir, scrutinee, visited));
            for arm in arms {
                if let Some(g) = &arm.guard {
                    acc = acc.combine(expr_profile(ir, g, visited));
                }
                acc = acc.combine(expr_profile(ir, &arm.body, visited));
            }
        }
        IrExprKind::StructLiteral { fields, spread, .. } => {
            for (_, v) in fields {
                acc = acc.combine(expr_profile(ir, v, visited));
            }
            if let Some(s) = spread {
                acc = acc.combine(expr_profile(ir, s, visited));
            }
        }
        IrExprKind::List { items } => {
            for i in items {
                acc = acc.combine(expr_profile(ir, i, visited));
            }
        }
        IrExprKind::MapLiteral { keys, values } => {
            for e in keys.iter().chain(values.iter()) {
                acc = acc.combine(expr_profile(ir, e, visited));
            }
        }
        IrExprKind::PageNew { items, next_cursor } => {
            acc = acc.combine(expr_profile(ir, items, visited));
            acc = acc.combine(expr_profile(ir, next_cursor, visited));
        }
        IrExprKind::UnwrapGrounded { value }
        | IrExprKind::ResultOk { inner: value }
        | IrExprKind::ResultErr { inner: value }
        | IrExprKind::OptionSome { inner: value }
        | IrExprKind::TryPropagate { inner: value }
        | IrExprKind::TrustBoundary { inner: value } => {
            acc = acc.combine(expr_profile(ir, value, visited));
        }
        IrExprKind::Ask { prompt, .. } => acc = acc.combine(expr_profile(ir, prompt, visited)),
        IrExprKind::TryRetry { body, .. } => acc = acc.combine(expr_profile(ir, body, visited)),
        // Literals, locals, decls, weak/stream combinators, replay:
        // no tool/prompt reachable through them in a parallel arm.
        _ => {}
    }
    acc
}

fn block_profile(ir: &IrFile, block: &IrBlock, visited: &mut HashSet<String>) -> ArmEffectProfile {
    let mut acc = ArmEffectProfile::zero();
    for stmt in &block.stmts {
        acc = acc.combine(stmt_profile(ir, stmt, visited));
    }
    acc
}

fn stmt_profile(ir: &IrFile, stmt: &IrStmt, visited: &mut HashSet<String>) -> ArmEffectProfile {
    match stmt {
        IrStmt::Let { value, .. }
        | IrStmt::Yield { value, .. }
        | IrStmt::Destructure { value, .. }
        | IrStmt::Assign { value, .. } => expr_profile(ir, value, visited),
        IrStmt::Expr { expr, .. } => expr_profile(ir, expr, visited),
        IrStmt::Return { value, .. } => value
            .as_ref()
            .map(|e| expr_profile(ir, e, visited))
            .unwrap_or_else(ArmEffectProfile::zero),
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            // Worst-case: the more expensive branch (Max), plus the
            // condition. Reversibility ANDs both branches conservatively.
            let cond_p = expr_profile(ir, cond, visited);
            let then_p = block_profile(ir, then_block, visited);
            let else_p = else_block
                .as_ref()
                .map(|b| block_profile(ir, b, visited))
                .unwrap_or_else(ArmEffectProfile::zero);
            let branch = ArmEffectProfile {
                cost: then_p.cost.max(else_p.cost),
                reversible: then_p.reversible && else_p.reversible,
            };
            cond_p.combine(branch)
        }
        IrStmt::For { iter, body, .. } => {
            expr_profile(ir, iter, visited).combine(block_profile(ir, body, visited))
        }
        IrStmt::While { cond, body, .. } => {
            expr_profile(ir, cond, visited).combine(block_profile(ir, body, visited))
        }
        IrStmt::Parallel { arms, .. } => arms
            .iter()
            .fold(ArmEffectProfile::zero(), |acc, arm| {
                acc.combine(expr_profile(ir, &arm.call, visited))
            }),
        _ => ArmEffectProfile::zero(),
    }
}
