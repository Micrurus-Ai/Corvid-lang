//! Typed path placeholders in the checker: `path.board` has the
//! declared type, and only types a path segment can be read as exactly
//! are allowed.

use super::check;
use crate::errors::TypeErrorKind;

#[test]
fn a_typed_path_parameter_has_its_type_in_the_handler() {
    let checked = check(
        "type Board:
    id: Int

server board_app:
    route GET \"/boards/{board: Int}\" -> json Board:
        return Board(path.board + 1)
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn an_untyped_path_parameter_stays_a_string() {
    let checked = check(
        "server board_app:
    route GET \"/boards/{board}\" -> json Int:
        return path.board + 1
",
    );
    assert!(!checked.errors.is_empty(), "adding 1 to a String path parameter must not check");
}

#[test]
fn a_type_a_segment_cannot_be_read_as_is_refused_by_name() {
    let checked = check(
        "server board_app:
    route GET \"/reports/{day: Decimal}\" -> json Int:
        return 1
",
    );
    let error = checked
        .errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::RoutePathParamType { param, ty, .. } if param == "day" && ty == "Decimal"))
        .unwrap_or_else(|| panic!("got {:?}", checked.errors));
    assert!(error.kind.message().contains("cannot be read as"));
}
