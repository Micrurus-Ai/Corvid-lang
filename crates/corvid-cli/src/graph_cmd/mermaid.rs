//! The Mermaid form of `corvid graph`: a flowchart for a pull request or
//! the docs. Each step is a node in the order it runs: a branch fans out
//! to its paths and they meet again at the next step, a loop's body runs
//! from the loop node and returns to it on a dotted edge, and each shape
//! says the kind: a tool is a box, a prompt rounded, an agent a
//! subroutine, an approval a parallelogram, a branch a diamond, a loop a
//! hexagon, a table operation a cylinder. Dangerous tools are outlined in
//! red.

use super::{detail, entry_title, label, route_title, surface_line, Place};
use corvid_driver::agent_graph::{has_unbounded_loop, steps, AgentGraph};
use corvid_types::{CostNodeKind, CostTreeNode};

pub(super) fn render(graph: &AgentGraph, place: &Place<'_>) -> String {
    let mut chart = Chart { lines: vec!["flowchart TD".to_string()], next: 0, dangerous: Vec::new() };
    for entry in &graph.entries {
        let mut title = entry_title(graph, &entry.tree);
        if let Some(surface) = graph.actions.get(&entry.tree.name) {
            title = format!("{title}<br/>{}", surface_line(&entry.tree.name, surface, place));
        }
        chart.root(&title, &entry.tree, entry.bounded, place);
    }
    for route in &graph.routes {
        chart.root(&route_title(route), &route.estimate.tree, route.estimate.bounded, place);
    }
    if !graph.never_reached.is_empty() {
        chart.lines.push("  subgraph never_reached [\"Never reached\"]".to_string());
        for (kind, name, span) in &graph.never_reached {
            let id = chart.fresh();
            chart.lines.push(format!("    {id}[\"{}\"]", text(&format!("{} {name}<br/>{}", kind.word(), place.at(*span)))));
        }
        chart.lines.push("  end".to_string());
    }
    if !chart.dangerous.is_empty() {
        chart.lines.push("  classDef dangerous stroke:#c62828,stroke-width:3px".to_string());
        chart.lines.push(format!("  class {} dangerous", chart.dangerous.join(",")));
    }
    let mut out = chart.lines.join("\n");
    out.push('\n');
    out
}

enum Shape {
    Box,
    Rounded,
    Subroutine,
    Parallelogram,
    Diamond,
    Hexagon,
    Stadium,
    Cylinder,
}

struct Chart {
    lines: Vec<String>,
    next: usize,
    dangerous: Vec<String>,
}

impl Chart {
    fn fresh(&mut self) -> String {
        self.next += 1;
        format!("n{}", self.next)
    }

    fn node(&mut self, title: &str, detail: &str, at: String, shape: Shape) -> String {
        let id = self.fresh();
        let body = text(&format!("{title}<br/>{detail}<br/>{at}"));
        let (open, close) = match shape {
            Shape::Box => ("[\"", "\"]"),
            Shape::Rounded => ("(\"", "\")"),
            Shape::Subroutine => ("[[\"", "\"]]"),
            Shape::Parallelogram => ("[/\"", "\"/]"),
            Shape::Diamond => ("{\"", "\"}"),
            Shape::Hexagon => ("{{\"", "\"}}"),
            Shape::Stadium => ("([\"", "\"])"),
            Shape::Cylinder => ("[(\"", "\")]"),
        };
        self.lines.push(format!("  {id}{open}{body}{close}"));
        id
    }

    fn root(&mut self, title: &str, tree: &CostTreeNode, bounded: bool, place: &Place<'_>) {
        let worst = if bounded && !has_unbounded_loop(tree) {
            format!("worst case {}", detail(tree))
        } else {
            "no bound: a loop has none".to_string()
        };
        let start = self.node(title, &worst, place.at(tree.span), Shape::Stadium);
        self.sequence(&steps(&tree.children), vec![start], place);
    }

    fn edges(&mut self, from: &[String], to: &str) {
        for source in from {
            self.lines.push(format!("  {source} --> {to}"));
        }
    }

    /// Draw `steps` in order after `from`; returns the nodes the next step
    /// follows.
    fn sequence(&mut self, steps: &[&CostTreeNode], mut from: Vec<String>, place: &Place<'_>) -> Vec<String> {
        for node in steps {
            from = self.step(node, from, place);
        }
        from
    }

    fn step(&mut self, node: &CostTreeNode, from: Vec<String>, place: &Place<'_>) -> Vec<String> {
        let shape = match node.kind {
            CostNodeKind::Tool => Shape::Box,
            CostNodeKind::Prompt => Shape::Rounded,
            CostNodeKind::Agent => Shape::Subroutine,
            CostNodeKind::Approval => Shape::Parallelogram,
            CostNodeKind::Branch => Shape::Diamond,
            CostNodeKind::Table { .. } => Shape::Cylinder,
            CostNodeKind::Loop { .. } => Shape::Hexagon,
            CostNodeKind::Sequence | CostNodeKind::Condition => Shape::Box,
        };
        let id = self.node(&label(node), &detail(node), place.at(node.span), shape);
        self.edges(&from, &id);
        if node.dangerous {
            self.dangerous.push(id.clone());
        }
        match node.kind {
            CostNodeKind::Branch => {
                let mut exits = Vec::new();
                for path in &node.children {
                    exits.extend(self.sequence(&steps(std::slice::from_ref(path)), vec![id.clone()], place));
                }
                exits
            }
            CostNodeKind::Loop { .. } => {
                let body_exits = self.sequence(&steps(&node.children), vec![id.clone()], place);
                for exit in body_exits.iter().filter(|exit| **exit != id) {
                    self.lines.push(format!("  {exit} -.-> {id}"));
                }
                vec![id]
            }
            CostNodeKind::Agent => self.sequence(&steps(&node.children), vec![id], place),
            _ => vec![id],
        }
    }
}

/// Text safe inside a quoted Mermaid label.
fn text(raw: &str) -> String {
    raw.replace('"', "#quot;")
}
