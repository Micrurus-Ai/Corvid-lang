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
fn an_action_cannot_take_or_return_a_secret() {
    let found = errors("action sign_in(password: Secret) -> Bool requires anyone:\n    return true\n");
    assert!(
        found.iter().any(|m| m.contains("an action's inputs and result")),
        "a form or an assistant would have to send the secret as JSON: {found:#?}"
    );
}

#[test]
fn a_route_cannot_take_an_actions_path() {
    let found = errors(
        "action ping() -> String requires anyone:\n    return \"pong\"\n\nserver api:\n    route POST \"/actions/ping\" -> json String:\n        return \"mine\"\n",
    );
    assert!(found.iter().any(|m| m.contains("takes the path action `ping` is served at")), "{found:#?}");
    let found = errors(
        "action ping() -> String requires anyone:\n    return \"pong\"\n\nserver api:\n    route GET \"/actions/other\" -> json String:\n        return \"mine\"\n",
    );
    assert!(found.is_empty(), "only an action's own path is taken: {found:#?}");
}

#[test]
fn the_contract_says_when_one_record_is_the_whole_input() {
    let contract = compile_to_application_contract_with_config(&format!("{IDENTITY}{BOARD}"), "app.cor", "test", None)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(contract.actions[0].input_type.as_deref(), Some("MoveCard"));
    assert_eq!(contract.actions[1].input_type, None, "`ping` takes no record");
}

const CALLER: &str = r#"
type Card:
    id: Int
    owner: String

action claim(card: Int, by: Actor) -> Card requires permission("board.edit"):
    return Card(card, by.tenant + "/" + by.id)

server board:
    route POST "/cards/{card: Int}/claim" -> json Card requires authenticated:
        return claim(path.card, actor)
"#;

#[test]
fn an_action_reads_its_caller_and_code_passes_one_along() {
    let found = errors(&format!("{IDENTITY}{CALLER}"));
    assert!(found.is_empty(), "{found:#?}");
    let contract = compile_to_application_contract_with_config(&format!("{IDENTITY}{CALLER}"), "app.cor", "test", None)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let inputs: Vec<&str> = contract.actions[0].callable.inputs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(inputs, ["card"], "the caller is filled from the session, never sent");
}

#[test]
fn only_a_verified_session_supplies_an_actor() {
    let found = errors("action claim(card: Int, by: Actor) -> Int requires anyone:\n    return card\n");
    assert!(found.iter().any(|m| m.contains("`requires anyone` verifies no caller")), "{found:#?}");

    let forged = format!(
        "{IDENTITY}type Claim:\n    card: Int\n    by: Actor\n\naction claim(claim: Claim) -> Int requires authenticated:\n    return claim.card\n"
    );
    let found = errors(&forged);
    assert!(
        found.iter().any(|m| m.contains("an `Actor` cannot be part of an action's input in field `Claim.by`")),
        "a client could name any caller: {found:#?}"
    );

    let route = format!(
        "{IDENTITY}type Claim:\n    by: Actor\n\nserver api:\n    route POST \"/claim\" body Claim -> json String:\n        return claim_owner(body)\n\nagent claim_owner(c: Claim) -> String:\n    return c.by.id\n"
    );
    let found = errors(&route);
    assert!(found.iter().any(|m| m.contains("a route's path, query or body")), "{found:#?}");
}

#[test]
fn action_stays_an_ordinary_name_elsewhere() {
    let found = errors("agent run(action: String) -> String:\n    return action\n");
    assert!(found.is_empty(), "{found:#?}");
}
