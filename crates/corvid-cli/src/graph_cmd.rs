//! `corvid graph` (B47): print each entry point's tree, the one `@budget`
//! checks, as steps a reader follows, and the tools and prompts nothing
//! reaches. See `docs/proposals/agent-graph.md`. This facade checks the
//! program, holds what the text and Mermaid forms both say about a step,
//! and hands the graph to one form's renderer. The JSON form is the
//! driver's, so every reader of it gets the same shape.

mod mermaid;
mod text;

use crate::cli::GraphFormat;
use anyhow::{Context, Result};
use corvid_driver::agent_graph::{
    agent_graph, has_unbounded_loop, ActionSurface, AgentGraph, AgentGraphError, GraphDocument, RouteGraph, StepKind,
};
use corvid_driver::{load_corvid_config_for, render_all_pretty};
use corvid_types::{CostNodeKind, CostTreeNode};
use std::path::Path;

pub(crate) fn run_graph(file: &Path, agent: Option<&str>, format: GraphFormat) -> Result<u8> {
    let source = std::fs::read_to_string(file).with_context(|| format!("cannot read `{}`", file.display()))?;
    let config = load_corvid_config_for(file);
    let graph = match agent_graph(&source, file, config.as_ref(), agent) {
        Ok(graph) => graph,
        Err(AgentGraphError::Diagnostics(diagnostics)) => {
            eprint!("{}", render_all_pretty(&diagnostics, file, &source));
            return Ok(1);
        }
        Err(AgentGraphError::NoSuchAgent(name)) => {
            eprintln!("error: `{name}` is not an agent of `{}`", file.display());
            return Ok(1);
        }
    };
    let place = Place { file, source: &source };
    match format {
        GraphFormat::Text => print!("{}", text::render(&graph, &place)),
        GraphFormat::Mermaid => print!("{}", mermaid::render(&graph, &place)),
        GraphFormat::Json => {
            let document = GraphDocument::new(&graph, &place.file_name(), &source);
            println!("{}", serde_json::to_string_pretty(&document)?);
        }
    }
    Ok(0)
}

/// Where a step is, as `file:line`.
struct Place<'a> {
    file: &'a Path,
    source: &'a str,
}

impl Place<'_> {
    fn at(&self, span: corvid_ast::Span) -> String {
        format!("{}:{}", self.file_name(), self.line(span))
    }

    fn line(&self, span: corvid_ast::Span) -> usize {
        self.source[..span.start.min(self.source.len())].matches('\n').count() + 1
    }

    fn file_name(&self) -> String {
        self.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// An entry point's heading: `agent refund_bot` or `action move_card`.
fn entry_title(graph: &AgentGraph, tree: &CostTreeNode) -> String {
    let kind = graph.declared.get(&tree.name).map_or(StepKind::Agent, |d| d.kind);
    format!("{} {}", kind.word(), tree.name)
}

/// A route's heading: `route GET /orders/{id}`, or `live /boards -> Card`
/// for a live declaration, with who may call it.
fn route_title(route: &RouteGraph) -> String {
    let head = match &route.live {
        Some(row) => format!("live {} -> {row}", route.path),
        None => format!("route {} {}", route.method, route.path),
    };
    format!("{head}, {}", policy_text(route.policy.as_ref()))
}

/// How outside callers reach an action: its route, policy, assistant
/// offer and the forms and buttons naming it.
fn surface_line(name: &str, surface: &ActionSurface, place: &Place<'_>) -> String {
    let mut parts = vec![
        format!("POST {}{name}", corvid_types::action_input::ACTIONS_PATH),
        policy_text(Some(&surface.policy)),
        match &surface.assistants {
            Some(offer) => format!("offered to assistants ({})", offer.mode.as_str()),
            None => "not offered to assistants".to_string(),
        },
    ];
    for (builder, container, span) in &surface.used_by {
        parts.push(format!("{builder} in {container} ({})", place.at(*span)));
    }
    parts.join(", ")
}

fn policy_text(policy: Option<&corvid_ast::RoutePolicy>) -> String {
    let Some(policy) = policy.filter(|p| p.requires_auth()) else {
        return "public".to_string();
    };
    let mut needs = Vec::new();
    needs.extend(policy.roles.iter().map(|role| format!("role {role}")));
    needs.extend(policy.permissions.iter().map(|permission| format!("permission {permission}")));
    if needs.is_empty() {
        "signed in".to_string()
    } else {
        format!("needs {}", needs.join(" and "))
    }
}

/// What a step is, as a reader says it.
fn label(node: &CostTreeNode) -> String {
    match &node.kind {
        CostNodeKind::Tool if node.dangerous => format!("tool {} (dangerous)", node.name),
        CostNodeKind::Tool => format!("tool {}", node.name),
        CostNodeKind::Prompt => format!("prompt {}", node.name),
        CostNodeKind::Agent => format!("agent {}", node.name),
        CostNodeKind::Approval => format!("approve {}", node.name),
        CostNodeKind::Branch => "one of these paths".to_string(),
        CostNodeKind::Table { operation } => format!("table {}.{operation}", node.name),
        CostNodeKind::Loop { iterations: Some(n) } => format!("loop, at most {n} times"),
        CostNodeKind::Loop { iterations: None } if node.name == "lambda" => {
            "lambda, called any number of times".to_string()
        }
        CostNodeKind::Loop { iterations: None } => "loop, unbounded".to_string(),
        CostNodeKind::Sequence | CostNodeKind::Condition => node.name.clone(),
    }
}

/// What a step can cost: its worst case, `free`, `a person decides` for an
/// approval, or `no bound` when a loop inside it has none (its numbers
/// would then be one pass, not a worst case).
fn detail(node: &CostTreeNode) -> String {
    if node.kind == CostNodeKind::Approval {
        return "a person decides".to_string();
    }
    if has_unbounded_loop(node) {
        return "no bound".to_string();
    }
    let mut parts = Vec::new();
    for dim in ["cost", "tokens", "latency_ms"] {
        let value = node.costs.get(dim).copied().unwrap_or(0.0);
        if value > f64::EPSILON {
            parts.push(corvid_types::format_numeric_dimension(dim, value));
        }
    }
    if parts.is_empty() {
        "free".to_string()
    } else {
        parts.join(", ")
    }
}
