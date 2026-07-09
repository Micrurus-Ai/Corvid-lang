//! Slice 45b — end-to-end pins for place assignment.
//!
//! Two load-bearing semantics are pinned here through the full
//! driver pipeline (compile → interpret):
//!
//! 1. **Reference semantics.** Structs and lists are shared heap
//!    cells (the Phase 17 memory model): mutation through one
//!    binding is visible through every alias. `alias = w;
//!    alias.balance *= 2.0` doubles the balance `w` sees, and a
//!    list stored into a struct field remains THE SAME list — no
//!    copy-on-write, matching Python's object model.
//! 2. **Compound checked arithmetic.** The compound operator lives
//!    in the IR (never desugared into `target = target op value`)
//!    and reuses the interpreter's checked `eval_binop`.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn place_assignment_reference_semantics_and_compound_ops() {
    let source = r#"
type Wallet:
    balance: Float
    label: String

type Account:
    wallet: Wallet
    scores: List<Int>

agent main() -> String:
    w = Wallet(100.0, "start")
    w.balance = 250.0
    w.balance += 50.0
    w.label = "updated"

    xs = [10, 20, 30]
    xs[1] = 99
    xs[2] += 1

    acct = Account(w, xs)
    acct.wallet.balance -= 100.0
    acct.scores[0] = 7

    alias = w
    alias.balance *= 2.0

    n = 5
    n += 37

    check1 = w.balance
    check2 = xs[0] + xs[1] + xs[2]
    check3 = n
    if check1 == 400.0:
        if check2 == 137:
            if check3 == 42:
                return "ok"
    return "FAILED"
"#;
    let ir = compile_to_ir(source).expect("45b e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45b e2e program must run");
    match out {
        Value::String(s) => assert_eq!(
            s.as_ref(),
            "ok",
            "reference-semantics/compound checks failed inside the program"
        ),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn index_assignment_out_of_bounds_traps() {
    let source = r#"
agent main() -> Int:
    xs = [1, 2, 3]
    xs[9] = 5
    return xs[0]
"#;
    let ir = compile_to_ir(source).expect("source must compile");
    let runtime = Runtime::builder().build();
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await;
    let err = result.expect_err("out-of-bounds store must trap");
    assert!(
        format!("{err:?}").contains("IndexOutOfBounds"),
        "expected an out-of-bounds trap, got {err:?}"
    );
}
