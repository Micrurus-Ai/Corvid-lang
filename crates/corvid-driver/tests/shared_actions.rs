//! B10-1 — `action` declarations: the operation a person, an assistant
//! and the program share. An action parses beside agents, is called from
//! Corvid code as any agent, always says who may call it from outside
//! (`requires anyone` when everyone may), and the contract lists it with
//! its input, result and caller policy.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const IDENTITY: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        board_member: "board.edit"
"#;

const BOARD: &str = r#"
type Column:
    | Todo
    | Done

type MoveCard:
    card: Int
    to: Column
    position: Int where between(0, 500)

type Moved:
    card: Int
    position: Int

action move_card(move: MoveCard) -> Moved requires permission("board.edit"):
    return Moved(move.card, move.position)

action ping() -> String requires anyone:
    return "pong"

agent tidy(card: Int) -> Moved:
    return move_card(MoveCard(card, Done, 0))
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

#[test]
fn an_action_checks_and_corvid_code_calls_it_as_an_agent() {
    let found = errors(&format!("{IDENTITY}{BOARD}"));
    assert!(found.is_empty(), "{found:#?}");
}

#[tokio::test]
async fn a_typed_call_runs_the_action_body() {
    let ir = compile_to_ir(&format!("{IDENTITY}{BOARD}")).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(ir.actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["move_card", "ping"]);
    assert_eq!(ir.actions[0].policy.permissions, ["board.edit"]);
    assert!(!ir.actions[1].policy.requires_auth(), "`requires anyone` names no authority");
    let moved = run_ir_with_runtime(&ir, Some("tidy"), vec![Value::Int(7)], &Runtime::builder().build()).await.expect("runs");
    let Value::Struct(moved) = moved else { panic!("a Moved record, got {moved:?}") };
    assert_eq!(moved.get_field("card"), Some(Value::Int(7)));
}

#[test]
fn an_action_must_say_who_may_call_it() {
    let found = errors("action ping() -> String:\n    return \"pong\"\n");
    assert!(
        found.iter().any(|m| m.contains("`requires` naming who may call the action") && m.contains("requires anyone")),
        "leaving the caller policy out is an error naming the choices: {found:#?}"
    );
    let found = errors("action ping() -> String requires anyone and authenticated:\n    return \"pong\"\n");
    assert!(found.iter().any(|m| m.contains("`requires anyone` on its own")), "{found:#?}");
}

#[test]
fn a_signed_in_policy_needs_an_identity_and_declared_authorities() {
    let found = errors("action wipe() -> String requires authenticated:\n    return \"done\"\n");
    assert!(found.iter().any(|m| m.contains("action `wipe` requires a signed-in caller")), "{found:#?}");
    let found = errors(&format!("{IDENTITY}action wipe() -> String requires permission(\"board.wipe\"):\n    return \"done\"\n"));
    assert!(
        found.iter().any(|m| m.contains("action `wipe` requires permission `board.wipe`")),
        "an authority the identity block does not declare could never be met: {found:#?}"
    );
}

#[test]
fn the_contract_lists_each_action_with_its_callers() {
    let contract = compile_to_application_contract_with_config(&format!("{IDENTITY}{BOARD}"), "app.cor", "test", None)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let names: Vec<&str> = contract.actions.iter().map(|a| a.callable.name.as_str()).collect();
    assert_eq!(names, ["move_card", "ping"]);
    let move_card = &contract.actions[0];
    assert_eq!(move_card.policy.as_ref().map(|p| p.permissions.clone()), Some(vec!["board.edit".to_string()]));
    assert!(contract.actions[1].policy.is_none(), "`requires anyone` carries no policy");
    assert!(
        contract.agents.iter().all(|a| a.name != "move_card"),
        "an action is listed once, as an action"
    );
    let json = serde_json::to_value(&contract).unwrap();
    assert_eq!(json["actions"][0]["name"], "move_card", "the action's callable fields sit on the entry itself");
}

#[test]
fn action_stays_an_ordinary_name_elsewhere() {
    let found = errors("agent run(action: String) -> String:\n    return action\n");
    assert!(found.is_empty(), "{found:#?}");
}
