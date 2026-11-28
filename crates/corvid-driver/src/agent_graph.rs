//! `corvid graph` (B47): each entry point's steps, branches, loops, tool
//! and prompt calls, approvals and worst-case cost, drawn from the cost
//! analyzer's tree, the one `@budget` checks. Because it is that tree, the
//! graph cannot drift from the code, and its numbers are the budget
//! check's. See `docs/proposals/agent-graph.md`.

use crate::pipeline::typecheck_driver_file;
use crate::Diagnostic;
use corvid_ast::{Decl, Expr, Span};
use corvid_types::{CostEstimate, CostNodeKind, CostTreeNode, CorvidConfig};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

/// Why a graph could not be drawn.
#[derive(Debug)]
pub enum AgentGraphError {
    /// The program does not check.
    Diagnostics(Vec<Diagnostic>),
    /// `--agent` named no agent of the program.
    NoSuchAgent(String),
}

/// The program's graph: one tree per entry point drawn, and the tools and
/// prompts no entry point reaches.
#[derive(Debug, Clone)]
pub struct AgentGraph {
    pub entries: Vec<CostEstimate>,
    /// `(kind, name, declared at)`, in source order, `kind` being `tool`
    /// or `prompt`.
    pub never_reached: Vec<(&'static str, String, Span)>,
}

/// Build the graph of `agent`, or of every entry point when `agent` is
/// `None`: each public agent and action, and each agent no other agent
/// calls (route handlers and scheduled agents among them). The
/// never-reached list is always taken over every entry point, so naming
/// one agent does not make the rest of the program look unused.
pub fn agent_graph(
    source: &str,
    source_path: &Path,
    config: Option<&CorvidConfig>,
    agent: Option<&str>,
) -> Result<AgentGraph, AgentGraphError> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let tokens = corvid_syntax::lex(source)
        .map_err(|errs| AgentGraphError::Diagnostics(errs.into_iter().map(Diagnostic::from).collect()))?;
    let (file, parse_errs) = corvid_syntax::parse_file(&tokens);
    diagnostics.extend(parse_errs.into_iter().map(Diagnostic::from));
    let resolved = corvid_resolve::resolve(&file);
    diagnostics.extend(resolved.errors.iter().cloned().map(Diagnostic::from));
    let typechecked = typecheck_driver_file(&file, &resolved, source_path, config);
    diagnostics.extend(typechecked.diagnostics);
    diagnostics.extend(typechecked.result.checked.errors.iter().cloned().map(Diagnostic::from));
    if !diagnostics.is_empty() {
        return Err(AgentGraphError::Diagnostics(diagnostics));
    }
    let registry = corvid_types::effect_registry_for(&file, typechecked.result.modules.as_ref(), config);
    let types = &typechecked.result.checked.types;
    let estimate = |name: &str| corvid_types::compute_worst_case_cost_with_types(&file, &resolved, &registry, name, types);

    let agents: Vec<&corvid_ast::AgentDecl> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Agent(agent) => Some(agent),
            _ => None,
        })
        .collect();
    let called: HashSet<String> = agents.iter().flat_map(|a| called_names(&a.body)).collect();
    let entry_names: Vec<String> = agents
        .iter()
        .filter(|a| {
            a.action.is_some()
                || !matches!(a.visibility, corvid_ast::Visibility::Private)
                || !called.contains(&a.name.name)
        })
        .map(|a| a.name.name.clone())
        .collect();

    let all_entries: Vec<CostEstimate> = entry_names.iter().filter_map(|name| estimate(name)).collect();
    let mut reached = HashSet::new();
    for entry in &all_entries {
        collect_calls(&entry.tree, &mut reached);
    }
    let mut never_reached = Vec::new();
    for decl in &file.decls {
        match decl {
            Decl::Tool(tool) if !reached.contains(&tool.name.name) => {
                never_reached.push(("tool", tool.name.name.clone(), tool.span))
            }
            Decl::Prompt(prompt) if !reached.contains(&prompt.name.name) => {
                never_reached.push(("prompt", prompt.name.name.clone(), prompt.span))
            }
            _ => {}
        }
    }

    let entries = match agent {
        Some(name) => {
            vec![estimate(name).ok_or_else(|| AgentGraphError::NoSuchAgent(name.to_string()))?]
        }
        None => all_entries,
    };
    Ok(AgentGraph { entries, never_reached })
}

/// The tool, prompt and agent names a tree calls.
fn collect_calls(node: &CostTreeNode, out: &mut HashSet<String>) {
    if matches!(node.kind, CostNodeKind::Tool | CostNodeKind::Prompt | CostNodeKind::Agent) {
        out.insert(node.name.clone());
    }
    for child in &node.children {
        collect_calls(child, out);
    }
}

/// The names `body` calls directly, by the callee written at each call.
fn called_names(body: &corvid_ast::Block) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    corvid_ast::visit::walk_block(body, &mut |_| {}, &mut |expr| {
        if let Expr::Call { callee, .. } = expr {
            if let Expr::Ident { name, .. } = callee.as_ref() {
                out.insert(name.name.clone());
            }
        }
    });
    out
}
