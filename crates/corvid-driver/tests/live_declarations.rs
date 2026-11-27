//! B14 RT-1 — `live` declarations. A `live` names a table query; a
//! subscriber receives the matching rows, then each committed change.
//! These pin the declaration: it parses beside routes, its body must be
//! one query of the table it delivers, the contract lists it by name
//! with its row type and permission, and `corvid serve` refuses to start
//! a program with one until the WebSocket transport (RT-3) serves it.

use corvid_driver::{
    check_contract_closure, compile_to_application_contract_with_config, compile_to_ir, RuntimeCapabilities,
};

const BOARD: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        board_member: "board.view"

database boards:
    engine: sqlite
    location: "boards.sqlite"

table Card:
    key card: Int
    board: String
    column: String
    title: String
"#;

fn program(server: &str) -> String {
    format!("{BOARD}\n{server}")
}

fn errors(server: &str) -> Vec<String> {
    match compile_to_ir(&program(server)) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

const LIVE: &str = r#"server board_app:
    live "/boards/{board}/cards" -> Card requires permission("board.view"):
        return Card.where(fn (c) -> c.board == path.board).order_by(fn (c) -> c.card)

    route GET "/boards/{board}/cards" -> json List<Card>:
        return Card.where(fn (c) -> c.board == path.board)
"#;

#[test]
fn a_live_declaration_checks_beside_a_route_on_the_same_path() {
    let found = errors(LIVE);
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_contract_lists_a_live_by_name_apart_from_the_routes() {
    let contract = compile_to_application_contract_with_config(&program(LIVE), "app.cor", "test", None)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(contract.live.len(), 1);
    let live = &contract.live[0];
    assert_eq!(live.name, "cards");
    assert_eq!(live.path, "/boards/{board}/cards");
    assert_eq!(live.row_type, "Card");
    assert_eq!(live.path_params[0].name, "board");
    assert_eq!(live.policy.as_ref().map(|p| p.permissions.clone()), Some(vec!["board.view".to_string()]));
    assert_eq!(contract.routes.len(), 1, "only the GET route is an HTTP route");
}

#[test]
fn generated_live_client_uses_the_table_schema_and_typed_route() {
    let source = include_str!("../../../sdk/typescript/client/test/boards.cor");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/typescript/client/test/boards.cor");
    let check = |source: &str| corvid_driver::compile_to_ir_with_config_at_path(source, &path, None);
    let contract = compile_to_application_contract_with_config(source, path.to_str().unwrap(), "test", None)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let live = &contract.live[0];
    assert_eq!(live.server, "board_app");
    assert_eq!(live.key_fields, ["board", "card"]);
    assert_eq!(live.path, "/boards/{board}/cards");
    assert_eq!(live.path_params[0].type_name, "Int");
    let card = contract.types.iter().find(|ty| ty.name == live.row_type).unwrap();
    let fields: Vec<_> = card.fields.iter().map(|f| (f.name.as_str(), f.type_name.as_str())).collect();
    assert_eq!(fields, [("board", "Int"), ("card", "Int"), ("title", "String"), ("revision", "Int")]);
    let files = corvid_abi::ts_client::emit_ts_client(&contract);
    let api = &files.iter().find(|file| file.filename == "api.ts").unwrap().contents;
    assert!(api.contains("path_board: number, observer: import(\"@corvid/client\").LiveObserver<Card>"));
    assert!(api.contains("[\"board\",\"card\"]"));

    let invalid = source.replace("{board: Int}", "{board}");
    assert!(check(&invalid).is_err(), "String path.board cannot match an Int column");
    let invalid = source.replace("live \"/boards/{board: Int}", "live \"/boards/{board: Card}");
    let diagnostics = check(&invalid).unwrap_err();
    assert!(diagnostics.iter().any(|error|
        error.message.contains("path parameter `board` of `/boards/{board}/cards`")
            && error.message.contains("a path segment cannot be read as")
    ), "{diagnostics:#?}");

    let agent_row = source.replace("for card in ui.live", "for agent in ui.live")
        .replace("bind card.title", "bind agent.title");
    check(&agent_row).unwrap_or_else(|errors| panic!("{errors:#?}"));
    for (original, replacement, expected) in [
        ("bind card.title", "bind card.caption", "caption"),
        ("board_app.cards, board", "board_app.unknown, board", "no live declaration"),
        ("board_app.cards, board", "board_app.cards", "ui.live path"),
        ("board_app.cards, board", "board_app.cards, \"7\"", "live path parameter"),
    ] {
        let errors = check(&source.replace(original, replacement)).unwrap_err();
        assert!(errors.iter().any(|e| e.message.contains(expected)), "{errors:#?}");
    }
}

#[test]
fn serve_refuses_to_start_a_live_until_the_transport_serves_it() {
    let ir = compile_to_ir(&program(LIVE)).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let gaps = check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier());
    assert_eq!(gaps.len(), 1, "{gaps:#?}");
    assert!(gaps[0].message().contains("live /boards/{board}/cards needs live subscriptions over the WebSocket transport"));
    let mut transport = RuntimeCapabilities::interpreter_tier();
    transport.live_subscriptions = true;
    assert!(check_contract_closure(&ir, transport).is_empty());
}

#[test]
fn a_live_body_is_one_query_of_the_table_it_delivers() {
    let not_a_query = errors("server board_app:\n    live \"/cards\" -> Card requires anyone:\n        cards = Card.all()\n        return cards\n");
    assert!(not_a_query.iter().any(|m| m.contains("must be one `return` of a table query")), "{not_a_query:#?}");

    let other_table = errors(
        "table Note:\n    key id: Int\n    text: String\n\nserver board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Note.all()\n",
    );
    assert!(!other_table.is_empty(), "{other_table:#?}");

    let not_a_table = errors("type Draft:\n    title: String\n\nserver board_app:\n    live \"/drafts\" -> Draft requires anyone:\n        return []\n");
    assert!(not_a_table.iter().any(|m| m.contains("delivers `Draft`, which is not a table")), "{not_a_table:#?}");

    let take = errors("server board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Card.all().take(10)\n");
    assert!(take.iter().any(|m| m.contains("uses `take`")), "{take:#?}");

    // Who may watch is written, never left to a default.
    let unguarded = errors("server board_app:\n    live \"/cards\" -> Card:\n        return Card.all()\n");
    assert!(unguarded.iter().any(|m| m.contains("declares no `requires`") && m.contains("`requires anyone` for a public list")), "{unguarded:#?}");
    assert!(errors("server board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Card.all()\n").is_empty());

    let count = errors("server board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Card.all().count()\n");
    assert!(count.iter().any(|m| m.contains("ends in `count`; a live query delivers rows")), "{count:#?}");

    // A list may hold literals and table queries, whose tables the feed
    // watches; a call by name could read anything, so it is refused.
    let called = errors("agent chosen() -> List<Int>:\n    return [1, 2]\n\nserver board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Card.where(fn (c) -> chosen().contains(c.card))\n");
    assert!(called.iter().any(|m| m.contains("filters by a list that calls `chosen`")), "{called:#?}");
    let literal = errors("server board_app:\n    live \"/cards\" -> Card requires anyone:\n        return Card.where(fn (c) -> [1, 2].contains(c.card))\n");
    assert!(!literal.iter().any(|m| m.contains("filters by a list")), "a literal list reads no table: {literal:#?}");
}

#[test]
fn two_lives_in_a_server_need_different_names() {
    let found = errors(
        "server board_app:\n    live \"/boards/{board}/cards\" -> Card requires anyone:\n        return Card.where(fn (c) -> c.board == path.board)\n\n    live \"/archive/cards\" -> Card requires anyone:\n        return Card.all()\n",
    );
    assert!(found.iter().any(|m| m.contains("is named `cards`, the same as another `live`")), "{found:#?}");
    let unnamed = errors("server board_app:\n    live \"/{board}\" -> Card requires anyone:\n        return Card.all()\n");
    assert!(unnamed.iter().any(|m| m.contains("has no fixed path segment")), "{unnamed:#?}");
}

#[test]
fn live_stays_an_ordinary_name_outside_a_server() {
    let found = errors("agent count(live: Int) -> Int:\n    return live + 1\n");
    assert!(found.is_empty(), "{found:#?}");
}
