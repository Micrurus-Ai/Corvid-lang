//! Typed path placeholders (B14, agreed with frontend development on
//! 2026-11-25): `"/boards/{board: Int}/cards"` declares `board` an
//! `Int` and is served as `"/boards/{board}/cards"`; `{board}` alone is
//! a `String`. Anything else between braces is an error naming the two
//! forms.

use corvid_ast::{Decl, HttpRouteDecl, TypeRef};
use corvid_syntax::{lex, parse_file};

fn routes(src: &str) -> (Vec<HttpRouteDecl>, Vec<String>) {
    let tokens = lex(src).expect("lex");
    let (file, errors) = parse_file(&tokens);
    let routes = file
        .decls
        .into_iter()
        .filter_map(|decl| match decl {
            Decl::Server(server) => Some(server.routes),
            _ => None,
        })
        .flatten()
        .collect();
    (routes, errors.iter().map(|e| format!("{e:?}")).collect())
}

fn type_name(ty: &TypeRef) -> &str {
    match ty {
        TypeRef::Named { name, .. } => &name.name,
        other => panic!("a named type, got {other:?}"),
    }
}

const SERVER: &str = "server board_app:\n";

#[test]
fn a_typed_placeholder_declares_its_type_and_the_path_is_served_without_it() {
    let src = format!(
        "{SERVER}    route GET \"/boards/{{board: Int}}/cards/{{card}}\" -> json Int:\n        return path.board\n    live \"/boards/{{board: Int}}/cards\" -> Card:\n        return Card.all()\n"
    );
    let (routes, errors) = routes(&src);
    assert!(errors.is_empty(), "{errors:?}");
    let route = &routes[0];
    assert_eq!(route.path, "/boards/{board}/cards/{card}", "the router and the contract see the plain form");
    let params: Vec<(&str, &str)> = route.path_params.iter().map(|p| (p.name.name.as_str(), type_name(&p.ty))).collect();
    assert_eq!(params, [("board", "Int"), ("card", "String")], "an untyped placeholder is a String");
    assert_eq!(routes[1].path, "/boards/{board}/cards", "`live` reads placeholders the same way");
    assert_eq!(type_name(&routes[1].path_params[0].ty), "Int");
}

#[test]
fn a_placeholders_name_points_at_the_name_in_the_source() {
    let src = format!("{SERVER}    route GET \"/boards/{{board: Int}}\" -> json Int:\n        return path.board\n");
    let (routes, _) = routes(&src);
    let param = &routes[0].path_params[0];
    assert_eq!(&src[param.name.span.start..param.name.span.end], "board");
    let TypeRef::Named { span, .. } = &param.ty else { panic!("named") };
    assert_eq!(&src[span.start..span.end], "Int");
}

#[test]
fn a_placeholder_written_any_other_way_is_an_error_naming_both_forms() {
    for bad in ["{9board}", "{board:}", "{board: List<Int>}", "{}", "{board"] {
        let src = format!("{SERVER}    route GET \"/boards/{bad}\" -> json Int:\n        return 1\n");
        let (_, errors) = routes(&src);
        assert!(
            errors.iter().any(|e| e.contains("`{name}` or `{name: Type}`")),
            "`{bad}` must be refused naming the forms, got {errors:?}"
        );
    }
}

#[test]
fn an_explicit_string_type_is_refused_in_favour_of_the_plain_placeholder() {
    let src = format!("{SERVER}    route GET \"/boards/{{board: String}}\" -> json String:\n        return path.board\n");
    let (_, errors) = routes(&src);
    assert!(
        errors.iter().any(|e| e.contains("`{board}`: a placeholder without a type is already a `String`")),
        "{errors:?}"
    );
}
