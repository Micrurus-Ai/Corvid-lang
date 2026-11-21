//! Provenance analyzer for `Grounded<T>`.
//!
//! Verifies that every agent declared to return `Grounded<T>` has
//! a provenance path from a retrieval-tagged source (a tool whose
//! effect row contains `data: grounded`) through its body to the
//! return value. Without that path, the agent's promise that the
//! output is sourced from real data cannot be statically
//! established and the checker emits `UngroundedReturn`.
//!
//! Extracted from `effects.rs` as part of Phase 20i responsibility
//! decomposition.

use super::analyze::{find_agent, find_prompt, find_tool};
use super::EffectRegistry;
use corvid_ast::DimensionValue;
use corvid_resolve::{Binding, LocalId};
use std::collections::HashSet;

// ---- Provenance analyzer for Grounded<T> ----

/// Result of provenance analysis for one agent.
#[derive(Debug, Clone)]
pub struct ProvenanceResult {
    pub agent_name: String,
    pub return_is_grounded: bool,
    pub grounded_locals: Vec<String>,
    pub ungrounded_return_path: Option<String>,
}

/// Check whether an agent returning `Grounded<T>` has provenance from
/// a `data: grounded` source feeding into its return value. Returns
/// violations for agents that fail the check.
pub fn check_grounded_returns(
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    registry: &EffectRegistry,
) -> Vec<ProvenanceViolation> {
    let mut violations = Vec::new();

    for decl in &file.decls {
        let corvid_ast::Decl::Agent(agent) = decl else {
            continue;
        };

        // Check if the return type is Grounded<T>.
        if !is_grounded_type_ref(&agent.return_ty) {
            continue;
        }

        let mut proof = ReturnProof {
            file,
            resolved,
            registry,
            saw_return: false,
        };
        let mut parameters = HashSet::new();
        for param in &agent.params {
            if is_grounded_type_ref(&param.ty) {
                proof.assign(&mut parameters, param.name.span, true);
            }
        }
        let result = proof.block(&agent.body, parameters, &mut LoopPaths::default());
        if result.is_err() || !proof.saw_return {
            violations.push(ProvenanceViolation {
                agent_name: agent.name.name.clone(),
                span: result.err().unwrap_or_else(|| agent.return_ty.span()),
                message: format!(
                    "agent `{}` returns `{}` and requires a proven source on every return path",
                    agent.name.name,
                    format_type_ref(&agent.return_ty),
                ),
            });
        }
    }

    violations
}

/// A provenance violation: an agent returns Grounded<T> without proof.
#[derive(Debug, Clone)]
pub struct ProvenanceViolation {
    pub agent_name: String,
    pub span: corvid_ast::Span,
    pub message: String,
}

type Sources = HashSet<LocalId>;

// None denotes an unreachable path. A local has a source at a join only
// if all paths reaching the join have one.
fn join(paths: &mut Option<Sources>, incoming: Option<Sources>) {
    if let Some(incoming) = incoming {
        if let Some(existing) = paths {
            existing.retain(|id| incoming.contains(id));
        } else {
            *paths = Some(incoming);
        }
    }
}

#[derive(Default)]
struct LoopPaths {
    breaks: Option<Sources>,
    continues: Option<Sources>,
}

struct ReturnProof<'a> {
    file: &'a corvid_ast::File,
    resolved: &'a corvid_resolve::Resolved,
    registry: &'a EffectRegistry,
    saw_return: bool,
}

impl ReturnProof<'_> {
    fn sourced(&self, expr: &corvid_ast::Expr, sources: &Sources) -> bool {
        expr_is_grounded(expr, self.file, self.resolved, self.registry, sources)
    }

    fn assign(&self, sources: &mut Sources, span: corvid_ast::Span, sourced: bool) {
        if let Some(Binding::Local(id)) = self.resolved.bindings.get(&span) {
            if sourced {
                sources.insert(*id);
            } else {
                sources.remove(id);
            }
        }
    }

    fn block(
        &mut self,
        block: &corvid_ast::Block,
        mut sources: Sources,
        exits: &mut LoopPaths,
    ) -> Result<Option<Sources>, corvid_ast::Span> {
        use corvid_ast::Stmt;
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { name, value, .. } => {
                    let sourced = self.sourced(value, &sources);
                    self.assign(&mut sources, name.span, sourced);
                }
                Stmt::Assign {
                    target: corvid_ast::Expr::Ident { span, .. },
                    op,
                    value,
                    ..
                } => {
                    let previously_sourced = matches!(self.resolved.bindings.get(span),
                        Some(Binding::Local(id)) if sources.contains(id));
                    let sourced =
                        self.sourced(value, &sources) || (op.is_some() && previously_sourced);
                    self.assign(&mut sources, *span, sourced);
                }
                Stmt::Return { value, span } => {
                    self.saw_return = true;
                    if !value
                        .as_ref()
                        .is_some_and(|value| self.sourced(value, &sources))
                    {
                        return Err(value.as_ref().map_or(*span, |expr| expr.span()));
                    }
                    return Ok(None);
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    let mut next = self.block(then_block, sources.clone(), exits)?;
                    let otherwise = match else_block {
                        Some(block) => self.block(block, sources, exits)?,
                        None => Some(sources),
                    };
                    join(&mut next, otherwise);
                    let Some(next) = next else { return Ok(None) };
                    sources = next;
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } => {
                    // Include zero iterations. Recheck the body when a later
                    // iteration may start with fewer proven sources.
                    loop {
                        let mut loop_paths = LoopPaths::default();
                        let mut next = self.block(body, sources.clone(), &mut loop_paths)?;
                        join(&mut next, loop_paths.continues);
                        let mut header = sources.clone();
                        if let Some(back_edge) = next {
                            header.retain(|id| back_edge.contains(id));
                        }
                        if header == sources {
                            if let Some(early_exit) = loop_paths.breaks {
                                sources.retain(|id| early_exit.contains(id));
                            }
                            break;
                        }
                        sources = header;
                    }
                }
                Stmt::Break { .. } => {
                    join(&mut exits.breaks, Some(sources));
                    return Ok(None);
                }
                Stmt::Continue { .. } => {
                    join(&mut exits.continues, Some(sources));
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(Some(sources))
    }
}

/// Determine if an expression produces a grounded value.
fn expr_is_grounded(
    expr: &corvid_ast::Expr,
    file: &corvid_ast::File,
    resolved: &corvid_resolve::Resolved,
    registry: &EffectRegistry,
    grounded: &HashSet<LocalId>,
) -> bool {
    match expr {
        corvid_ast::Expr::Call { callee, args, .. } => {
            // Check if callee is a tool/prompt/agent with grounded effects.
            if let corvid_ast::Expr::Ident { span, .. } = &**callee {
                if let Some(corvid_resolve::Binding::Decl(def_id)) = resolved.bindings.get(span) {
                    let entry = resolved.symbols.get(*def_id);
                    match entry.kind {
                        corvid_resolve::DeclKind::Tool => {
                            if let Some(tool) = find_tool(file, &entry.name) {
                                // Check if the tool has a grounded effect.
                                if tool_is_grounded(tool, registry) {
                                    return true;
                                }
                            }
                        }
                        corvid_resolve::DeclKind::Prompt => {
                            // A prompt is grounded if its own effect row
                            // carries `data: grounded` (Design X, slice
                            // 2b — promotes the prompt's return type),
                            // OR if any of its args are grounded (the
                            // original args-flow rule: grounded input
                            // produces grounded output).
                            if let Some(prompt) = find_prompt(file, &entry.name) {
                                if effect_row_is_grounded(&prompt.effect_row, registry) {
                                    return true;
                                }
                            }
                            for arg in args {
                                if expr_is_grounded(arg, file, resolved, registry, grounded) {
                                    return true;
                                }
                            }
                        }
                        corvid_resolve::DeclKind::Agent => {
                            // Check if the called agent returns Grounded<T>.
                            if let Some(agent) = find_agent(file, &entry.name) {
                                if is_grounded_type_ref(&agent.return_ty) {
                                    // A callee may return its grounded parameter directly.
                                    // Its contract only applies to arguments with a source.
                                    return agent.params.iter().zip(args).all(|(param, arg)| {
                                        !is_grounded_type_ref(&param.ty)
                                            || expr_is_grounded(
                                                arg, file, resolved, registry, grounded,
                                            )
                                    });
                                }
                                // Also check if the agent has grounded effects.
                                for eff in &agent.effect_row.effects {
                                    if let Some(profile) = registry.get(&eff.name.name) {
                                        if profile.dimensions.get("data")
                                            == Some(&DimensionValue::Name("grounded".into()))
                                        {
                                            return true;
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            false
        }
        corvid_ast::Expr::Ident { span, .. } => {
            matches!(resolved.bindings.get(span), Some(Binding::Local(id)) if grounded.contains(id))
        }
        corvid_ast::Expr::FieldAccess { target, .. } => {
            // Field access on a grounded struct is grounded.
            expr_is_grounded(target, file, resolved, registry, grounded)
        }
        corvid_ast::Expr::BinOp {
            op, left, right, ..
        } => {
            // Provenance Propagation contagion law (D1): an operator
            // expression is grounded if either operand is grounded —
            // the reachability-analysis half of the law that
            // `check_binop` enforces at the type level. `&&` / `||`
            // are scoped out of the contagion law (they short-circuit),
            // so they do not propagate grounding, matching the
            // type-level rule.
            if matches!(op, corvid_ast::BinaryOp::And | corvid_ast::BinaryOp::Or) {
                false
            } else {
                expr_is_grounded(left, file, resolved, registry, grounded)
                    || expr_is_grounded(right, file, resolved, registry, grounded)
            }
        }
        corvid_ast::Expr::UnOp { operand, .. } => {
            // Contagion law (D1): a unary operator expression is
            // grounded if its operand is grounded.
            expr_is_grounded(operand, file, resolved, registry, grounded)
        }
        _ => false,
    }
}

/// Does this effect row carry `data: grounded` — i.e. does a value
/// produced under it carry provenance?
///
/// The single source of truth for "is this grounded," shared by the
/// provenance-reachability analysis (this module) and the
/// typechecker's Design X return-type wrapping (`check_*_call` in
/// `checker/call.rs`). The built-in `retrieval` effect is grounded by
/// definition; user effects are grounded iff their `data` dimension
/// resolves to `grounded` in the registry.
pub fn effect_row_is_grounded(
    effect_row: &corvid_ast::EffectRow,
    registry: &EffectRegistry,
) -> bool {
    effect_row.effects.iter().any(|eff| {
        eff.name.name == "retrieval"
            || registry
                .get(&eff.name.name)
                .map(|profile| {
                    profile.dimensions.get("data") == Some(&DimensionValue::Name("grounded".into()))
                })
                .unwrap_or(false)
    })
}

fn tool_is_grounded(tool: &corvid_ast::ToolDecl, registry: &EffectRegistry) -> bool {
    effect_row_is_grounded(&tool.effect_row, registry)
}

fn is_grounded_type_ref(ty: &corvid_ast::TypeRef) -> bool {
    matches!(ty, corvid_ast::TypeRef::Generic { name, args, .. }
        if name.name == "Grounded" && args.len() == 1)
}

fn format_type_ref(ty: &corvid_ast::TypeRef) -> String {
    match ty {
        corvid_ast::TypeRef::Named { name, .. } => name.name.clone(),
        corvid_ast::TypeRef::Qualified { alias, name, .. } => {
            format!("{}.{}", alias.name, name.name)
        }
        corvid_ast::TypeRef::Generic { name, args, .. } => {
            let inner: Vec<String> = args.iter().map(format_type_ref).collect();
            format!("{}<{}>", name.name, inner.join(", "))
        }
        corvid_ast::TypeRef::Weak { inner, .. } => format!("Weak<{}>", format_type_ref(inner)),
        corvid_ast::TypeRef::Function { params, ret, .. } => {
            let ps: Vec<String> = params.iter().map(format_type_ref).collect();
            format!("({}) -> {}", ps.join(", "), format_type_ref(ret))
        }
    }
}

/// Slice 50i — does this effect row mark its results as UNTRUSTED
/// content (`data: untrusted`)? The taint mirror of
/// [`effect_row_is_grounded`].
pub fn effect_row_is_untrusted(
    effect_row: &corvid_ast::EffectRow,
    registry: &EffectRegistry,
) -> bool {
    effect_row.effects.iter().any(|eff| {
        registry
            .get(&eff.name.name)
            .map(|profile| {
                profile.dimensions.get("data") == Some(&DimensionValue::Name("untrusted".into()))
            })
            .unwrap_or(false)
    })
}
