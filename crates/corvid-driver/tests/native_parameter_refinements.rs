//! Native code does not check a parameter's `where` refinement yet
//! (PR-2), so a program with one runs on the interpreter, refused by name
//! rather than compiled without the check.

use corvid_driver::{compile_to_ir, native_ability};

#[test]
fn a_refined_parameter_keeps_the_program_on_the_interpreter_by_name() {
    let ir = compile_to_ir(
        "agent page(limit: Int where between(1, 50)) -> Int:\n    return limit\n\nagent main() -> Int:\n    return page(5)\n",
    )
    .unwrap_or_else(|d| panic!("{d:?}"));
    let reason = native_ability(&ir).expect_err("refused until native code checks it").to_string();
    assert!(reason.contains("`limit`") && reason.contains("`page`") && reason.contains("PR-2"), "{reason}");
}
