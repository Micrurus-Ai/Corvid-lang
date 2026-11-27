//! B10-2b-5 — forms for actions. `form_for` takes an action by name where
//! it takes a route, `button_for(action, value, label)` posts a prepared
//! input, and both render a form for `POST /actions/<name>` carrying the
//! reserved `corvid_csrf` field. The compiler checks each against the
//! action's input, so renaming an input field is an error at the form.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
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

action move_card(move: MoveCard, by: Caller) -> Moved requires permission("board.edit"):
    return Moved(move.card, move.position)

action archive_card(card: Int) -> Int requires permission("board.edit"):
    return card

action rename(card: Int, title: String) -> Int requires permission("board.edit"):
    return card
"#;

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(&format!("{IDENTITY}{BOARD}{source}")) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

async fn rendered(agent: &str) -> String {
    let ir = compile_to_ir(&format!("{IDENTITY}{BOARD}{agent}")).unwrap_or_else(|e| panic!("must compile: {e:#?}"));
    let runtime = Runtime::builder().build();
    match run_ir_with_runtime(&ir, Some("page"), vec![], &runtime).await.expect("renders") {
        Value::String(html) => html.to_string(),
        other => panic!("expected html, got {other:?}"),
    }
}

#[tokio::test]
async fn form_for_an_action_posts_its_input_with_the_csrf_field() {
    let html = rendered(
        r#"
agent page() -> String:
    return form_for(move_card, MoveCard(3, Done, 7), "Move")
"#,
    )
    .await;
    assert!(html.starts_with(r#"<form method="POST" action="/actions/move_card" data-corvid-action="move_card">"#), "{html}");
    assert!(html.contains(r#"<input type="hidden" name="corvid_csrf" value="">"#), "the page script fills the token: {html}");
    assert!(html.contains(r#"name="card" value="3""#), "{html}");
    // An enum field is a choice of its variants, with the value selected.
    assert!(html.contains(r#"<select name="to"><option>Todo</option><option selected>Done</option></select>"#), "{html}");
    assert!(html.contains(r#"name="position" value="7""#), "{html}");
    assert!(!html.contains("by"), "the caller is never part of the form: {html}");
    assert!(html.ends_with(r#"<button type="submit">Move</button></form>"#), "{html}");
}

#[tokio::test]
async fn button_for_posts_a_prepared_input_in_hidden_fields() {
    let one = rendered(
        r#"
agent page() -> String:
    return button_for(archive_card, 42, "Archive")
"#,
    )
    .await;
    assert!(one.contains(r#"action="/actions/archive_card""#), "{one}");
    assert!(one.contains(r#"<input type="hidden" name="card" value="42">"#), "one value travels as the action's one input: {one}");
    assert!(one.contains(r#"name="corvid_csrf""#) && one.ends_with(r#"<button type="submit">Archive</button></form>"#), "{one}");

    let record = rendered(
        r#"
agent page() -> String:
    return button_for(move_card, MoveCard(3, Done, 0), "Finish")
"#,
    )
    .await;
    for field in [r#"name="card" value="3""#, r#"name="to" value="Done""#, r#"name="position" value="0""#] {
        assert!(record.contains(&format!(r#"<input type="hidden" {field}>"#)), "a record travels field by field: {record}");
    }
}

#[test]
fn a_form_is_checked_against_the_actions_input() {
    let wrong = errors(
        r#"
agent page() -> String:
    return form_for(move_card, Moved(3, 7), "Move")
"#,
    );
    assert!(wrong.iter().any(|e| e.contains("MoveCard") && e.contains("Moved")), "{wrong:#?}");

    let not_action = errors(
        r#"
agent page() -> String:
    return form_for(page_helper, MoveCard(3, Done, 7), "Move")

agent page_helper(card: Int) -> Int:
    return card
"#,
    );
    assert!(not_action.iter().any(|e| e.contains("`page_helper` is not an action")), "{not_action:#?}");

    let several = errors(
        r#"
agent page() -> String:
    return form_for(rename, MoveCard(3, Done, 7), "Rename")
"#,
    );
    assert!(several.iter().any(|e| e.contains("needs an action whose input is one record") && e.contains("2 inputs")), "{several:#?}");

    let button_several = errors(
        r#"
agent page() -> String:
    return button_for(rename, 3, "Rename")
"#,
    );
    assert!(button_several.iter().any(|e| e.contains("calls an action with one value") && e.contains("`form_for`")), "{button_several:#?}");

    let button_wrong = errors(
        r#"
agent page() -> String:
    return button_for(archive_card, "3", "Archive")
"#,
    );
    assert!(button_wrong.iter().any(|e| e.contains("Int") && e.contains("String")), "{button_wrong:#?}");
}

#[test]
fn an_action_input_cannot_take_the_csrf_fields_name() {
    for source in [
        "action tag(corvid_csrf: String) -> String requires anyone:\n    return corvid_csrf\n",
        "type Tagged:\n    corvid_csrf: String\n\naction tag(t: Tagged) -> String requires anyone:\n    return t.corvid_csrf\n",
    ] {
        let found = errors(source);
        assert!(found.iter().any(|e| e.contains("`corvid_csrf`") && e.contains("CSRF")), "{source}\n{found:#?}");
    }
}

/// Forms take a field exactly when the server reads it from one form
/// value (`text_parameters::query_field_shape`): a type of plain variants
/// is a choice, and a shape the server reads but a form cannot show yet
/// is refused by name.
#[test]
fn a_route_form_takes_every_field_one_form_value_carries() {
    let route = |field: &str| {
        format!(
            "type Filter:\n    name: String\n    {field}\n\nserver api:\n    route POST \"/save\" form Filter -> html:\n        return \"saved\"\n\nagent page() -> String:\n    return form_for(\"POST /save\", Filter(\"milk\", {}), \"Save\")\n",
            if field.contains("Column") { "Done" } else { "1.5" }
        )
    };
    let column = errors(&route("column: Column"));
    assert!(column.is_empty(), "{column:#?}");
    let decimal = errors(&route("amount: Decimal"));
    assert!(decimal.iter().any(|e| e.contains("`amount`") && e.contains("cannot show yet")), "{decimal:#?}");
}
