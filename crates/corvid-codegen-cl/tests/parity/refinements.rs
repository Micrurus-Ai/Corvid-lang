//! B09 Refine-2: native code holds every refined value to its refinement,
//! as the interpreter does. A value in range builds the same record on
//! both tiers; one out of range stops both with the same message, where
//! the record is built or the field assigned.

use super::scalar_harness::{assert_parity_stop, assert_parity_text};

const SEARCH: &str = "type Search:\n    query: String where len_between(1, 20)\n    limit: Int where between(1, 50)\n\n";

fn program(body: &str) -> String {
    format!("{SEARCH}{body}")
}

#[test]
fn a_value_in_range_builds_the_same_record_on_both_tiers() {
    let text = assert_parity_text(&program(
        "agent widen(n: Int) -> Int:\n    search = Search(\"dune\", n)\n    search.limit = search.limit + 10\n    return search.limit\n\nagent main() -> Int:\n    return widen(25)\n",
    ));
    assert_eq!(text, "35");
}

#[test]
fn an_int_out_of_range_stops_where_the_record_is_built() {
    assert_parity_stop(
        &program("agent build(n: Int) -> Int:\n    search = Search(\"dune\", n)\n    return search.limit\n\nagent main() -> Int:\n    return build(500)\n"),
        "`Search.limit` must be between(1, 50), but this value is 500",
    );
}

#[test]
fn a_field_assignment_out_of_range_stops_there() {
    assert_parity_stop(
        &program("agent shrink(n: Int) -> Int:\n    search = Search(\"dune\", 10)\n    search.limit = n\n    return search.limit\n\nagent main() -> Int:\n    return shrink(0 - 3)\n"),
        "`Search.limit` must be between(1, 50), but this value is -3",
    );
}

/// A text value is shown as the interpreter shows it: quoted, with a
/// quote inside escaped.
#[test]
fn a_string_out_of_length_stops_with_the_value_quoted() {
    assert_parity_stop(
        &program("agent name(q: String) -> Int:\n    search = Search(q, 5)\n    return search.limit\n\nagent main() -> Int:\n    return name(\"a \\\"long\\\" query of many words\")\n"),
        "`Search.query` must be len_between(1, 20), but this value is \"a \\\"long\\\" query of many words\"",
    );
}

const BOARD: &str = "type BoardError:\n    | ColumnFull(limit: Int where between(1, 50))\n    | Closed\n\n";

/// A variant's payload field takes a refinement as a record field does:
/// in range it builds the same value on both tiers, and out of range it
/// stops both where the variant is built, naming the variant.
#[test]
fn a_variant_payload_holds_its_refinement_on_both_tiers() {
    let text = assert_parity_text(&format!(
        "{BOARD}agent limit_of(n: Int) -> Int:\n    e = ColumnFull(n)\n    return match e:\n        ColumnFull(limit) -> limit\n        Closed -> 0\n\nagent main() -> Int:\n    return limit_of(12)\n"
    ));
    assert_eq!(text, "12");
    assert_parity_stop(
        &format!("{BOARD}agent build(n: Int) -> Int:\n    e = ColumnFull(n)\n    return match e:\n        ColumnFull(limit) -> limit\n        Closed -> 0\n\nagent main() -> Int:\n    return build(80)\n"),
        "`ColumnFull.limit` must be between(1, 50), but this value is 80",
    );
}
