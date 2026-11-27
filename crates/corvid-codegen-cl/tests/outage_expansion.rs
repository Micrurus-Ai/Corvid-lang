//! The expansion pass and the outage plan agree on which calls that can
//! fail are hoisted (recoverable failures TE-3): every hoisted call is
//! wrapped in `IrOutage::Checked` and followed by its test, and every
//! call the pass leaves in place is one the plan reports as blocking a
//! native `try`. If either side learns a new position without the other,
//! this fails.

use corvid_codegen_cl::outage_plan::OutagePlan;
use corvid_ir::{lower, IrExprKind, IrFile, IrOutage};
use corvid_resolve::resolve;
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;

fn ir_of(src: &str) -> IrFile {
    let tokens = lex(src).expect("lex");
    let (file, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse: {parse_errors:?}");
    let resolved = resolve(&file);
    let checked = typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
    lower(&file, &resolved, &checked)
}

/// The number of calls in `agent`'s expanded body that are checked, and
/// the number of failing calls left unchecked.
fn checked_and_unchecked(ir: &IrFile, plan: &OutagePlan, agent: &str) -> (usize, usize) {
    let agent = ir.agents.iter().find(|a| a.name == agent).expect("agent");
    let expanded = corvid_codegen_cl::dup_drop::insert_dup_drop_with(agent, plan);
    let mut file = ir.clone();
    file.agents = vec![expanded];
    let (mut checked, mut calls) = (0, 0);
    corvid_ir::for_each_expr(&file, &mut |_, e| match &e.kind {
        IrExprKind::Outage(IrOutage::Checked(_)) => checked += 1,
        IrExprKind::Call { kind, .. } if plan.call_can_fail(kind) => calls += 1,
        _ => {}
    });
    (checked, calls - checked)
}

const SRC: &str = r#"
tool lookup(s: String) -> Int

agent fails(s: String) -> Int:
    return lookup(s)

agent hoisted(s: String) -> Int:
    a = fails(s)
    b = 1 + fails(s)
    c = [fails(s), 2].length()
    if fails(s) > 0 and fails(s) > 1:
        return a + b + c
    while fails(s) < 0:
        a = a + 1
    return a + b + c

agent in_arm(name: Option<String>) -> Int:
    return match name:
        Some(n) -> fails(n)
        None -> 0
"#;

#[test]
fn every_failing_call_in_a_hoisted_position_is_checked() {
    let ir = ir_of(SRC);
    let plan = OutagePlan::for_file(&ir);
    // `fails` once in each of: a `let`, an operand, a list item, both
    // sides of `and`, and a loop condition.
    assert_eq!(checked_and_unchecked(&ir, &plan, "hoisted"), (6, 0));
    assert_eq!(checked_and_unchecked(&ir, &plan, "fails"), (1, 0));
}

#[test]
fn a_failing_call_the_pass_leaves_in_place_is_one_the_plan_refuses_a_try_over() {
    let ir = ir_of(SRC);
    let plan = OutagePlan::for_file(&ir);
    assert_eq!(checked_and_unchecked(&ir, &plan, "in_arm"), (0, 1));
    let reason = ir
        .agents
        .iter()
        .find(|a| a.name == "in_arm")
        .map(|agent| {
            let mut found = None;
            corvid_ir::for_each_expr(
                &IrFile { agents: vec![agent.clone()], ..ir.clone() },
                &mut |_, e| {
                    if found.is_none() {
                        if let IrExprKind::Match { .. } = &e.kind {
                            found = plan.try_refusal(e);
                        }
                    }
                },
            );
            found
        })
        .flatten();
    assert!(reason.as_deref().is_some_and(|r| r.contains("`fails`") && r.contains("match")), "{reason:?}");
    assert!(plan.fallible.len() == 3, "fails, hoisted and in_arm can fail: {:?}", plan.fallible);
}
