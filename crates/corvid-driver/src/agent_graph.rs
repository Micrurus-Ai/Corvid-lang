//! `corvid graph` (B47): each entry point's steps, branches, loops, tool
//! and prompt calls, approvals and worst-case cost, drawn from the cost
//! analyzer's tree, the one `@budget` checks. Because it is that tree, the
//! graph cannot drift from the code, and its numbers are the budget
//! check's. See `docs/proposals/agent-graph.md`.

mod document;

pub use document::{
    ActionCall, ActionUse, AssistantOffer, GraphDocument, GraphEntry, GraphRoute, GraphSchedule, GraphStep, Location,
    NeverReached, Requires, StepKind, AGENT_GRAPH_SCHEMA,
};

use crate::pipeline::typecheck_driver_file;
use crate::Diagnostic;
use corvid_ast::{Decl, Expr, Span};
use corvid_types::{CostEstimate, CostNodeKind, CostTreeNode, CorvidConfig};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

/// Why a graph could not be drawn.
#[derive(Debug)]
pub enum AgentGraphError {
    /// The program does not check.
    Diagnostics(Vec<Diagnostic>),
    /// `--agent` named no agent of the program.
    NoSuchAgent(String),
}

/// The program's graph: one tree per entry point and route drawn, and the
/// tools and prompts nothing reaches.
#[derive(Debug, Clone)]
pub struct AgentGraph {
    pub entries: Vec<CostEstimate>,
    /// Each route and `live` declaration, in source order, rooted at its
    /// handler. Empty when one agent is drawn.
    pub routes: Vec<RouteGraph>,
    /// Each `schedule`, in source order: `(id, cron, zone, the agent it
    /// runs, where)`. Empty when one agent is drawn.
    pub schedules: Vec<ScheduleRoot>,
    /// How each action is called from outside the program, by name.
    pub actions: BTreeMap<String, ActionSurface>,
    /// What each callable the trees can name is, by the name the tree
    /// records: its kind, declared effects, and whether it is dangerous.
    pub declared: BTreeMap<String, Declared>,
    /// `(kind, name, declared at)`, in source order: the tools, connector
    /// operations and prompts no entry point reaches.
    pub never_reached: Vec<(StepKind, String, Span)>,
}

/// A route's handler, costed as an agent's body is.
#[derive(Debug, Clone)]
pub struct RouteGraph {
    /// `server:METHOD path`, or `server:live path` for a `live`
    /// declaration, unique in a program.
    pub id: String,
    pub server: String,
    pub method: &'static str,
    pub path: String,
    /// For a `live` declaration, the row type it delivers, as written.
    pub live: Option<String>,
    /// `None` for a public route.
    pub policy: Option<corvid_ast::RoutePolicy>,
    pub estimate: CostEstimate,
}

/// A `schedule`: work the program starts with no caller.
#[derive(Debug, Clone)]
pub struct ScheduleRoot {
    /// `schedule:<agent>`, with `#2` for a second schedule of one agent.
    pub id: String,
    pub cron: String,
    pub zone: String,
    pub runs: String,
    pub span: Span,
}

/// How outside callers reach an action.
#[derive(Debug, Clone)]
pub struct ActionSurface {
    pub policy: corvid_ast::RoutePolicy,
    /// `None` when no assistant is offered it.
    pub assistants: Option<corvid_ast::AssistantExposure>,
    /// Each `form_for` or `button_for` naming it: `(builder, the agent or
    /// route containing it, where)`, in source order.
    pub used_by: Vec<(&'static str, String, Span)>,
}

/// A declaration a step calls.
#[derive(Debug, Clone)]
pub struct Declared {
    pub kind: StepKind,
    /// The effects it declares with `uses`, plus `dangerous` for a
    /// dangerous tool, in source order.
    pub effects: Vec<String>,
    pub dangerous: bool,
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
    let route_decls: Vec<(&corvid_ast::ServerDecl, &corvid_ast::HttpRouteDecl)> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Server(server) => Some(server),
            _ => None,
        })
        .flat_map(|server| server.routes.iter().map(move |route| (server, route)))
        .collect();
    // A `live` declaration may share a GET route's path, so it is told
    // apart by `live` in place of the method.
    let route_id = |server: &corvid_ast::ServerDecl, route: &corvid_ast::HttpRouteDecl| {
        let method = if route.response.live_row().is_some() { "live" } else { route.method.as_str() };
        format!("{}:{method} {}", server.name.name, route.path)
    };
    // An agent a route or another agent calls is drawn under its caller.
    let called: HashSet<String> = agents
        .iter()
        .map(|a| &a.body)
        .chain(route_decls.iter().map(|(_, route)| &route.body))
        .flat_map(called_names)
        .collect();
    let entry_names: Vec<String> = agents
        .iter()
        .filter(|a| {
            a.action.is_some()
                || !matches!(a.visibility, corvid_ast::Visibility::Private)
                || !called.contains(&a.name.name)
        })
        .map(|a| a.name.name.clone())
        .collect();
    let all_routes: Vec<RouteGraph> = route_decls
        .iter()
        .map(|(server, route)| {
            let id = route_id(server, route);
            RouteGraph {
                server: server.name.name.clone(),
                method: route.method.as_str(),
                path: route.path.clone(),
                live: route.response.live_row().map(|ty| {
                    let span = ty.span();
                    source[span.start..span.end].to_string()
                }),
                policy: route.policy.clone(),
                estimate: corvid_types::compute_route_cost_with_types(&file, &resolved, &registry, route, &id, types),
                id,
            }
        })
        .collect();

    let mut actions: BTreeMap<String, ActionSurface> = agents
        .iter()
        .filter_map(|a| {
            let spec = a.action.as_ref()?;
            Some((
                a.name.name.clone(),
                ActionSurface { policy: spec.policy.clone(), assistants: spec.assistants.clone(), used_by: Vec::new() },
            ))
        })
        .collect();
    let bodies = agents
        .iter()
        .map(|a| (a.name.name.clone(), &a.body))
        .chain(route_decls.iter().map(|(server, route)| (route_id(server, route), &route.body)));
    for (container, body) in bodies {
        for (builder, action, span) in action_builders(body) {
            if let Some(surface) = actions.get_mut(&action) {
                surface.used_by.push((builder, container.clone(), span));
            }
        }
    }

    let declared = declarations(&file);
    let all_entries: Vec<CostEstimate> = entry_names.iter().filter_map(|name| estimate(name)).collect();
    let mut reached = HashSet::new();
    for tree in all_entries.iter().map(|e| &e.tree).chain(all_routes.iter().map(|r| &r.estimate.tree)) {
        collect_calls(tree, &mut reached);
    }
    let mut never_reached = Vec::new();
    for decl in &file.decls {
        match decl {
            Decl::Tool(tool) if !reached.contains(&tool.name.name) => {
                never_reached.push((StepKind::Tool, tool.name.name.clone(), tool.span))
            }
            Decl::Connector(connector) => {
                for op in connector.operations.iter().filter(|op| !reached.contains(&op.name.name)) {
                    never_reached.push((StepKind::ConnectorOperation, op.name.name.clone(), op.span));
                }
            }
            Decl::Prompt(prompt) if !reached.contains(&prompt.name.name) => {
                never_reached.push((StepKind::Prompt, prompt.name.name.clone(), prompt.span))
            }
            _ => {}
        }
    }

    let mut per_agent: BTreeMap<&str, usize> = BTreeMap::new();
    let all_schedules: Vec<ScheduleRoot> = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Schedule(schedule) => Some(schedule),
            _ => None,
        })
        .map(|schedule| {
            let count = per_agent.entry(schedule.target.name.as_str()).or_insert(0);
            *count += 1;
            let id = match *count {
                1 => format!("schedule:{}", schedule.target.name),
                n => format!("schedule:{}#{n}", schedule.target.name),
            };
            ScheduleRoot {
                id,
                cron: schedule.cron.clone(),
                zone: schedule.zone.clone(),
                runs: schedule.target.name.clone(),
                span: schedule.span,
            }
        })
        .collect();

    let (entries, routes, schedules) = match agent {
        Some(name) => (
            vec![estimate(name).ok_or_else(|| AgentGraphError::NoSuchAgent(name.to_string()))?],
            Vec::new(),
            Vec::new(),
        ),
        None => (all_entries, all_routes, all_schedules),
    };
    Ok(AgentGraph { entries, routes, schedules, actions, declared, never_reached })
}

/// What every callable a tree can name is: tools, connector operations,
/// Python functions (by `module.function`, as the tree names them),
/// prompts, agents and actions.
fn declarations(file: &corvid_ast::File) -> BTreeMap<String, Declared> {
    fn names(row: &corvid_ast::EffectRow) -> Vec<String> {
        row.effects.iter().map(|effect| effect.name.name.clone()).collect()
    }
    fn tool(effect: &corvid_ast::Effect, row: &corvid_ast::EffectRow, kind: StepKind) -> Declared {
        let dangerous = matches!(effect, corvid_ast::Effect::Dangerous);
        let mut effects = names(row);
        if dangerous {
            effects.insert(0, "dangerous".to_string());
        }
        Declared { kind, effects, dangerous }
    }
    let mut out = BTreeMap::new();
    for decl in &file.decls {
        match decl {
            Decl::Tool(t) => {
                out.insert(t.name.name.clone(), tool(&t.effect, &t.effect_row, StepKind::Tool));
            }
            Decl::Connector(connector) => {
                for op in &connector.operations {
                    out.insert(op.name.name.clone(), tool(&op.effect, &op.effect_row, StepKind::ConnectorOperation));
                }
            }
            Decl::Import(import) if import.source == corvid_ast::ImportSource::Python => {
                for function in &import.python_functions {
                    out.insert(
                        format!("{}.{}", import.module, function.name.name),
                        Declared { kind: StepKind::PythonFunction, effects: names(&import.effect_row), dangerous: false },
                    );
                }
            }
            Decl::Prompt(p) => {
                out.insert(p.name.name.clone(), Declared { kind: StepKind::Prompt, effects: names(&p.effect_row), dangerous: false });
            }
            Decl::Agent(a) => {
                let kind = if a.action.is_some() { StepKind::Action } else { StepKind::Agent };
                out.insert(a.name.name.clone(), Declared { kind, effects: Vec::new(), dangerous: false });
            }
            _ => {}
        }
    }
    out
}

/// The steps a reader sees under `children`: grouping nodes (a statement,
/// a condition) are not steps, so their children take their place. Every
/// form of the graph draws these.
pub fn steps(children: &[CostTreeNode]) -> Vec<&CostTreeNode> {
    let mut out = Vec::new();
    for child in children {
        match child.kind {
            CostNodeKind::Sequence | CostNodeKind::Condition => out.extend(steps(&child.children)),
            _ => out.push(child),
        }
    }
    out
}

/// Whether `node` holds a loop nothing bounds. Its numbers are then one
/// pass, not a worst case, and every form says there is no bound instead
/// of showing them.
pub fn has_unbounded_loop(node: &CostTreeNode) -> bool {
    matches!(node.kind, CostNodeKind::Loop { iterations: None }) || node.children.iter().any(has_unbounded_loop)
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

/// Each `form_for(action, ...)` and `button_for(action, ...)` in `body`:
/// `(builder, action, where)`.
fn action_builders(body: &corvid_ast::Block) -> Vec<(&'static str, String, Span)> {
    let mut out = Vec::new();
    corvid_ast::visit::walk_block(body, &mut |_| {}, &mut |expr| {
        if let Expr::Call { callee, args, span, .. } = expr {
            let (Expr::Ident { name, .. }, Some(Expr::Ident { name: action, .. })) = (callee.as_ref(), args.first())
            else {
                return;
            };
            let builder = match name.name.as_str() {
                "form_for" => "form_for",
                "button_for" => "button_for",
                _ => return,
            };
            out.push((builder, action.name.clone(), *span));
        }
    });
    out
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
