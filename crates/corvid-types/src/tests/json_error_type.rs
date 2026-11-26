//! B01 — `JsonError` is a type a program can write: a helper agent can
//! take the error `from_json` returns, and a signature can return
//! `Result<T, JsonError>`. It is still only produced by `from_json`.

use super::check;
use crate::errors::TypeErrorKind;

#[test]
fn a_helper_agent_takes_the_error_from_json_returns() {
    let checked = check(
        "type Order:
    id: String
    total: Int

agent explain(problem: JsonError) -> String:
    return problem.path + \": \" + problem.message

agent read_order(text: String) -> Result<Order, JsonError>:
    return Order.from_json(text)

agent main(text: String) -> String:
    return match read_order(text):
        Ok(order) -> order.id
        Err(problem) -> explain(problem)
",
    );
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

#[test]
fn json_error_fields_are_typed_in_a_parameter_too() {
    let checked = check("agent where_(problem: JsonError) -> Int:\n    return problem.position\n");
    assert!(
        checked.errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::UnknownField { struct_name, field }
            if struct_name == "JsonError" && field == "position")),
        "got {:?}",
        checked.errors
    );
}

#[test]
fn json_error_is_a_type_not_a_value() {
    let checked = check("agent main() -> Int:\n    e = JsonError\n    return 0\n");
    assert!(
        checked.errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::TypeAsValue { name } if name == "JsonError")),
        "got {:?}",
        checked.errors
    );
}
