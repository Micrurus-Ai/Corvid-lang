//! `corvid graph` (B47): print each entry point's tree, the one `@budget`
//! checks, as steps a reader follows, and the tools and prompts nothing
//! reaches. See `docs/proposals/agent-graph.md`. This facade checks the
//! program and hands the graph to one form's renderer.

mod text;

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
    print!("{}", text::render(&graph, &place));
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
