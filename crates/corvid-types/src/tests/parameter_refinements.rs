//! Refinements on parameters (PR-1): a parameter takes the `where`
//! refinement a field takes, a constant argument outside it is a compile
//! error at the call, and a computed one is left to the check when the
//! call starts.

use super::check;
use crate::errors::TypeErrorKind;

fn kinds(src: &str) -> Vec<TypeErrorKind> {
    check(src).errors.into_iter().map(|e| e.kind).collect()
}

#[test]
fn a_constant_argument_outside_a_parameter_refinement_is_a_compile_error() {
    let errors = kinds(
        "tool fetch_page(limit: Int where between(1, 50)) -> String

prompt titled(name: String where len_between(1, 20)) -> String:
    \"Title for {name}\"

agent page(limit: Int where between(1, 50)) -> Int:
    return limit

agent main() -> Int:
    a = fetch_page(500)
    b = titled(\"\")
    return page(0)
",
    );
    let refused: Vec<(String, String, String)> = errors
        .iter()
        .filter_map(|e| match e {
            TypeErrorKind::ArgumentOutOfRange { callable, param, value, .. } => {
                Some((callable.clone(), param.clone(), value.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        vec![
            ("fetch_page".to_string(), "limit".to_string(), "500".to_string()),
            ("titled".to_string(), "name".to_string(), "\"\"".to_string()),
            ("page".to_string(), "limit".to_string(), "0".to_string()),
        ],
        "{errors:?}"
    );
}

#[test]
fn a_constant_inside_the_refinement_and_a_computed_argument_compile() {
    let checked = check(
        "agent page(limit: Int where between(1, 50)) -> Int:
    return limit

agent main(n: Int) -> Int:
    return page(50) + page(-1 + 2) + page(n)
",
    );
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}

#[test]
fn a_parameter_refinement_must_fit_its_type() {
    let errors = kinds(
        "agent a(name: String where between(1, 5)) -> String:
    return name

agent b(n: Int where len_between(1, 5)) -> Int:
    return n

agent c(n: Int where between(9, 1)) -> Int:
    return n
",
    );
    let invalid: Vec<&str> = errors
        .iter()
        .filter_map(|e| match e {
            TypeErrorKind::ParamRefinementInvalid { callable, .. } => Some(callable.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(invalid, ["a", "b", "c"], "{errors:?}");
}

#[test]
fn a_refinement_and_an_ownership_annotation_combine_on_a_parameter() {
    let checked = check(
        "agent take(name: String where len_between(1, 80) @borrowed) -> String:
    return name
",
    );
    assert!(checked.errors.is_empty(), "{:?}", checked.errors);
}

fn cost_program(param: &str, body_prefix: &str) -> String {
    format!(
        "effect paid_notice:\n    cost: $0.30\n\ntool notify(n: Int) -> String uses paid_notice\n\n@budget($1.00)\nagent notify_all(limit: {param}) -> String:\n    sent = \"\"\n{body_prefix}    for i in range(0, limit):\n        sent = notify(i)\n    return sent\n"
    )
}

fn unbounded(c: &crate::Checked) -> bool {
    c.warnings.iter().any(|w| matches!(w.kind, crate::errors::TypeWarningKind::UnboundedCostAnalysis { .. }))
}

fn over_budget(c: &crate::Checked) -> bool {
    c.errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::EffectConstraintViolation { dimension, .. } if dimension == "cost"))
}

/// Parameter refinements PR-4: a loop over `range(0, limit)` runs at most
/// `limit`'s `between` maximum times, so `@budget` can bound it, as a
/// refined field's `take` already is.
#[test]
fn a_loop_over_a_refined_parameter_is_bounded_by_its_refinement() {
    let three = check(&cost_program("Int where between(1, 3)", ""));
    assert!(!over_budget(&three) && !unbounded(&three), "errors {:?} warnings {:?}", three.errors, three.warnings);

    let fifty = check(&cost_program("Int where between(1, 50)", ""));
    assert!(over_budget(&fifty), "fifty notices at $0.30 exceed $1.00: {:?}", fifty.errors);

    let unrefined = check(&cost_program("Int", ""));
    assert!(unbounded(&unrefined), "an unrefined count has no bound: {:?}", unrefined.warnings);
}

/// A parameter the body rebinds is checked only when the call starts, so
/// its refinement says nothing about the value the loop reads.
#[test]
fn a_rebound_parameter_bounds_no_loop() {
    let rebound = check(&cost_program("Int where between(1, 3)", "    limit = 1000\n"));
    assert!(unbounded(&rebound), "the loop reads 1000, not a value in 1..3: {:?}", rebound.warnings);
}
