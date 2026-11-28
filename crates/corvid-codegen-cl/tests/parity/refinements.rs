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

/// Refined parameters (PR-2): native code checks a parameter's
/// refinement when a call starts, with the interpreter's message, and
/// lets a value in range through.
#[test]
fn a_refined_parameter_admits_a_value_in_range_on_both_tiers() {
    let text = assert_parity_text(
        "agent page(limit: Int where between(1, 50), term: String where len_between(1, 20)) -> Int:\n    return limit + term.length()\n\nagent main() -> Int:\n    n = 40\n    return page(n + 2, \"dune\")\n",
    );
    assert_eq!(text, "46");
}

#[test]
fn a_computed_argument_outside_a_parameter_refinement_stops_the_call() {
    assert_parity_stop(
        "agent page(limit: Int where between(1, 50)) -> Int:\n    return limit\n\nagent main() -> Int:\n    n = 25\n    return page(n * 20)\n",
        "`limit` is 500, outside `between(1, 50)`, in this call to `page`",
    );
    assert_parity_stop(
        "agent titled(name: String where len_between(1, 5)) -> Int:\n    return name.length()\n\nagent main() -> Int:\n    words = \"too \" + \"long\"\n    return titled(words)\n",
        "`name` is \"too long\", outside `len_between(1, 5)`, in this call to `titled`",
    );
}

/// A tool's refined parameter is checked before the tool runs, natively
/// through the runtime call as on the interpreter.
#[test]
fn a_tool_argument_outside_its_refinement_stops_before_the_tool_runs() {
    assert_parity_stop(
        "tool double_int(n: Int where between(1, 50)) -> Int\n\nagent main() -> Int:\n    n = 250\n    return double_int(n * 2)\n",
        "`n` is 500, outside `between(1, 50)`, in this call to `double_int`",
    );
}
