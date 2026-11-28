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
    let names: Vec<_> = graph.never_reached.iter().map(|(kind, name, _)| (kind.word(), name.as_str())).collect();
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

#[test]
fn the_document_names_each_step_with_its_id_location_effects_and_cost() {
    use corvid_driver::agent_graph::{GraphDocument, StepKind, AGENT_GRAPH_SCHEMA};
    let doc = GraphDocument::new(&graph(REFUND, Some("refund_bot")), "refund.cor", REFUND);
    assert_eq!(doc.schema, AGENT_GRAPH_SCHEMA);
    let entry = &doc.entries[0];
    assert_eq!((entry.id.as_str(), entry.kind), ("refund_bot", StepKind::Agent));
    assert_eq!(entry.effects, vec!["dangerous", "lookup", "message", "model_call", "payment"]);
    let total = entry.worst_case.as_ref().expect("bounded")["cost"];
    assert!((total - 0.03).abs() < 1e-9, "the number @budget checks, in dollars: {total}");

    let approval = &entry.steps[1];
    assert_eq!(approval.id, "refund_bot/approve.IssueRefund");
    assert_eq!(approval.kind, StepKind::Approval);
    assert_eq!(approval.guards.as_deref(), Some("issue_refund"));
    assert_eq!(approval.dangerous, Some(true));
    assert_eq!((approval.location.line, approval.location.column), (line_of(REFUND, "approve IssueRefund"), 5));

    let branch = &entry.steps[2];
    assert_eq!(branch.kind, StepKind::Branch);
    let refund = &branch.paths[1][0];
    assert_eq!(refund.id, "refund_bot/branch/path2/issue_refund");
    assert_eq!((refund.kind, refund.dangerous), (StepKind::Tool, Some(true)));
    assert_eq!(refund.effects, vec!["dangerous", "payment"]);

    let lp = &entry.steps[3];
    assert_eq!((lp.kind, lp.iterations), (StepKind::Loop, Some(50)));
    assert_eq!(lp.steps[0].id, "refund_bot/loop/notify");

    assert_eq!(doc.never_reached[0].name, "archive_order");
    // B22 and the dev console read the same struct back.
    let text = serde_json::to_string(&doc).unwrap();
    assert_eq!(serde_json::from_str::<GraphDocument>(&text).unwrap(), doc);
}

#[test]
fn an_unbounded_loop_keeps_its_steps_and_reports_no_bound() {
    use corvid_driver::agent_graph::{GraphDocument, StepKind};
    let source = r#"effect message:
    cost: $0.01

tool notify(item: Int) -> String uses message

agent drain(items: List<Int>) -> Int:
    for item in items:
        sent = notify(item)
    return 0
"#;
    let graph = graph(source, None);
    // The body of a loop nothing bounds is still drawn, so its tool is
    // reached, not listed as unused.
    assert!(graph.never_reached.is_empty(), "{:?}", graph.never_reached);
    let doc = GraphDocument::new(&graph, "drain.cor", source);
    let entry = &doc.entries[0];
    assert_eq!(entry.worst_case, None);
    let lp = &entry.steps[0];
    assert_eq!((lp.kind, lp.iterations, lp.worst_case.is_none()), (StepKind::Loop, None, true));
    assert_eq!(lp.steps[0].name.as_deref(), Some("notify"));
}

#[test]
fn a_step_id_survives_adding_an_unrelated_step_and_repeats_are_numbered() {
    use corvid_driver::agent_graph::{GraphDocument, GraphStep};
    fn ids(steps: &[GraphStep], out: &mut Vec<String>) {
        for step in steps {
            out.push(step.id.clone());
            ids(&step.steps, out);
            for path in &step.paths {
                ids(path, out);
            }
        }
    }
    let all = |source: &str| {
        let doc = GraphDocument::new(&graph(source, Some("refund_bot")), "refund.cor", source);
        let mut out = Vec::new();
        ids(&doc.entries[0].steps, &mut out);
        out
    };
    let before = all(REFUND);
    let edited = REFUND.replace(
        "    amount = lookup_order(id)\n",
        "    archived = archive_order(id)\n    amount = lookup_order(id)\n    again = lookup_order(id)\n",
    );
    let after = all(&edited);
    for id in &before {
        assert!(after.contains(id), "`{id}` changed meaning after an insertion upstream: {after:?}");
    }
    assert!(after.contains(&"refund_bot/archive_order".to_string()));
    assert!(after.contains(&"refund_bot/lookup_order#2".to_string()));
}

const BOARD: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        board_member: "board.view"
        board_editor: "board.edit"

database boards:
    engine: sqlite
    location: "boards.sqlite"

table Card:
    key card: Int
    board: String
    column: String
    title: String

effect notify_cost:
    cost: $0.001

tool notify_watchers(card: Int) -> Int uses notify_cost
tool audit_view(card: String) -> Int uses notify_cost

action archive_card(card: Int, by: Caller) -> Int requires permission("board.edit"):
    assistants: confirm "Archive a card."
    sent = notify_watchers(card)
    return card

action rename_card(card: Int, by: Caller) -> Int requires permission("board.edit"):
    return card

traces: none

server board_app:
    live "/boards/{board}/cards" -> Card requires permission("board.view"):
        return Card.where(fn (c) -> c.board == path.board).order_by(fn (c) -> c.card)

    route GET "/cards/{card}" -> html requires permission("board.view"):
        seen = audit_view(path.card)
        return button_for(archive_card, 1, "Archive")

    route GET "/health" -> json Int:
        return 1
"#;

#[test]
fn routes_and_live_declarations_are_roots_and_a_tool_only_a_route_calls_is_reached() {
    use corvid_driver::agent_graph::{GraphDocument, StepKind};
    let graph = graph(BOARD, None);
    assert!(graph.never_reached.is_empty(), "audit_view is called by a route: {:?}", graph.never_reached);
    let doc = GraphDocument::new(&graph, "board.cor", BOARD);
    let ids: Vec<_> = doc.routes.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["board_app:live /boards/{board}/cards", "board_app:GET /cards/{card}", "board_app:GET /health"]);

    let live = &doc.routes[0];
    assert_eq!(live.live.as_deref(), Some("Card"));
    assert_eq!(live.requires.as_ref().map(|r| r.permissions.clone()), Some(vec!["board.view".to_string()]));

    let card = &doc.routes[1];
    assert_eq!(card.steps[0].id, "board_app:GET /cards/{card}/audit_view");
    assert_eq!(card.steps[0].kind, StepKind::Tool);
    assert!((card.worst_case.as_ref().unwrap()["cost"] - 0.001).abs() < 1e-12);
    assert_eq!(card.effects, vec!["notify_cost"]);

    assert_eq!(doc.routes[2].requires, None, "a public route says so, never an empty policy");
}

#[test]
fn an_action_says_its_route_policy_assistant_offer_and_the_forms_that_call_it() {
    use corvid_driver::agent_graph::{GraphDocument, StepKind};
    let doc = GraphDocument::new(&graph(BOARD, None), "board.cor", BOARD);
    let archive = doc.entries.iter().find(|e| e.name == "archive_card").unwrap();
    assert_eq!(archive.kind, StepKind::Action);
    let call = archive.action.as_ref().expect("an action's calling surface");
    assert_eq!((call.method.as_str(), call.path.as_str()), ("POST", "/actions/archive_card"));
    assert_eq!(call.requires.permissions, vec!["board.edit"]);
    let offer = call.assistants.as_ref().expect("offered with confirm");
    assert_eq!((offer.mode.as_str(), offer.description.as_str()), ("confirm", "Archive a card."));
    assert_eq!(call.used_by.len(), 1);
    assert_eq!(call.used_by[0].builder, "button_for");
    assert_eq!(call.used_by[0].container, "board_app:GET /cards/{card}");
    assert_eq!(call.used_by[0].location.line, line_of(BOARD, "return button_for"));

    let rename = doc.entries.iter().find(|e| e.name == "rename_card").unwrap();
    assert_eq!(rename.action.as_ref().unwrap().assistants, None, "nothing is offered to an assistant by default");
    assert!(doc.entries.iter().all(|e| e.kind == StepKind::Action || e.action.is_none()));
}

#[test]
fn a_table_operation_is_one_step_naming_its_table_and_operation() {
    use corvid_driver::agent_graph::{GraphDocument, GraphStep, StepKind};
    let source = BOARD.replace(
        "action rename_card(card: Int, by: Caller) -> Int requires permission(\"board.edit\"):\n    return card\n",
        "action rename_card(card: Int, by: Caller) -> Int requires permission(\"board.edit\"):\n    \
         found = Card.get(card)\n    \
         open = Card.where(fn (c) -> c.column == \"open\").order_by(fn (c) -> c.card).take(10)\n    \
         total = Card.where(fn (c) -> c.board == \"main\").count()\n    \
         return total\n",
    );
    let doc = GraphDocument::new(&graph(&source, None), "board.cor", &source);
    let rename = doc.entries.iter().find(|e| e.name == "rename_card").unwrap();
    let ops: Vec<(&str, Option<&str>, Option<&str>, StepKind)> = rename
        .steps
        .iter()
        .map(|s: &GraphStep| (s.id.as_str(), s.table.as_deref(), s.operation.as_deref(), s.kind))
        .collect();
    assert_eq!(
        ops,
        vec![
            ("rename_card/Card.get", Some("Card"), Some("get"), StepKind::TableOperation),
            // A query chain is one statement, so one step, not one per stage.
            ("rename_card/Card.query", Some("Card"), Some("query"), StepKind::TableOperation),
            ("rename_card/Card.count", Some("Card"), Some("count"), StepKind::TableOperation),
        ]
    );
    assert_eq!(rename.steps[0].name.as_deref(), Some("Card.get"));
    assert_eq!(rename.steps[1].location.line, line_of(&source, "open = Card.where"));
    assert_eq!(rename.worst_case.as_ref().unwrap()["cost"], 0.0, "a table operation adds no cost");

    let live = &doc.routes[0];
    assert_eq!(live.steps.len(), 1);
    assert_eq!(live.steps[0].id, "board_app:live /boards/{board}/cards/Card.query");
}

#[test]
fn a_schedule_is_a_root_naming_the_entry_it_runs() {
    use corvid_driver::agent_graph::GraphDocument;
    let source = r#"effect email:
    cost: $0.05

tool send_brief(text: String) -> Int uses email

public @replayable
agent daily_brief() -> Int uses email:
    return send_brief("ok")

schedule "0 8 * * *" zone "America/New_York" -> daily_brief() uses email
schedule "0 18 * * *" zone "Europe/London" -> daily_brief() uses email
"#;
    let doc = GraphDocument::new(&graph(source, None), "brief.cor", source);
    let ids: Vec<_> = doc.schedules.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["schedule:daily_brief", "schedule:daily_brief#2"]);
    let first = &doc.schedules[0];
    assert_eq!((first.cron.as_str(), first.zone.as_str(), first.runs.as_str()), ("0 8 * * *", "America/New_York", "daily_brief"));
    assert_eq!(first.location.line, line_of(source, "schedule \"0 8"));
    assert_eq!(doc.entries.len(), 1, "the agent it runs is drawn once, as an entry");
    assert_eq!(doc.entries[0].name, "daily_brief");
}
