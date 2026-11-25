//! Where the ownership pass places the `Dup`s and `Drop`s around
//! rebinding, branches and loops, checked on the IR it produces. The
//! native parity tests (`parity/loop_ownership.rs`) run the same
//! shapes and check the allocation counts; these pin the placement
//! that makes those counts balance.

use corvid_codegen_cl::dup_drop::insert_dup_drop;
use corvid_ir::{lower, IrAgent, IrBlock, IrStmt};
use corvid_resolve::{resolve, LocalId};
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;

fn agent_after_pass(src: &str, name: &str) -> IrAgent {
    let tokens = lex(src).expect("lex");
    let (file, perr) = parse_file(&tokens);
    assert!(perr.is_empty(), "parse: {perr:?}");
    let resolved = resolve(&file);
    assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
    let checked = typecheck(&file, &resolved);
    assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
    let ir = lower(&file, &resolved, &checked);
    let agent = ir.agents.iter().find(|a| a.name == name).expect("agent");
    insert_dup_drop(agent)
}

fn local_named(agent: &IrAgent, name: &str) -> LocalId {
    fn find(block: &IrBlock, name: &str) -> Option<LocalId> {
        block.stmts.iter().find_map(|stmt| match stmt {
            IrStmt::Let { local_id, name: n, .. } if n == name => Some(*local_id),
            IrStmt::For { var_local, var_name, .. } if var_name == name => Some(*var_local),
            IrStmt::If { then_block, else_block, .. } => {
                find(then_block, name).or_else(|| else_block.as_ref().and_then(|b| find(b, name)))
            }
            IrStmt::For { body, .. } | IrStmt::While { body, .. } => find(body, name),
            _ => None,
        })
    }
    agent
        .params
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.local_id)
        .or_else(|| find(&agent.body, name))
        .unwrap_or_else(|| panic!("no local `{name}`"))
}

fn is_drop_of(stmt: &IrStmt, id: LocalId) -> bool {
    matches!(stmt, IrStmt::Drop { local_id, .. } if *local_id == id)
}

fn drops_in(block: &IrBlock, id: LocalId) -> usize {
    block.stmts.iter().filter(|s| is_drop_of(s, id)).count()
}

fn first_if(block: &IrBlock) -> (&IrBlock, Option<&IrBlock>) {
    block
        .stmts
        .iter()
        .find_map(|s| match s {
            IrStmt::If { then_block, else_block, .. } => Some((then_block, else_block.as_ref())),
            _ => None,
        })
        .expect("an if")
}

fn first_for_body(block: &IrBlock) -> &IrBlock {
    block
        .stmts
        .iter()
        .find_map(|s| match s {
            IrStmt::For { body, .. } => Some(body),
            _ => None,
        })
        .expect("a for loop")
}

/// Native lowering releases the old value when it rebinds `s`, so the
/// pass must not drop `s` in the branch: the only value left there to
/// drop would be the new one.
#[test]
fn a_local_rebound_in_a_branch_is_not_dropped_in_that_branch() {
    let agent = agent_after_pass(
        r#"
agent pick(s: String, flag: Bool) -> String:
    if flag:
        s = "new" + "!"
    return s
"#,
        "pick",
    );
    let s = local_named(&agent, "s");
    let (then_block, else_block) = first_if(&agent.body);
    assert_eq!(drops_in(then_block, s), 0, "{then_block:?}");
    assert!(else_block.is_none(), "nothing dies when the branch is skipped");
    assert_eq!(drops_in(&agent.body, s), 0);
}

/// The condition is the last read of `header`: the drop lands at the
/// start of both branches, so it also runs on the path that returns
/// from inside the branch, and no drop follows the `if`.
#[test]
fn a_local_last_read_by_a_condition_is_dropped_at_the_start_of_each_branch() {
    let agent = agent_after_pass(
        r#"
agent signed(seed: String) -> Int:
    header = seed + ",v1=abc"
    if header.contains("v1="):
        return 1
    return 0
"#,
        "signed",
    );
    let header = local_named(&agent, "header");
    let (then_block, else_block) = first_if(&agent.body);
    assert!(is_drop_of(&then_block.stmts[0], header), "{then_block:?}");
    assert_eq!(drops_in(else_block.expect("a synthesized else"), header), 1);
    assert_eq!(drops_in(&agent.body, header), 0, "{:?}", agent.body.stmts);
}

/// `prefix` is needed only if the loop finishes, and the body always
/// returns without reading it: it is dropped on entering the body.
#[test]
fn a_local_only_the_loop_exit_needs_is_dropped_on_entering_a_body_that_returns() {
    let agent = agent_after_pass(
        r#"
agent first(seed: String, header: String) -> String:
    prefix = seed + "="
    for part in header.split(","):
        return part
    return prefix
"#,
        "first",
    );
    let prefix = local_named(&agent, "prefix");
    let body = first_for_body(&agent.body);
    assert!(is_drop_of(&body.stmts[0], prefix), "{body:?}");
}

/// The receiver of the iterable stays alive while the loop runs, so a
/// `return` from inside the body releases it on the way out.
#[test]
fn the_receiver_of_a_loop_iterable_is_released_by_a_return_from_the_body() {
    let agent = agent_after_pass(
        r#"
agent first(seed: String) -> String:
    header = seed + ",t=9"
    for part in header.split(","):
        return part
    return ""
"#,
        "first",
    );
    let header = local_named(&agent, "header");
    let body = first_for_body(&agent.body);
    assert_eq!(drops_in(body, header), 1, "{body:?}");
}
