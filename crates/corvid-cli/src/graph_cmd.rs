//! `corvid graph` (B47): print each entry point's tree, the one `@budget`
//! checks, as steps a reader follows, and the tools and prompts nothing
//! reaches. See `docs/proposals/agent-graph.md`.

use anyhow::{Context, Result};
use corvid_driver::agent_graph::{agent_graph, AgentGraphError};
use corvid_driver::{load_corvid_config_for, render_all_pretty};
use corvid_types::{CostNodeKind, CostTreeNode};
use std::path::Path;

pub(crate) fn run_graph(file: &Path, agent: Option<&str>) -> Result<u8> {
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
    let mut out = String::new();
    for (index, entry) in graph.entries.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        render_entry(&entry.tree, entry.bounded, &place, &mut out);
    }
    if !graph.never_reached.is_empty() {
        out.push_str("\nNever reached:\n");
        for (kind, name, span) in &graph.never_reached {
            out.push_str(&format!("  {kind} {name} ({})\n", place.at(*span)));
        }
    }
    print!("{out}");
    Ok(0)
}

/// Where a step is, as `file:line`.
struct Place<'a> {
    file: &'a Path,
    source: &'a str,
}

impl Place<'_> {
    fn at(&self, span: corvid_ast::Span) -> String {
        let line = self.source[..span.start.min(self.source.len())].matches('\n').count() + 1;
        let name = self.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{name}:{line}")
    }
}

/// The width of a step's indentation and label, so every step's cost and
/// location line up whatever its depth.
const STEP_WIDTH: usize = 48;

/// `text` padded with spaces to `width` characters (the tree's box
/// characters are one character each, though several bytes).
fn pad(text: &str, width: usize) -> String {
    let count = text.chars().count();
    format!("{text}{}", " ".repeat(width.saturating_sub(count)))
}

fn render_entry(tree: &CostTreeNode, bounded: bool, place: &Place<'_>, out: &mut String) {
    let worst = if bounded { costs(tree) } else { "no bound: a loop has none".to_string() };
    out.push_str(&format!("{} worst case {worst}\n", pad(&format!("agent {}", tree.name), STEP_WIDTH)));
    let steps = steps(&tree.children);
    for (index, step) in steps.iter().enumerate() {
        render_step(step, "", index + 1 == steps.len(), place, out);
    }
}

/// The steps a reader sees: grouping nodes (a statement, a condition)
/// are not steps, so their children take their place.
fn steps(children: &[CostTreeNode]) -> Vec<&CostTreeNode> {
    let mut out = Vec::new();
    for child in children {
        match child.kind {
            CostNodeKind::Sequence | CostNodeKind::Condition => out.extend(steps(&child.children)),
            _ => out.push(child),
        }
    }
    out
}

fn render_step(node: &CostTreeNode, prefix: &str, last: bool, place: &Place<'_>, out: &mut String) {
    let branch = if last { "└─ " } else { "├─ " };
    let label = match &node.kind {
        CostNodeKind::Tool if node.dangerous => format!("tool {} (dangerous)", node.name),
        CostNodeKind::Tool => format!("tool {}", node.name),
        CostNodeKind::Prompt => format!("prompt {}", node.name),
        CostNodeKind::Agent => format!("agent {}", node.name),
        CostNodeKind::Approval => format!("approve {}", node.name),
        CostNodeKind::Branch => "one of these paths".to_string(),
        CostNodeKind::Loop { iterations: Some(n) } => format!("loop, at most {n} times"),
        CostNodeKind::Loop { iterations: None } => "loop, unbounded".to_string(),
        CostNodeKind::Sequence | CostNodeKind::Condition => node.name.clone(),
    };
    let detail = match node.kind {
        CostNodeKind::Approval => "a person decides".to_string(),
        _ => costs(node),
    };
    let left = format!("{prefix}{branch}{label}");
    out.push_str(&format!("{} {detail:<24} {}\n", pad(&left, STEP_WIDTH), place.at(node.span)));
    let inner = format!("{prefix}{}", if last { "   " } else { "│  " });
    match node.kind {
        // Each path of a branch is one child, drawn as its own group.
        CostNodeKind::Branch => {
            for (index, path) in node.children.iter().enumerate() {
                let path_last = index + 1 == node.children.len();
                out.push_str(&format!("{inner}{}path {}\n", if path_last { "└─ " } else { "├─ " }, index + 1));
                let path_prefix = format!("{inner}{}", if path_last { "   " } else { "│  " });
                let path_steps = steps(std::slice::from_ref(path));
                if path_steps.is_empty() {
                    out.push_str(&format!("{path_prefix}└─ (nothing)\n"));
                }
                for (step_index, step) in path_steps.iter().enumerate() {
                    render_step(step, &path_prefix, step_index + 1 == path_steps.len(), place, out);
                }
            }
        }
        _ => {
            let children = steps(&node.children);
            for (index, child) in children.iter().enumerate() {
                render_step(child, &inner, index + 1 == children.len(), place, out);
            }
        }
    }
}

/// A node's worst-case costs, the dimensions that are not zero, or `free`.
fn costs(node: &CostTreeNode) -> String {
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
