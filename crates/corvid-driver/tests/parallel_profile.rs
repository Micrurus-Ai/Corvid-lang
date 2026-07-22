//! Effect-aware `parallel` scheduling profiles (slice 52d-1).
//!
//! Compiles real source through the pipeline and checks that the
//! per-arm + combined effect profiles a `parallel:` block computes match
//! the declared effect costs / reversibility — the awareness the 52d
//! scheduler and the cancellation×reversibility rule are built on.

use corvid_ir::IrStmt;
use corvid_vm::parallel_profile::{arm_effect_profile, block_effect_profile};

fn compile(source: &str) -> corvid_ir::IrFile {
    corvid_driver::compile_to_ir(source).expect("source compiles")
}

/// Pull the arm call expressions out of the first `parallel:` block in
/// the named agent.
fn parallel_arm_calls<'a>(ir: &'a corvid_ir::IrFile, agent: &str) -> Vec<&'a corvid_ir::IrExpr> {
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == agent)
        .expect("agent present");
    for stmt in &agent.body.stmts {
        if let IrStmt::Parallel { arms, .. } = stmt {
            return arms.iter().map(|a| &a.call).collect();
        }
    }
    panic!("no parallel block in agent `{agent}`", agent = agent.name);
}

const SOURCE: &str = r#"effect cheap_read:
    cost: $0.01
    trust: autonomous

effect pricey_write:
    cost: $0.50
    trust: autonomous
    reversible: false

tool do_read() -> Bool uses cheap_read
tool do_write() -> Bool uses pricey_write

agent worker() -> Bool:
    parallel:
        r = do_read()
        w = do_write()
    return r
"#;

#[test]
fn per_arm_profiles_reflect_declared_cost_and_reversibility() {
    let ir = compile(SOURCE);
    let calls = parallel_arm_calls(&ir, "worker");
    assert_eq!(calls.len(), 2);

    // Arm `r` calls a cheap, reversible read.
    let read = arm_effect_profile(&ir, calls[0]);
    assert!((read.cost - 0.01).abs() < 1e-9, "read cost: {}", read.cost);
    assert!(read.reversible, "read must be reversible");

    // Arm `w` calls an irreversible write.
    let write = arm_effect_profile(&ir, calls[1]);
    assert!((write.cost - 0.50).abs() < 1e-9, "write cost: {}", write.cost);
    assert!(!write.reversible, "write must be irreversible");
}

#[test]
fn combined_profile_sums_cost_and_ands_reversibility() {
    let ir = compile(SOURCE);
    let calls = parallel_arm_calls(&ir, "worker");
    let (combined, per_arm) = block_effect_profile(&ir, &calls);

    assert_eq!(per_arm.len(), 2);
    // Costs SUM (every arm runs — the parallel operator's Sum).
    assert!(
        (combined.cost - 0.51).abs() < 1e-9,
        "combined cost: {}",
        combined.cost
    );
    // One irreversible arm makes the whole block irreversible.
    assert!(
        !combined.reversible,
        "block with an irreversible arm must be irreversible"
    );
}

#[test]
fn an_all_reversible_block_is_reversible() {
    let source = r#"effect cheap_read:
    cost: $0.02
    trust: autonomous

tool do_read() -> Bool uses cheap_read

agent worker() -> Bool:
    parallel:
        a = do_read()
        b = do_read()
    return a
"#;
    let ir = compile(source);
    let calls = parallel_arm_calls(&ir, "worker");
    let (combined, _) = block_effect_profile(&ir, &calls);
    assert!((combined.cost - 0.04).abs() < 1e-9, "combined cost: {}", combined.cost);
    assert!(combined.reversible, "all-reversible block must be reversible");
}

/// The profile follows agent calls transitively: an arm that calls an
/// agent which calls an irreversible tool is itself irreversible.
#[test]
fn profile_follows_agent_calls_transitively() {
    let source = r#"effect pricey_write:
    cost: $0.30
    trust: autonomous
    reversible: false

tool do_write() -> Bool uses pricey_write

agent inner() -> Bool uses pricey_write:
    return do_write()

agent worker() -> Bool:
    parallel:
        a = inner()
        b = inner()
    return a
"#;
    let ir = compile(source);
    let calls = parallel_arm_calls(&ir, "worker");
    let one = arm_effect_profile(&ir, calls[0]);
    assert!((one.cost - 0.30).abs() < 1e-9, "transitive cost: {}", one.cost);
    assert!(!one.reversible, "transitive irreversibility must propagate");
}
