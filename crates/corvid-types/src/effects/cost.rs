//! Worst-case cost tree analysis.
//!
//! Walks an agent's body in AST shape (as opposed to the typed IR)
//! and builds a `CostTreeNode` tree whose leaves are tool / prompt
//! decl costs and whose interior nodes represent sequencing (sum)
//! or branching (max). Iteration bounds come from `@budget`
//! annotations + statically-bounded for-loops.
//!
//! `render_cost_tree` formats the tree for REPL / diagnostic output.
//!
//! Extracted from `effects.rs` as part of Phase 20i responsibility
//! decomposition.

use super::analyze::{find_agent, find_connector_operation, find_prompt, find_tool};
use super::{
    ComposedProfile, CostEstimate, CostNodeKind, CostTreeNode, CostWarning, CostWarningKind,
    EffectRegistry,
};
use corvid_ast::{DimensionValue, Effect, EffectConstraint};
use std::collections::{HashMap, HashSet};

mod query;
mod render;
pub use query::{cost_path_for_dimension, numeric_constraint_value};
pub use render::{format_numeric_dimension, render_cost_tree};

// ---- Worst-case cost analysis ----

const COST_DIMENSIONS: [&str; 3] = ["cost", "tokens", "latency_ms"];

/// The most observations a verified provider protocol can make before its
/// declared deadline forces a terminal state (slice 52h-3).
///
/// Deliberately conservative — a budget bound must never UNDER-estimate.
/// A fixed cadence gives `deadline / interval`. An adaptive cadence backs
/// off over time, so its true count is lower; we bound it by the fastest
/// tick the adaptive schedule can produce (1s) rather than modelling the
/// backoff, which over-approximates in the safe direction.
fn worst_case_poll_count(protocol: &corvid_ast::ProviderProtocolDecl) -> u64 {
    let per_poll_secs = match protocol.interval {
        corvid_ast::ProtocolPollInterval::FixedSeconds(secs) => secs.max(1),
        corvid_ast::ProtocolPollInterval::Adaptive => 1,
    };
    // Ceiling division, and always at least one observation: a protocol
    // that never polls could never reach a terminal state.
    protocol
        .deadline_secs
        .div_ceil(per_poll_secs)
        .max(1)
}

pub fn compute_worst_case_cost(
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    registry: &EffectRegistry,
    agent_name: &str,
) -> Option<CostEstimate> {
    let mut analyzer = CostAnalyzer::new(file, resolved, registry);
    analyzer.analyze_agent(agent_name)
}

struct CostAnalyzer<'a> {
    file: &'a corvid_ast::File,
    resolved: &'a corvid_resolve::Resolved,
    registry: &'a EffectRegistry,
    memo: HashMap<String, CostEstimate>,
    visiting: HashSet<String>,
}

impl<'a> CostAnalyzer<'a> {
    fn new(
        file: &'a corvid_ast::File,
        resolved: &'a corvid_resolve::Resolved,
        registry: &'a EffectRegistry,
    ) -> Self {
        Self {
            file,
            resolved,
            registry,
            memo: HashMap::new(),
            visiting: HashSet::new(),
        }
    }

    fn analyze_agent(&mut self, agent_name: &str) -> Option<CostEstimate> {
        if let Some(cached) = self.memo.get(agent_name) {
            return Some(cached.clone());
        }

        let agent = find_agent(self.file, agent_name)?;
        if !self.visiting.insert(agent_name.to_string()) {
            return Some(zero_estimate(agent_name, CostNodeKind::Agent, agent.span));
        }

        let body_estimate = self.analyze_block(&agent.body, agent_name);
        self.visiting.remove(agent_name);

        let result = CostEstimate {
            dimensions: body_estimate.dimensions.clone(),
            tree: CostTreeNode {
                name: agent_name.to_string(),
                kind: CostNodeKind::Agent,
                costs: body_estimate.dimensions,
                children: body_estimate.tree.children,
            },
            warnings: body_estimate.warnings,
            bounded: body_estimate.bounded,
        };
        self.memo.insert(agent_name.to_string(), result.clone());
        Some(result)
    }

    fn analyze_block(&mut self, block: &corvid_ast::Block, agent_name: &str) -> CostEstimate {
        let mut children = Vec::new();
        let mut warnings = Vec::new();
        let mut bounded = true;

        for stmt in &block.stmts {
            let estimate = self.analyze_stmt(stmt, agent_name);
            if !tree_is_zero(&estimate.tree) {
                children.push(estimate.tree);
            }
            warnings.extend(estimate.warnings);
            bounded &= estimate.bounded;
        }

        let tree = sequence_tree("block", CostNodeKind::Sequence, children, block.span);
        CostEstimate {
            dimensions: tree.costs.clone(),
            tree,
            warnings,
            bounded,
        }
    }

    fn analyze_stmt(&mut self, stmt: &corvid_ast::Stmt, agent_name: &str) -> CostEstimate {
        match stmt {
            // Place assignment (45b): the target's index expressions can
            // themselves contain calls, so BOTH sides feed the cost
            // estimate — under-counting the target would let an
            // over-budget call hide inside `xs[classify(t)] = v`.
            corvid_ast::Stmt::Assign {
                target, value, span, ..
            } => {
                let target_est = self.analyze_expr(target, agent_name);
                let value_est = self.analyze_expr(value, agent_name);
                let sequence = sequence_tree(
                    "assign",
                    CostNodeKind::Sequence,
                    vec![
                        wrap_if_needed("target", target_est.tree, target.span()),
                        wrap_if_needed("value", value_est.tree, value.span()),
                    ],
                    *span,
                );
                CostEstimate {
                    dimensions: sequence.costs.clone(),
                    tree: sequence,
                    warnings: collect_warnings(&[target_est.warnings, value_est.warnings]),
                    bounded: target_est.bounded && value_est.bounded,
                }
            }
            corvid_ast::Stmt::Let { value, span, .. }
            | corvid_ast::Stmt::Yield { value, span }
            | corvid_ast::Stmt::Expr { expr: value, span }
            | corvid_ast::Stmt::Approve {
                action: value,
                span,
            } => {
                let mut estimate = self.analyze_expr(value, agent_name);
                estimate.tree.name = "stmt".into();
                estimate.tree.kind = CostNodeKind::Sequence;
                estimate.tree.costs = estimate.dimensions.clone();
                estimate.tree.children =
                    prune_children(std::mem::take(&mut estimate.tree.children));
                estimate.tree = wrap_if_needed("stmt", estimate.tree, *span);
                estimate
            }
            corvid_ast::Stmt::Return {
                value: Some(value),
                span,
            } => {
                let mut estimate = self.analyze_expr(value, agent_name);
                estimate.tree = wrap_if_needed("return", estimate.tree, *span);
                estimate
            }
            corvid_ast::Stmt::Return { value: None, span } => {
                zero_estimate("return", CostNodeKind::Sequence, *span)
            }
            corvid_ast::Stmt::If {
                cond,
                then_block,
                else_block,
                span,
            } => {
                let cond_est = self.analyze_expr(cond, agent_name);
                let then_est = self.analyze_block(then_block, agent_name);
                let else_est = else_block
                    .as_ref()
                    .map(|block| self.analyze_block(block, agent_name))
                    .unwrap_or_else(|| zero_estimate("else", CostNodeKind::Sequence, *span));
                let branch = branch_tree("if", &then_est.tree, &else_est.tree, *span);
                let sequence = sequence_tree(
                    "if",
                    CostNodeKind::Sequence,
                    vec![
                        wrap_if_needed("condition", cond_est.tree, cond.span()),
                        branch,
                    ],
                    *span,
                );
                CostEstimate {
                    dimensions: sequence.costs.clone(),
                    tree: sequence,
                    warnings: collect_warnings(&[
                        cond_est.warnings,
                        then_est.warnings,
                        else_est.warnings,
                    ]),
                    bounded: cond_est.bounded && then_est.bounded && else_est.bounded,
                }
            }
            // `while` (45k) has no static iteration count by
            // construction: the body + condition feed the tree once
            // and the estimate is UNBOUNDED so `@budget` stays sound.
            // (`for` over a static range stays the bounded-loop tool.)
            corvid_ast::Stmt::While { cond, body, span } => {
                let cond_est = self.analyze_expr(cond, agent_name);
                let body_est = self.analyze_block(body, agent_name);
                let tree = sequence_tree(
                    "while",
                    CostNodeKind::Sequence,
                    vec![
                        wrap_if_needed("cond", cond_est.tree, cond.span()),
                        body_est.tree,
                    ],
                    *span,
                );
                let mut warnings = cond_est.warnings;
                warnings.extend(body_est.warnings);
                let zero = tree_is_zero(&tree);
                if !zero {
                    warnings.push(CostWarning {
                        kind: CostWarningKind::UnboundedLoop {
                            agent: agent_name.to_string(),
                            message: format!(
                                "`while` loop at {}..{} — iteration count unknown at compile time",
                                span.start, span.end
                            ),
                        },
                        span: *span,
                    });
                }
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    // A zero-cost while body cannot spend budget no
                    // matter how many times it runs.
                    bounded: zero,
                }
            }
            corvid_ast::Stmt::Destructure { value, span, .. } => {
                let mut estimate = self.analyze_expr(value, agent_name);
                estimate.tree = wrap_if_needed("destructure", estimate.tree, *span);
                estimate
            }
            // Parallel arms (46e): the $ dimension SUMS across arms
            // (both are paid) — a sequence node composes costs by
            // sum, so `@budget` stays sound; wall-clock parallelism
            // is a latency property, not a cost one.
            corvid_ast::Stmt::Parallel { arms, span } => {
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                for arm in arms {
                    let e = self.analyze_expr(&arm.call, agent_name);
                    if !tree_is_zero(&e.tree) {
                        children.push(e.tree);
                    }
                    warnings.extend(e.warnings);
                    bounded &= e.bounded;
                }
                let tree = sequence_tree("parallel", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Stmt::Break { span }
            | corvid_ast::Stmt::Continue { span }
            | corvid_ast::Stmt::Pass { span } => {
                let tree = sequence_tree("flow", CostNodeKind::Sequence, Vec::new(), *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings: Vec::new(),
                    bounded: true,
                }
            }
            corvid_ast::Stmt::For {
                iter, body, span, ..
            } => {
                let iter_est = self.analyze_expr(iter, agent_name);
                if let Some(iterations) = static_loop_bound(iter) {
                    let body_est = self.analyze_block(body, agent_name);
                    let loop_tree = scale_tree(body_est.tree.clone(), iterations, *span);
                    let sequence = sequence_tree(
                        "for",
                        CostNodeKind::Sequence,
                        vec![
                            wrap_if_needed("iter", iter_est.tree, iter.span()),
                            loop_tree.clone(),
                        ],
                        *span,
                    );
                    let mut warnings = iter_est.warnings;
                    warnings.extend(body_est.warnings);
                    CostEstimate {
                        dimensions: sequence.costs.clone(),
                        tree: sequence,
                        warnings,
                        bounded: iter_est.bounded && body_est.bounded,
                    }
                } else {
                    let warning = CostWarning {
                        kind: CostWarningKind::UnboundedLoop {
                            agent: agent_name.to_string(),
                            message: format!(
                                "unbounded loop at {}..{} — static iteration count unknown",
                                span.start, span.end
                            ),
                        },
                        span: *span,
                    };
                    let tree = sequence_tree(
                        "for",
                        CostNodeKind::Sequence,
                        vec![wrap_if_needed("iter", iter_est.tree, iter.span())],
                        *span,
                    );
                    let mut warnings = iter_est.warnings;
                    warnings.push(warning);
                    CostEstimate {
                        dimensions: tree.costs.clone(),
                        tree,
                        warnings,
                        bounded: false,
                    }
                }
            }
        }
    }

    fn analyze_expr(&mut self, expr: &corvid_ast::Expr, agent_name: &str) -> CostEstimate {
        match expr {
            corvid_ast::Expr::Call { callee, args, span } => {
                let mut children = Vec::new();
                let callee_est = self.analyze_expr(callee, agent_name);
                if !tree_is_zero(&callee_est.tree) {
                    children.push(callee_est.tree);
                }
                let mut warnings = callee_est.warnings;
                let mut bounded = callee_est.bounded;
                for arg in args {
                    let arg_est = self.analyze_expr(arg, agent_name);
                    if !tree_is_zero(&arg_est.tree) {
                        children.push(arg_est.tree);
                    }
                    warnings.extend(arg_est.warnings);
                    bounded &= arg_est.bounded;
                }
                if let Some(call_tree) = self.call_cost_tree(callee, agent_name, *span) {
                    children.push(call_tree);
                }
                let tree = sequence_tree("call", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::FieldAccess { target, .. } => self.analyze_expr(target, agent_name),
            corvid_ast::Expr::Index {
                target,
                index,
                span,
            } => {
                let target_est = self.analyze_expr(target, agent_name);
                let index_est = self.analyze_expr(index, agent_name);
                let tree = sequence_tree(
                    "index",
                    CostNodeKind::Sequence,
                    vec![target_est.tree, index_est.tree],
                    *span,
                );
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings: collect_warnings(&[target_est.warnings, index_est.warnings]),
                    bounded: target_est.bounded && index_est.bounded,
                }
            }
            corvid_ast::Expr::BinOp {
                left, right, span, ..
            } => {
                let left_est = self.analyze_expr(left, agent_name);
                let right_est = self.analyze_expr(right, agent_name);
                let tree = sequence_tree(
                    "binop",
                    CostNodeKind::Sequence,
                    vec![left_est.tree, right_est.tree],
                    *span,
                );
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings: collect_warnings(&[left_est.warnings, right_est.warnings]),
                    bounded: left_est.bounded && right_est.bounded,
                }
            }
            corvid_ast::Expr::UnOp { operand, span, .. } => {
                let estimate = self.analyze_expr(operand, agent_name);
                let tree = wrap_if_needed("unop", estimate.tree, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings: estimate.warnings,
                    bounded: estimate.bounded,
                }
            }
            corvid_ast::Expr::Match {
                scrutinee,
                arms,
                span,
            } => {
                // Conservative over-approximation: cost = scrutinee +
                // every arm (guards + bodies). Keeps @budget sound
                // without per-branch max machinery; tighten later if
                // real programs need it.
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                let s_est = self.analyze_expr(scrutinee, agent_name);
                if !tree_is_zero(&s_est.tree) {
                    children.push(s_est.tree);
                }
                warnings.extend(s_est.warnings);
                bounded &= s_est.bounded;
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        let e = self.analyze_expr(g, agent_name);
                        if !tree_is_zero(&e.tree) {
                            children.push(e.tree);
                        }
                        warnings.extend(e.warnings);
                        bounded &= e.bounded;
                    }
                    let e = self.analyze_expr(&arm.body, agent_name);
                    if !tree_is_zero(&e.tree) {
                        children.push(e.tree);
                    }
                    warnings.extend(e.warnings);
                    bounded &= e.bounded;
                }
                let sequence = sequence_tree("match", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: sequence.costs.clone(),
                    tree: sequence,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::StructLiteral {
                fields,
                spread,
                span,
                ..
            } => {
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                for f in fields {
                    if let Some(v) = &f.value {
                        let e = self.analyze_expr(v, agent_name);
                        if !tree_is_zero(&e.tree) {
                            children.push(e.tree);
                        }
                        warnings.extend(e.warnings);
                        bounded &= e.bounded;
                    }
                }
                if let Some(s) = spread {
                    let e = self.analyze_expr(s, agent_name);
                    if !tree_is_zero(&e.tree) {
                        children.push(e.tree);
                    }
                    warnings.extend(e.warnings);
                    bounded &= e.bounded;
                }
                let tree = sequence_tree("struct_literal", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::Lambda { body, span, .. } => {
                // A lambda's body runs once per CALL, and the call
                // count is statically unknown at the creation site.
                // A body with zero static cost keeps the lambda
                // free; any nonzero cost makes the estimate
                // unbounded (the same rule as a loop without a
                // static iteration count) so `@budget` stays sound.
                let body_est = self.analyze_expr(body, agent_name);
                if tree_is_zero(&body_est.tree) && body_est.bounded {
                    let tree =
                        sequence_tree("lambda", CostNodeKind::Sequence, Vec::new(), *span);
                    CostEstimate {
                        dimensions: tree.costs.clone(),
                        tree,
                        warnings: body_est.warnings,
                        bounded: true,
                    }
                } else {
                    let warning = CostWarning {
                        kind: CostWarningKind::UnboundedLoop {
                            agent: agent_name.to_string(),
                            message: format!(
                                "lambda with effectful body at {}..{} — static call count unknown",
                                span.start, span.end
                            ),
                        },
                        span: *span,
                    };
                    let tree = sequence_tree(
                        "lambda",
                        CostNodeKind::Sequence,
                        vec![body_est.tree],
                        *span,
                    );
                    let mut warnings = body_est.warnings;
                    warnings.push(warning);
                    CostEstimate {
                        dimensions: tree.costs.clone(),
                        tree,
                        warnings,
                        bounded: false,
                    }
                }
            }
            corvid_ast::Expr::MapLiteral { entries, span } => {
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                for (k, v) in entries {
                    for e in [k, v] {
                        let estimate = self.analyze_expr(e, agent_name);
                        if !tree_is_zero(&estimate.tree) {
                            children.push(estimate.tree);
                        }
                        warnings.extend(estimate.warnings);
                        bounded &= estimate.bounded;
                    }
                }
                let sequence =
                    sequence_tree("map", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: sequence.costs.clone(),
                    tree: sequence,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::List { items, span } => {
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                for item in items {
                    let estimate = self.analyze_expr(item, agent_name);
                    if !tree_is_zero(&estimate.tree) {
                        children.push(estimate.tree);
                    }
                    warnings.extend(estimate.warnings);
                    bounded &= estimate.bounded;
                }
                let tree = sequence_tree("list", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::TryPropagate { inner, .. } => self.analyze_expr(inner, agent_name),
            corvid_ast::Expr::TrustBoundary { inner, .. } => self.analyze_expr(inner, agent_name),
            corvid_ast::Expr::TryRetry { body, span, .. } => {
                let estimate = self.analyze_expr(body, agent_name);
                let tree = wrap_if_needed("retry", estimate.tree, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings: estimate.warnings,
                    bounded: estimate.bounded,
                }
            }
            corvid_ast::Expr::Replay {
                trace,
                arms,
                else_body,
                span,
            } => {
                // Cost accounting for `replay` blocks: replay reads a
                // recorded trace and substitutes its values — the
                // block itself issues no live LLM / tool calls. Cost
                // for replay is therefore the cost of whichever arm
                // body ends up executing, approximated as the max
                // across arms. Subexpression costs are still
                // analyzed so e.g. an arm body calling a priced
                // tool shows up in the estimate.
                //
                // Exact arm-selection + pattern cost-typing lands
                // with 21-inv-E-3; until then we take the union
                // (sequence) of every arm's estimate as a
                // conservative upper bound.
                let mut children = Vec::new();
                let mut warnings = Vec::new();
                let mut bounded = true;
                let trace_est = self.analyze_expr(trace, agent_name);
                if !tree_is_zero(&trace_est.tree) {
                    children.push(trace_est.tree);
                }
                warnings.extend(trace_est.warnings);
                bounded &= trace_est.bounded;
                for arm in arms {
                    let arm_est = self.analyze_expr(&arm.body, agent_name);
                    if !tree_is_zero(&arm_est.tree) {
                        children.push(arm_est.tree);
                    }
                    warnings.extend(arm_est.warnings);
                    bounded &= arm_est.bounded;
                }
                let else_est = self.analyze_expr(else_body, agent_name);
                if !tree_is_zero(&else_est.tree) {
                    children.push(else_est.tree);
                }
                warnings.extend(else_est.warnings);
                bounded &= else_est.bounded;
                let tree = sequence_tree("replay", CostNodeKind::Sequence, children, *span);
                CostEstimate {
                    dimensions: tree.costs.clone(),
                    tree,
                    warnings,
                    bounded,
                }
            }
            corvid_ast::Expr::Literal { .. } | corvid_ast::Expr::Ident { .. } => {
                zero_estimate("expr", CostNodeKind::Sequence, expr.span())
            }
        }
    }

    fn call_cost_tree(
        &mut self,
        callee: &corvid_ast::Expr,
        agent_name: &str,
        span: corvid_ast::Span,
    ) -> Option<CostTreeNode> {
        let corvid_ast::Expr::Ident { name, .. } = callee else {
            return None;
        };
        let binding = self.resolved.bindings.get(&name.span)?;
        let corvid_resolve::Binding::Decl(def_id) = binding else {
            return None;
        };
        let entry = self.resolved.symbols.get(*def_id);
        match entry.kind {
            corvid_resolve::DeclKind::Tool => {
                if let Some(tool) = find_tool(self.file, &entry.name) {
                    return Some(effect_node_for_decl(
                        &tool.name.name,
                        CostNodeKind::Tool,
                        &tool.effect_row,
                        tool.effect,
                        self.registry,
                        span,
                    ));
                }
                // A connector `operation` also resolves as a Tool but
                // lives inside `Decl::Connector` (slice 52g-3a), so
                // `find_tool` misses it. Costing it here is what keeps
                // `@budget` sound for connector-using programs.
                let op = find_connector_operation(self.file, &entry.name)?;
                let node = effect_node_for_decl(
                    &op.name.name,
                    CostNodeKind::Tool,
                    &op.effect_row,
                    op.effect,
                    self.registry,
                    span,
                );
                // A verified provider protocol does not make ONE request
                // — it submits and then polls until a terminal state or
                // the declared deadline. The worst case is knowable
                // statically from the declaration, so the budget bounds
                // polling at COMPILE time (slice 52h-3): scale by
                // 1 submit + the worst-case poll count, reusing the same
                // `scale_tree` a bounded loop uses.
                match &op.protocol {
                    Some(protocol) => {
                        let requests = 1 + worst_case_poll_count(protocol);
                        Some(scale_tree(node, requests, span))
                    }
                    None => Some(node),
                }
            }
            corvid_resolve::DeclKind::Prompt => {
                let prompt = find_prompt(self.file, &entry.name)?;
                Some(effect_node_for_prompt(prompt, self.registry))
            }
            corvid_resolve::DeclKind::Agent => self
                .analyze_agent(&entry.name)
                .map(|estimate| rename_tree(estimate.tree, &entry.name)),
            _ => {
                let _ = agent_name;
                None
            }
        }
    }
}

fn zero_estimate(name: &str, kind: CostNodeKind, _span: corvid_ast::Span) -> CostEstimate {
    let costs = zero_costs();
    CostEstimate {
        dimensions: costs.clone(),
        tree: CostTreeNode {
            name: name.to_string(),
            kind,
            costs,
            children: Vec::new(),
        },
        warnings: Vec::new(),
        bounded: true,
    }
}

fn zero_costs() -> HashMap<String, f64> {
    COST_DIMENSIONS
        .iter()
        .map(|dim| ((*dim).to_string(), 0.0))
        .collect()
}

fn sequence_tree(
    name: &str,
    kind: CostNodeKind,
    children: Vec<CostTreeNode>,
    _span: corvid_ast::Span,
) -> CostTreeNode {
    let children = prune_children(children);
    let mut costs = zero_costs();
    for child in &children {
        add_costs(&mut costs, &child.costs);
    }
    CostTreeNode {
        name: name.to_string(),
        kind,
        costs,
        children,
    }
}

fn branch_tree(
    name: &str,
    then_tree: &CostTreeNode,
    else_tree: &CostTreeNode,
    _span: corvid_ast::Span,
) -> CostTreeNode {
    let mut costs = zero_costs();
    for dim in COST_DIMENSIONS {
        let then_cost = then_tree.costs.get(dim).copied().unwrap_or(0.0);
        let else_cost = else_tree.costs.get(dim).copied().unwrap_or(0.0);
        costs.insert(dim.to_string(), then_cost.max(else_cost));
    }
    CostTreeNode {
        name: name.to_string(),
        kind: CostNodeKind::Branch,
        costs,
        children: prune_children(vec![then_tree.clone(), else_tree.clone()]),
    }
}

fn wrap_if_needed(name: &str, tree: CostTreeNode, span: corvid_ast::Span) -> CostTreeNode {
    if tree.name == name && matches!(tree.kind, CostNodeKind::Sequence) {
        tree
    } else {
        sequence_tree(name, CostNodeKind::Sequence, vec![tree], span)
    }
}

fn tree_is_zero(tree: &CostTreeNode) -> bool {
    tree.costs.values().all(|value| *value <= f64::EPSILON) && tree.children.is_empty()
}

fn prune_children(children: Vec<CostTreeNode>) -> Vec<CostTreeNode> {
    children
        .into_iter()
        .filter(|child| !tree_is_zero(child))
        .collect()
}

fn add_costs(target: &mut HashMap<String, f64>, source: &HashMap<String, f64>) {
    for dim in COST_DIMENSIONS {
        let next =
            target.get(dim).copied().unwrap_or(0.0) + source.get(dim).copied().unwrap_or(0.0);
        target.insert(dim.to_string(), next);
    }
}

fn scale_tree(tree: CostTreeNode, iterations: u64, _span: corvid_ast::Span) -> CostTreeNode {
    let name = tree.name.clone();
    let costs = tree
        .costs
        .iter()
        .map(|(dim, value)| (dim.clone(), value * iterations as f64))
        .collect();
    CostTreeNode {
        name,
        kind: CostNodeKind::Loop {
            iterations: Some(iterations),
        },
        costs,
        children: vec![tree],
    }
}

fn collect_warnings(chunks: &[Vec<CostWarning>]) -> Vec<CostWarning> {
    let mut warnings = Vec::new();
    for chunk in chunks {
        warnings.extend(chunk.clone());
    }
    warnings
}

fn static_loop_bound(expr: &corvid_ast::Expr) -> Option<u64> {
    match expr {
        corvid_ast::Expr::List { items, .. } => Some(items.len() as u64),
        _ => None,
    }
}

fn effect_node_for_decl(
    name: &str,
    kind: CostNodeKind,
    effect_row: &corvid_ast::EffectRow,
    legacy_effect: Effect,
    registry: &EffectRegistry,
    _span: corvid_ast::Span,
) -> CostTreeNode {
    let mut effect_names: Vec<&str> = effect_row
        .effects
        .iter()
        .map(|effect| effect.name.name.as_str())
        .collect();
    if matches!(legacy_effect, Effect::Dangerous) {
        effect_names.push("dangerous");
    }
    let profile = registry.compose(&effect_names);
    CostTreeNode {
        name: name.to_string(),
        kind,
        costs: numeric_dimensions_from_profile(&profile),
        children: Vec::new(),
    }
}

fn effect_node_for_prompt(
    prompt: &corvid_ast::PromptDecl,
    registry: &EffectRegistry,
) -> CostTreeNode {
    let effect_names: Vec<&str> = prompt
        .effect_row
        .effects
        .iter()
        .map(|effect| effect.name.name.as_str())
        .collect();
    let profile = registry.compose(&effect_names);
    let mut costs = numeric_dimensions_from_profile(&profile);
    // Structured-output auto-repair (`with repair N`) may re-ask
    // the model up to N extra times, and every attempt spends real
    // cost/tokens/latency — the interpreter accumulates the failed
    // attempts' cost onto the result. The WORST-CASE analysis must
    // match, or `@budget` verifies a bound the runtime can exceed:
    // a $0.30 prompt with `repair 2` is a $0.90 worst case.
    if let Some(repair) = prompt.stream.repair {
        let multiplier = 1.0 + repair as f64;
        for value in costs.values_mut() {
            *value *= multiplier;
        }
    }
    // Multi-dispatch clauses spend per MODEL CALL, and the runtime
    // sums those costs onto the result — the worst-case analysis
    // must match or `@budget` verifies a bound the runtime exceeds:
    //   - ensemble: every member fires concurrently (+1 when a
    //     disagreement-escalation member is configured);
    //   - progressive: worst case every stage runs;
    //   - adversarial: propose + challenge + adjudicate = 3 calls.
    // `route` and `rollout` pick ONE model per call: 1x.
    let dispatch_calls: f64 = if let Some(ensemble) = &prompt.ensemble {
        (ensemble.models.len() + usize::from(ensemble.disagreement_escalation.is_some())) as f64
    } else if let Some(progressive) = &prompt.progressive {
        progressive.stages.len() as f64
    } else if prompt.adversarial.is_some() {
        3.0
    } else {
        1.0
    };
    if dispatch_calls > 1.0 {
        for value in costs.values_mut() {
            *value *= dispatch_calls;
        }
    }
    // A judged output guard (slice 50l) runs an LLM judge after
    // every attempt — one extra model call per attempt, worst-case
    // approximated at the prompt's own per-call cost.
    if prompt.stream.judged.is_some() {
        for value in costs.values_mut() {
            *value *= 2.0;
        }
    }
    CostTreeNode {
        name: prompt.name.name.clone(),
        kind: CostNodeKind::Prompt,
        costs,
        children: Vec::new(),
    }
}

fn numeric_dimensions_from_profile(profile: &ComposedProfile) -> HashMap<String, f64> {
    let mut costs = zero_costs();
    for dim in COST_DIMENSIONS {
        let value = match profile.dimensions.get(dim) {
            Some(DimensionValue::Cost(value)) => *value,
            Some(DimensionValue::Number(value)) => *value,
            _ => 0.0,
        };
        costs.insert(dim.to_string(), value);
    }
    costs
}

fn rename_tree(mut tree: CostTreeNode, name: &str) -> CostTreeNode {
    tree.name = name.to_string();
    tree
}
