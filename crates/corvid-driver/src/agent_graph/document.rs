//! The agent graph as a document (B47 G2): the one shape `corvid graph
//! --format json`, the dev console and an assistant asking the compiler
//! (B22) all serialize, so they cannot drift. Documented in
//! `docs/reference/agent-graph.md`, versioned by [`AGENT_GRAPH_SCHEMA`].

use super::{has_unbounded_loop, steps, AgentGraph, Declared};
use corvid_types::{CostNodeKind, CostTreeNode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The schema name every graph document carries. A change a reader must
/// handle differently takes a new version.
pub const AGENT_GRAPH_SCHEMA: &str = "corvid.agent_graph.v1";

/// The whole graph: one entry per entry point drawn, and what no entry
/// point reaches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphDocument {
    pub schema: String,
    /// The source file, as the command was given it.
    pub file: String,
    pub entries: Vec<GraphEntry>,
    pub never_reached: Vec<NeverReached>,
}

/// One entry point: an action, or an agent nothing else calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEntry {
    /// The agent's name; every step's id starts with it.
    pub id: String,
    /// `agent` or `action`.
    pub kind: StepKind,
    pub name: String,
    pub location: Location,
    /// Every effect a step under it declares, sorted.
    pub effects: Vec<String>,
    /// Worst-case cost by dimension (`cost` in US dollars), the numbers
    /// `@budget` checks. `None` (JSON `null`) when a loop has no bound.
    pub worst_case: Option<BTreeMap<String, f64>>,
    pub steps: Vec<GraphStep>,
}

/// One step a path takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphStep {
    /// The entry's name, then each enclosing step and this one by what it
    /// calls (`approve.Label`, `branch`, `loop`, `lambda` for the rest),
    /// with `pathN` for a branch's paths and `#N` for a repeat in the same
    /// block: `refund_bot/branch/path2/issue_refund`. Adding or removing
    /// an unrelated step changes no other step's id.
    pub id: String,
    pub kind: StepKind,
    /// The callee, for a tool, connector operation, Python function,
    /// prompt, agent or action; the label, for an approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub location: Location,
    /// The effects the callee declares, or every effect under a grouping
    /// step, sorted.
    pub effects: Vec<String>,
    /// For a tool or connector operation: whether it is `dangerous`. For an
    /// approval: whether the tool it guards is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dangerous: Option<bool>,
    /// For an approval: the dangerous tool its label names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guards: Option<String>,
    /// For a loop: the most times it runs. Absent when nothing bounds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u64>,
    /// Worst-case cost by dimension (`cost` in US dollars); `None` when a
    /// loop inside it has no bound.
    pub worst_case: Option<BTreeMap<String, f64>>,
    /// The steps inside an agent, action or loop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<GraphStep>,
    /// A branch's paths, each its own list of steps (an empty one does
    /// nothing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<Vec<GraphStep>>,
}

/// A declaration nothing reaches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeverReached {
    pub kind: StepKind,
    pub name: String,
    pub location: Location,
}

/// Where something is: 1-based line and column (in characters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Agent,
    Action,
    Tool,
    ConnectorOperation,
    PythonFunction,
    Prompt,
    Approval,
    Branch,
    Loop,
    /// A function value whose body runs any number of times.
    Lambda,
}

impl StepKind {
    /// The kind as a reader says it: `tool`, `connector operation`.
    pub fn word(self) -> &'static str {
        match self {
            StepKind::Agent => "agent",
            StepKind::Action => "action",
            StepKind::Tool => "tool",
            StepKind::ConnectorOperation => "connector operation",
            StepKind::PythonFunction => "Python function",
            StepKind::Prompt => "prompt",
            StepKind::Approval => "approval",
            StepKind::Branch => "branch",
            StepKind::Loop => "loop",
            StepKind::Lambda => "lambda",
        }
    }
}

impl GraphDocument {
    /// `graph` as a document. `file` names the source in it; `source` is
    /// the text, which turns spans into lines and columns.
    pub fn new(graph: &AgentGraph, file: &str, source: &str) -> Self {
        let build = Builder { declared: &graph.declared, source };
        let entries = graph
            .entries
            .iter()
            .map(|entry| {
                let tree = &entry.tree;
                let steps = build.step_list(&tree.children, &tree.name);
                GraphEntry {
                    id: tree.name.clone(),
                    kind: build.declared.get(&tree.name).map_or(StepKind::Agent, |d| d.kind),
                    name: tree.name.clone(),
                    location: build.location(tree.span),
                    effects: union(&steps),
                    worst_case: (entry.bounded && !has_unbounded_loop(tree)).then(|| worst_case(tree)),
                    steps,
                }
            })
            .collect();
        let never_reached = graph
            .never_reached
            .iter()
            .map(|(kind, name, span)| NeverReached { kind: *kind, name: name.clone(), location: build.location(*span) })
            .collect();
        GraphDocument { schema: AGENT_GRAPH_SCHEMA.to_string(), file: file.to_string(), entries, never_reached }
    }
}

struct Builder<'a> {
    declared: &'a BTreeMap<String, Declared>,
    source: &'a str,
}

impl Builder<'_> {
    fn location(&self, span: corvid_ast::Span) -> Location {
        let before = &self.source[..span.start.min(self.source.len())];
        let line_start = before.rfind('\n').map_or(0, |at| at + 1);
        Location { line: before.matches('\n').count() + 1, column: before[line_start..].chars().count() + 1 }
    }

    fn step_list(&self, children: &[CostTreeNode], parent: &str) -> Vec<GraphStep> {
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        steps(children)
            .into_iter()
            .map(|node| {
                let segment = id_segment(node);
                let count = seen.entry(segment.clone()).or_insert(0);
                *count += 1;
                let id = match *count {
                    1 => format!("{parent}/{segment}"),
                    n => format!("{parent}/{segment}#{n}"),
                };
                self.step(node, id)
            })
            .collect()
    }

    fn step(&self, node: &CostTreeNode, id: String) -> GraphStep {
        let declared = self.declared.get(&node.name);
        let kind = match node.kind {
            CostNodeKind::Tool => declared.map_or(StepKind::Tool, |d| d.kind),
            CostNodeKind::Prompt => StepKind::Prompt,
            CostNodeKind::Agent => declared.map_or(StepKind::Agent, |d| d.kind),
            CostNodeKind::Approval => StepKind::Approval,
            CostNodeKind::Branch => StepKind::Branch,
            CostNodeKind::Loop { .. } if node.name == "lambda" => StepKind::Lambda,
            // `steps` never yields a grouping node; a connector's retries
            // and polls are a bounded loop around its call.
            CostNodeKind::Loop { .. } | CostNodeKind::Sequence | CostNodeKind::Condition => StepKind::Loop,
        };
        let calls = matches!(node.kind, CostNodeKind::Tool | CostNodeKind::Prompt | CostNodeKind::Agent);
        let (steps, paths) = match node.kind {
            CostNodeKind::Branch => {
                let paths = node
                    .children
                    .iter()
                    .enumerate()
                    .map(|(index, path)| self.step_list(std::slice::from_ref(path), &format!("{id}/path{}", index + 1)))
                    .collect();
                (Vec::new(), paths)
            }
            CostNodeKind::Agent | CostNodeKind::Loop { .. } => (self.step_list(&node.children, &id), Vec::new()),
            _ => (Vec::new(), Vec::new()),
        };
        let effects = match (node.kind.clone(), declared) {
            (CostNodeKind::Tool | CostNodeKind::Prompt, Some(d)) => {
                let mut effects = d.effects.clone();
                effects.sort();
                effects.dedup();
                effects
            }
            _ => union(steps.iter().chain(paths.iter().flatten())),
        };
        let guards = (node.kind == CostNodeKind::Approval)
            .then(|| {
                self.declared
                    .iter()
                    .find(|(name, d)| d.dangerous && corvid_types::approve_label_for(name) == node.name)
                    .map(|(name, _)| name.clone())
            })
            .flatten();
        let dangerous = match node.kind {
            CostNodeKind::Tool => Some(node.dangerous),
            CostNodeKind::Approval => Some(guards.is_some()),
            _ => None,
        };
        GraphStep {
            id,
            kind,
            name: (calls || node.kind == CostNodeKind::Approval).then(|| node.name.clone()),
            location: self.location(node.span),
            effects,
            dangerous,
            guards,
            iterations: match node.kind {
                CostNodeKind::Loop { iterations } => iterations,
                _ => None,
            },
            worst_case: (!has_unbounded_loop(node)).then(|| worst_case(node)),
            steps,
            paths,
        }
    }
}

/// A step's part of its id: what it calls, so adding or removing an
/// unrelated step elsewhere changes no other step's id. `#2`, `#3` mark a
/// repeat of the same segment in the same block.
fn id_segment(node: &CostTreeNode) -> String {
    match node.kind {
        CostNodeKind::Tool | CostNodeKind::Prompt | CostNodeKind::Agent => node.name.clone(),
        CostNodeKind::Approval => format!("approve.{}", node.name),
        CostNodeKind::Branch => "branch".to_string(),
        CostNodeKind::Loop { .. } if node.name == "lambda" => "lambda".to_string(),
        CostNodeKind::Loop { .. } | CostNodeKind::Sequence | CostNodeKind::Condition => "loop".to_string(),
    }
}

/// Every effect the steps declare, sorted, once each.
fn union<'a>(steps: impl IntoIterator<Item = &'a GraphStep>) -> Vec<String> {
    let all: BTreeSet<&String> = steps.into_iter().flat_map(|step| step.effects.iter()).collect();
    all.into_iter().cloned().collect()
}

fn worst_case(node: &CostTreeNode) -> BTreeMap<String, f64> {
    node.costs.iter().map(|(dim, value)| (dim.clone(), *value)).collect()
}
