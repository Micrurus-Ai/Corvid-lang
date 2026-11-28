use super::ir_of;
use corvid_codegen_cl::compile_to_object;

const IDENTITY: &str = r#"identity users:
    provider google
    provisioning:
        first_login: invited
        tenant: fixed("public")
    roles:
        board_member: "board.edit"
"#;

#[test]
fn an_authenticated_route_can_read_checked_path_and_caller_fields() {
    let ir = ir_of(
        &[
            IDENTITY,
            r#"
traces: none

server board:
    route GET "/cards/{card: Int}" -> json Int requires authenticated:
        return path.card
    route GET "/owner" -> json String requires authenticated:
        return actor.id
"#,
        ]
        .concat(),
    );
    let dir = tempfile::tempdir().unwrap();
    compile_to_object(
        &ir,
        "board",
        &dir.path().join("board.o"),
        None,
        None,
        None,
        true,
    )
    .unwrap();
}

#[test]
fn a_library_cannot_compile_away_an_actions_caller_check() {
    let ir = ir_of(
        &[
            IDENTITY,
            r#"
action rename(card: Int, by: Caller) -> Int requires permission("board.edit"):
    return card
"#,
        ]
        .concat(),
    );
    let dir = tempfile::tempdir().unwrap();
    let error = compile_to_object(
        &ir,
        "board",
        &dir.path().join("board.o"),
        None,
        None,
        None,
        true,
    )
    .expect_err("the interpreter authorizes every action call with a Caller");
    assert!(error.to_string().contains("authorization check"), "{error}");
}
