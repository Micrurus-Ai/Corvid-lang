//! Call-graph effect analyzer.
//!
//! Walks each agent body, collects the effects used by every tool /
//! prompt / agent it calls, composes those per dimension via the
//! registry's archetype rules, and records any constraint violations.
//! `collect_body_capabilities` does the parallel walk for the
//! `capability` dimension (Max-composed through the call graph to
//! gate prompt dispatch).
//!
//! Extracted from `effects.rs` as part of Phase 20i responsibility
//! decomposition.

use super::compose::capability_max;
use super::{ComposedProfile, ConstraintViolation, EffectRegistry};
use corvid_ast::DimensionValue;
use corvid_resolve::DefId;
use std::collections::{HashMap, HashSet};

// ---- Call-graph effect analyzer ----

/// Per-agent inferred effect profile: the union of all effects used
/// by tools/prompts/agents called in the agent's body.
#[derive(Debug, Clone)]
pub struct AgentEffectSummary {
    pub agent_def_id: DefId,
    pub agent_name: String,
    pub declared_effects: Vec<String>,
    pub inferred_effects: Vec<String>,
    pub composed: ComposedProfile,
    pub violations: Vec<ConstraintViolation>,
}

/// Analyze all agents in the file and produce per-agent effect summaries.
pub fn analyze_effects(
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    registry: &EffectRegistry,
) -> Vec<AgentEffectSummary> {
    let mut direct = HashMap::new();
    let mut agents = Vec::new();
    for decl in &file.decls {
        let corvid_ast::Decl::Agent(agent) = decl else { continue };
        let Some(id) = resolved.symbols.lookup_def(&agent.name.name) else { continue };
        let mut effects = BodyEffects::default();
        collect_body_effects(&agent.body, file, resolved, &mut effects);
        direct.insert(id, effects);
        agents.push((id, agent));
    }

    let mut summaries = Vec::new();
    for (def_id, agent) in agents {
        let mut pending = vec![def_id];
        let mut seen = HashSet::new();
        let mut effect_names = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(body) = direct.get(&id) {
                effect_names.extend(body.names.iter().cloned());
                pending.extend(body.agents.iter().copied());
            }
        }
        effect_names.sort();
        effect_names.dedup();

        // Compose the dimensional profile.
        let refs: Vec<&str> = effect_names.iter().map(|s| s.as_str()).collect();
        let mut composed = registry.compose(&refs);

        // Phase 20h: collect capability requirements from every
        // prompt call in the body and Max-compose them into the
        // agent's `capability` dimension. A prompt's `requires:
        // <level>` clause sets the minimum model capability its
        // dispatch needs; the agent-level composed requirement is
        // the strictest capability any call needs.
        let mut capabilities: Vec<String> = Vec::new();
        collect_body_capabilities(&agent.body, file, resolved, &mut capabilities);
        if !capabilities.is_empty() {
            let mut iter = capabilities.into_iter();
            let mut acc = iter.next().unwrap();
            for next in iter {
                acc = capability_max(&acc, &next).to_string();
            }
            composed
                .dimensions
                .insert("capability".into(), DimensionValue::Name(acc));
        }

        // Check constraints.
        let violations = registry.check_constraints(&composed, &agent.constraints);

        // Declared effects from the agent's `uses` clause.
        let declared: Vec<String> = agent
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect();

        summaries.push(AgentEffectSummary {
            agent_def_id: def_id,
            agent_name: agent.name.name.clone(),
            declared_effects: declared,
            inferred_effects: effect_names,
            composed,
            violations,
        });
    }

    summaries
}

#[derive(Default)]
struct BodyEffects {
    names: Vec<String>,
    agents: Vec<DefId>,
}

fn collect_body_effects(
    block: &corvid_ast::Block,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    effects: &mut BodyEffects,
) {
    for stmt in &block.stmts {
        collect_stmt_effects(stmt, file, resolved, effects);
    }
}

/// Phase 20h: walk a block and collect the `capability_required`
/// string from every prompt call. Mirrors `collect_body_effects`'s
/// recursion pattern but looks at prompt-level attributes instead of
/// effect-row references.
fn collect_body_capabilities(
    block: &corvid_ast::Block,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    caps: &mut Vec<String>,
) {
    for stmt in &block.stmts {
        collect_stmt_capabilities(stmt, file, resolved, caps);
    }
}

fn collect_stmt_capabilities(
    stmt: &corvid_ast::Stmt,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    caps: &mut Vec<String>,
) {
    match stmt {
        corvid_ast::Stmt::Let { value, .. } => {
            collect_expr_capabilities(value, file, resolved, caps);
        }
        corvid_ast::Stmt::Return { value: Some(v), .. } => {
            collect_expr_capabilities(v, file, resolved, caps);
        }
        corvid_ast::Stmt::Return { value: None, .. } => {}
        corvid_ast::Stmt::Yield { value, .. } => {
            collect_expr_capabilities(value, file, resolved, caps);
        }
        corvid_ast::Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            collect_expr_capabilities(cond, file, resolved, caps);
            collect_body_capabilities(then_block, file, resolved, caps);
            if let Some(eb) = else_block {
                collect_body_capabilities(eb, file, resolved, caps);
            }
        }
        corvid_ast::Stmt::For { iter, body, .. } => {
            collect_expr_capabilities(iter, file, resolved, caps);
            collect_body_capabilities(body, file, resolved, caps);
        }
        corvid_ast::Stmt::Atomic { body, .. } => {
            collect_body_capabilities(body, file, resolved, caps);
        }
        corvid_ast::Stmt::While { cond, body, .. } => {
            collect_expr_capabilities(cond, file, resolved, caps);
            collect_body_capabilities(body, file, resolved, caps);
        }
        corvid_ast::Stmt::Destructure { value, .. } => {
            collect_expr_capabilities(value, file, resolved, caps);
        }
        corvid_ast::Stmt::Parallel { arms, .. } => {
            for arm in arms {
                collect_expr_capabilities(&arm.call, file, resolved, caps);
            }
        }
        corvid_ast::Stmt::Break { .. }
        | corvid_ast::Stmt::Continue { .. }
        | corvid_ast::Stmt::Pass { .. } => {}
        corvid_ast::Stmt::Expr { expr, .. } => {
            collect_expr_capabilities(expr, file, resolved, caps);
        }
        _ => {}
    }
}

fn collect_expr_capabilities(
    expr: &corvid_ast::Expr,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    caps: &mut Vec<String>,
) {
    match expr {
        corvid_ast::Expr::Call { callee, args, .. } => {
            if let corvid_ast::Expr::Ident { name: ident, .. } = callee.as_ref() {
                if let Some(corvid_resolve::Binding::Decl(def_id)) =
                    resolved.bindings.get(&ident.span)
                {
                    let entry = resolved.symbols.get(*def_id);
                    if matches!(entry.kind, corvid_resolve::DeclKind::Prompt) {
                        if let Some(prompt) = find_prompt(file, &entry.name) {
                            if let Some(req) = &prompt.capability_required {
                                caps.push(req.name.clone());
                            }
                        }
                    }
                    if matches!(entry.kind, corvid_resolve::DeclKind::Agent) {
                        if let Some(agent) = find_agent(file, &entry.name) {
                            collect_body_capabilities(&agent.body, file, resolved, caps);
                        }
                    }
                }
            }
            collect_expr_capabilities(callee, file, resolved, caps);
            for arg in args {
                collect_expr_capabilities(arg, file, resolved, caps);
            }
        }
        corvid_ast::Expr::FieldAccess { target, .. } => {
            collect_expr_capabilities(target, file, resolved, caps);
        }
        corvid_ast::Expr::BinOp { left, right, .. } => {
            collect_expr_capabilities(left, file, resolved, caps);
            collect_expr_capabilities(right, file, resolved, caps);
        }
        corvid_ast::Expr::UnOp { operand, .. } => {
            collect_expr_capabilities(operand, file, resolved, caps);
        }
        _ => {}
    }
}

fn collect_stmt_effects(
    stmt: &corvid_ast::Stmt,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    effects: &mut BodyEffects,
) {
    match stmt {
        corvid_ast::Stmt::Let { value, .. } => {
            collect_expr_effects(value, file, resolved, effects);
        }
        corvid_ast::Stmt::Return { value: Some(v), .. } => {
            collect_expr_effects(v, file, resolved, effects);
        }
        corvid_ast::Stmt::Return { value: None, .. } => {}
        corvid_ast::Stmt::Yield { value, .. } => {
            collect_expr_effects(value, file, resolved, effects);
        }
        corvid_ast::Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            collect_expr_effects(cond, file, resolved, effects);
            collect_body_effects(then_block, file, resolved, effects);
            if let Some(eb) = else_block {
                collect_body_effects(eb, file, resolved, effects);
            }
        }
        corvid_ast::Stmt::For { iter, body, .. } => {
            collect_expr_effects(iter, file, resolved, effects);
            collect_body_effects(body, file, resolved, effects);
        }
        corvid_ast::Stmt::Atomic { body, .. } => {
            collect_body_effects(body, file, resolved, effects);
        }
        corvid_ast::Stmt::While { cond, body, .. } => {
            collect_expr_effects(cond, file, resolved, effects);
            collect_body_effects(body, file, resolved, effects);
        }
        corvid_ast::Stmt::Destructure { value, .. } => {
            collect_expr_effects(value, file, resolved, effects);
        }
        corvid_ast::Stmt::Parallel { arms, .. } => {
            for arm in arms {
                collect_expr_effects(&arm.call, file, resolved, effects);
            }
        }
        corvid_ast::Stmt::Break { .. }
        | corvid_ast::Stmt::Continue { .. }
        | corvid_ast::Stmt::Pass { .. } => {}
        corvid_ast::Stmt::Approve { action, .. } => {
            effects.names.push("approve".into());
            if let corvid_ast::Expr::Call { args, .. } = action {
                for arg in args {
                    collect_expr_effects(arg, file, resolved, effects);
                }
            } else {
                collect_expr_effects(action, file, resolved, effects);
            }
        }
        corvid_ast::Stmt::Expr { expr, .. } => {
            collect_expr_effects(expr, file, resolved, effects);
        }
        corvid_ast::Stmt::Assign { target, value, .. } => {
            collect_expr_effects(target, file, resolved, effects);
            collect_expr_effects(value, file, resolved, effects);
        }
    }
}

fn collect_expr_effects(
    expr: &corvid_ast::Expr,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    effects: &mut BodyEffects,
) {
    match expr {
        corvid_ast::Expr::Call { callee, args, .. } => {
            // `Shelf.save(row)` uses the built-in database effects (B09).
            if let Some((_, op)) = crate::table_call_target(callee, file, &resolved.symbols, &resolved.bindings) {
                effects.names.push(op.effect().into());
            }
            if crate::table_ops::table_query_call(callee, args, file, &resolved.symbols, &resolved.bindings).is_some() {
                effects.names.push(crate::TableOp::Get.effect().into());
            }
            if let Some((import, _)) = python_call_target(callee, file, &resolved.symbols, &resolved.bindings) {
                effects.names.extend(import.effect_row.effects.iter().map(|e| e.name.name.clone()));
            }
            // Check if callee resolves to a tool/prompt/agent with effects.
            if let corvid_ast::Expr::Ident { span, .. } = &**callee {
                if let Some(corvid_resolve::Binding::Decl(def_id)) = resolved.bindings.get(span) {
                    let entry = resolved.symbols.get(*def_id);
                    match entry.kind {
                        corvid_resolve::DeclKind::Tool => {
                            // Find the tool declaration and collect its effect row.
                            if let Some(tool) = find_tool(file, &entry.name) {
                                for eff in &tool.effect_row.effects {
                                    effects.names.push(eff.name.name.clone());
                                }
                                // Legacy: dangerous → implicit "dangerous" effect
                                if matches!(tool.effect, corvid_ast::Effect::Dangerous) {
                                    effects.names.push("dangerous".into());
                                }
                            }
                        }
                        corvid_resolve::DeclKind::Prompt => {
                            if let Some(prompt) = find_prompt(file, &entry.name) {
                                for eff in &prompt.effect_row.effects {
                                    effects.names.push(eff.name.name.clone());
                                }
                            }
                        }
                        corvid_resolve::DeclKind::Agent => {
                            if let Some(agent) = find_agent(file, &entry.name) {
                                effects.agents.push(*def_id);
                                for eff in &agent.effect_row.effects {
                                    effects.names.push(eff.name.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if matches!(
                    resolved.bindings.get(span),
                    Some(corvid_resolve::Binding::BuiltIn(
                        corvid_resolve::BuiltIn::Ask | corvid_resolve::BuiltIn::Choose
                    ))
                ) {
                    effects.names.push("human".into());
                }
            }
            collect_expr_effects(callee, file, resolved, effects);
            for arg in args {
                collect_expr_effects(arg, file, resolved, effects);
            }
        }
        corvid_ast::Expr::FieldAccess { target, .. } => {
            collect_expr_effects(target, file, resolved, effects);
        }
        corvid_ast::Expr::Index { target, index, .. } => {
            collect_expr_effects(target, file, resolved, effects);
            collect_expr_effects(index, file, resolved, effects);
        }
        corvid_ast::Expr::BinOp { left, right, .. } => {
            collect_expr_effects(left, file, resolved, effects);
            collect_expr_effects(right, file, resolved, effects);
        }
        corvid_ast::Expr::UnOp { operand, .. } => {
            collect_expr_effects(operand, file, resolved, effects);
        }
        corvid_ast::Expr::List { items, .. } => {
            for item in items {
                collect_expr_effects(item, file, resolved, effects);
            }
        }
        corvid_ast::Expr::TryPropagate { inner, .. } => {
            collect_expr_effects(inner, file, resolved, effects);
        }
        corvid_ast::Expr::TryRetry { body, .. } => {
            collect_expr_effects(body, file, resolved, effects);
        }
        corvid_ast::Expr::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_expr_effects(key, file, resolved, effects);
                collect_expr_effects(value, file, resolved, effects);
            }
        }
        corvid_ast::Expr::Match { scrutinee, arms, .. } => {
            collect_expr_effects(scrutinee, file, resolved, effects);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    collect_expr_effects(guard, file, resolved, effects);
                }
                collect_expr_effects(&arm.body, file, resolved, effects);
            }
        }
        corvid_ast::Expr::StructLiteral { fields, spread, .. } => {
            for field in fields {
                if let Some(value) = &field.value {
                    collect_expr_effects(value, file, resolved, effects);
                }
            }
            if let Some(base) = spread {
                collect_expr_effects(base, file, resolved, effects);
            }
        }
        corvid_ast::Expr::TrustBoundary { inner, .. } | corvid_ast::Expr::Lambda { body: inner, .. } => {
            collect_expr_effects(inner, file, resolved, effects);
        }
        corvid_ast::Expr::Replay { trace, arms, else_body, .. } => {
            collect_expr_effects(trace, file, resolved, effects);
            for arm in arms {
                collect_expr_effects(&arm.body, file, resolved, effects);
            }
            collect_expr_effects(else_body, file, resolved, effects);
        }
        corvid_ast::Expr::Ident { .. } | corvid_ast::Expr::Literal { .. } => {}
    }
}

/// Resolve a typed Python member using the receiver's lexical binding.
pub(crate) fn python_call_target<'a>(
    callee: &corvid_ast::Expr,
    file: &'a corvid_ast::File,
    symbols: &corvid_resolve::SymbolTable,
    bindings: &std::collections::HashMap<corvid_ast::Span, corvid_resolve::Binding>,
) -> Option<(&'a corvid_ast::ImportDecl, &'a corvid_ast::PythonFunctionSignature)> {
    use corvid_ast::{Decl, Expr, ImportSource};
    use corvid_resolve::{Binding, DeclKind};

    let Expr::FieldAccess { target, field, .. } = callee else { return None };
    let Expr::Ident { name, .. } = target.as_ref() else { return None };
    let Binding::Decl(id) = bindings.get(&name.span)? else { return None };
    let entry = symbols.get(*id);
    if entry.kind != DeclKind::Import {
        return None;
    }
    file.decls.iter().find_map(|decl| {
        let Decl::Import(import) = decl else { return None };
        if import.span != entry.span || import.source != ImportSource::Python {
            return None;
        }
        let function = import.python_functions.iter().find(|f| f.name.name == field.name)?;
        Some((import, function))
    })
}

pub(super) fn find_tool<'a>(
    file: &'a corvid_ast::File,
    name: &str,
) -> Option<&'a corvid_ast::ToolDecl> {
    file.decls.iter().find_map(|d| match d {
        corvid_ast::Decl::Tool(t) if t.name.name == name => Some(t),
        _ => None,
    })
}

/// Find a connector `operation` by name (slice 52h-3).
///
/// A connector operation resolves as `DeclKind::Tool` (slice 52g-3a) but
/// lives inside a `Decl::Connector`, NOT a `Decl::Tool` — so `find_tool`
/// cannot see it. Without this lookup the cost analyzer silently dropped
/// every connector call, and `@budget` was unsound for connector-using
/// programs: a `$2.00` operation contributed `$0`. Commodity features
/// must compose with the moat, so cost analysis has to see them.
pub(super) fn find_connector_operation<'a>(
    file: &'a corvid_ast::File,
    name: &str,
) -> Option<&'a corvid_ast::OperationDecl> {
    file.decls.iter().find_map(|d| match d {
        corvid_ast::Decl::Connector(c) => c.operations.iter().find(|op| op.name.name == name),
        _ => None,
    })
}

/// The connector an operation belongs to. Reliability (`retry:`) is
/// declared on the CONNECTOR but multiplies the cost of each operation
/// call, so the budget analysis needs both.
pub(super) fn find_connector_of_operation<'a>(
    file: &'a corvid_ast::File,
    name: &str,
) -> Option<&'a corvid_ast::ConnectorDecl> {
    file.decls.iter().find_map(|d| match d {
        corvid_ast::Decl::Connector(c) if c.operations.iter().any(|op| op.name.name == name) => {
            Some(c)
        }
        _ => None,
    })
}

pub(super) fn find_prompt<'a>(
    file: &'a corvid_ast::File,
    name: &str,
) -> Option<&'a corvid_ast::PromptDecl> {
    file.decls.iter().find_map(|d| match d {
        corvid_ast::Decl::Prompt(p) if p.name.name == name => Some(p),
        _ => None,
    })
}

pub(super) fn find_agent<'a>(
    file: &'a corvid_ast::File,
    name: &str,
) -> Option<&'a corvid_ast::AgentDecl> {
    file.decls.iter().find_map(|d| match d {
        corvid_ast::Decl::Agent(a) if a.name.name == name => Some(a),
        _ => None,
    })
}
