//! The text form of `corvid graph`: a tree a person reads in a terminal.

use super::{detail, label, Place};
use corvid_driver::agent_graph::{has_unbounded_loop, steps, AgentGraph};
use corvid_types::{CostNodeKind, CostTreeNode};

pub(super) fn render(graph: &AgentGraph, place: &Place<'_>) -> String {
    let mut out = String::new();
    for (index, entry) in graph.entries.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        render_entry(&entry.tree, entry.bounded, place, &mut out);
    }
    if !graph.never_reached.is_empty() {
        out.push_str("\nNever reached:\n");
        for (kind, name, span) in &graph.never_reached {
            out.push_str(&format!("  {} {name} ({})\n", kind.word(), place.at(*span)));
        }
    }
    out
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
    let worst = if bounded && !has_unbounded_loop(tree) {
        detail(tree)
    } else {
        "no bound: a loop has none".to_string()
    };
    out.push_str(&format!("{} worst case {worst}\n", pad(&format!("agent {}", tree.name), STEP_WIDTH)));
    let steps = steps(&tree.children);
    for (index, step) in steps.iter().enumerate() {
        render_step(step, "", index + 1 == steps.len(), place, out);
    }
}

fn render_step(node: &CostTreeNode, prefix: &str, last: bool, place: &Place<'_>, out: &mut String) {
    let branch = if last { "└─ " } else { "├─ " };
    let label = label(node);
    let detail = detail(node);
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
