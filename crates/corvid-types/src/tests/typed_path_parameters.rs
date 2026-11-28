//! Request text in the checker: typed path placeholders and query
//! fields take every type the server reads exactly, through the shape
//! `corvid_types::text_parameters` gives both.

use super::check;
use crate::errors::TypeErrorKind;

#[test]
fn a_typed_path_parameter_has_its_type_in_the_handler() {
    let checked = check(
        "type Board:
    id: Int

traces: none

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
        "traces: none\n\nserver board_app:
    route GET \"/boards/{board}\" -> json Int:
        return path.board + 1
",
    );
    assert!(!checked.errors.is_empty(), "adding 1 to a String path parameter must not check");
}

#[test]
fn every_exact_scalar_and_a_type_of_plain_variants_is_a_path_parameter() {
    let checked = check(
        "type Status:
    | Open
    | Closed

traces: none

server reports:
    route GET \"/reports/{day: Instant}/{amount: Decimal}/{status: Status}/{ratio: Float}/{draft: Bool}\" -> json Bool:
        return path.draft
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn a_type_one_segment_cannot_hold_is_refused_by_name() {
    let checked = check(
        "type Outcome:
    | Done(code: Int)
    | Pending

traces: none

server reports:
    route GET \"/reports/{outcome: Outcome}\" -> json Int:
        return 1
",
    );
    let error = checked
        .errors
        .iter()
        .find(|e| matches!(&e.kind, TypeErrorKind::RoutePathParamType { param, .. } if param == "outcome"))
        .unwrap_or_else(|| panic!("a variant with fields is not one name: {:?}", checked.errors));
    assert!(error.kind.message().contains("cannot be read as"));
}

#[test]
fn query_fields_may_be_optional_repeated_or_exact_and_nothing_else() {
    let good = check(
        "type Status:
    | Open
    | Closed

type Search:
    status: List<Status>
    since: Option<Instant>
    limit: Option<Int>
    total: Decimal

traces: none

server search:
    route GET \"/cards\" query Search -> json Int:
        return 1
",
    );
    assert!(good.errors.is_empty(), "got {:?}", good.errors);

    let bad = check(
        "type Line:
    sku: String

type Search:
    lines: List<Line>

traces: none

server search:
    route GET \"/cards\" query Search -> json Int:
        return 1
",
    );
    assert!(
        bad.errors
            .iter()
            .any(|e| matches!(&e.kind, TypeErrorKind::RouteQueryFieldType { field, .. } if field == "lines")),
        "a list of records is not query text: {:?}",
        bad.errors
    );
}
