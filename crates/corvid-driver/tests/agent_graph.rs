//! `corvid graph` (B47): the graph is the cost analyzer's tree, so these
//! tests check what that tree now records for a reader (approvals, call
//! sites, dangerous tools, loop bounds) and which entries and unreached
//! declarations the driver picks.

use corvid_driver::agent_graph::{agent_graph, AgentGraph, AgentGraphError};
use corvid_types::{CostNodeKind, CostTreeNode};
use std::path::Path;

const REFUND: &str = r#"effect lookup:
    cost: $0.00

effect model_call:
    cost: $0.02

effect payment:
    cost: $0.01

effect message:
    cost: $0.0002

tool lookup_order(id: String) -> Int uses lookup
tool issue_refund(id: String, amount: Int) -> String dangerous uses payment
tool notify(item: Int) -> String uses message
tool archive_order(id: String) -> String uses lookup

prompt escalate(amount: Int) -> String uses model_call:
    "Should we escalate a refund of {amount}?"

agent refund_bot(id: String, items: Int where between(1, 50)) -> String:
    amount = lookup_order(id)
    approve IssueRefund(id, amount)
    outcome = ""
    if amount > 100:
        outcome = escalate(amount)
    else:
        outcome = issue_refund(id, amount)
    for i in range(0, items):
        sent = notify(i)
    return outcome
"#;

fn graph(source: &str, agent: Option<&str>) -> AgentGraph {
    match agent_graph(source, Path::new("refund.cor"), None, agent) {
        Ok(graph) => graph,
        Err(error) => panic!("graph failed: {error:?}"),
    }
}

fn find<'a>(node: &'a CostTreeNode, pred: &dyn Fn(&CostTreeNode) -> bool) -> Option<&'a CostTreeNode> {
    if pred(node) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, pred))
}

fn line_of(source: &str, needle: &str) -> usize {
    source[..source.find(needle).expect("needle in source")].matches('\n').count() + 1
}

fn line_at(source: &str, span: corvid_ast::Span) -> usize {
    source[..span.start].matches('\n').count() + 1
}

#[test]
fn the_tree_records_approvals_dangerous_tools_loop_bounds_and_call_sites() {
    let graph = graph(REFUND, Some("refund_bot"));
    let tree = &graph.entries[0].tree;
    assert!(graph.entries[0].bounded);

    let approval = find(tree, &|n| n.kind == CostNodeKind::Approval).expect("an approval step");
    assert_eq!(approval.name, "IssueRefund");
    assert!(approval.costs.values().all(|v| *v == 0.0), "approving costs nothing");
    assert_eq!(line_at(REFUND, approval.span), line_of(REFUND, "approve IssueRefund"));

    let refund = find(tree, &|n| n.name == "issue_refund").expect("the refund tool");
    assert!(refund.dangerous);
    let lookup = find(tree, &|n| n.name == "lookup_order").expect("the lookup tool");
    assert!(!lookup.dangerous, "a free tool is still a step");

    // A prompt is located where it is called, not where it is declared.
    let prompt = find(tree, &|n| n.kind == CostNodeKind::Prompt).expect("the prompt");
    assert_eq!(line_at(REFUND, prompt.span), line_of(REFUND, "outcome = escalate"));

    let lp = find(tree, &|n| matches!(n.kind, CostNodeKind::Loop { .. })).expect("the loop");
    assert_eq!(lp.kind, CostNodeKind::Loop { iterations: Some(50) }, "bounded by the refined parameter");
    assert!(find(tree, &|n| n.kind == CostNodeKind::Branch).is_some());
}

#[test]
fn never_reached_lists_tools_and_prompts_no_entry_point_calls() {
    let graph = graph(REFUND, None);
    let names: Vec<_> = graph.never_reached.iter().map(|(kind, name, _)| (*kind, name.as_str())).collect();
    assert_eq!(names, vec![("tool", "archive_order")]);
    let (_, _, span) = &graph.never_reached[0];
    assert_eq!(line_at(REFUND, *span), line_of(REFUND, "tool archive_order"));
}

#[test]
fn entries_are_agents_nothing_else_calls_and_naming_one_keeps_the_whole_program_view() {
    let source = format!(
        "{REFUND}\nagent helper(id: String) -> Int:\n    return lookup_order(id)\n\n\
         agent front(id: String) -> Int:\n    return helper(id)\n\n\
         agent archiver(id: String) -> String:\n    return archive_order(id)\n"
    );
    let all = graph(&source, None);
    let names: Vec<_> = all.entries.iter().map(|e| e.tree.name.as_str()).collect();
    assert_eq!(names, vec!["refund_bot", "front", "archiver"], "helper is called by front, so it is not an entry");
    assert!(all.never_reached.is_empty());

    // Drawing one agent does not make what the others call look unused.
    let one = graph(&source, Some("refund_bot"));
    assert_eq!(one.entries.len(), 1);
    assert!(one.never_reached.is_empty());
}

#[test]
fn an_unknown_agent_and_a_program_that_does_not_check_are_refused() {
    match agent_graph(REFUND, Path::new("refund.cor"), None, Some("nobody")) {
        Err(AgentGraphError::NoSuchAgent(name)) => assert_eq!(name, "nobody"),
        other => panic!("expected NoSuchAgent, got {other:?}"),
    }
    let broken = "agent a() -> Int:\n    return missing()\n";
    assert!(matches!(
        agent_graph(broken, Path::new("broken.cor"), None, None),
        Err(AgentGraphError::Diagnostics(d)) if !d.is_empty()
    ));
}
