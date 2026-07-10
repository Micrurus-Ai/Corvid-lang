//! Slice 45c — end-to-end pin for the builtin-method machinery's
//! pilot: `String.length()` counts Unicode scalar values (Python's
//! `len(str)`), not UTF-8 bytes.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn string_length_counts_unicode_scalars() {
    // "héllo" is 5 Unicode scalars but 6 UTF-8 bytes — the assertion
    // fails if length() ever regresses to byte counting.
    let source = "
agent main() -> Int:
    ascii = \"hello, Corvid\"
    accented = \"héllo\"
    return ascii.length() + accented.length()
";
    let ir = compile_to_ir(source).expect("45c e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45c e2e program must run");
    match out {
        Value::Int(n) => assert_eq!(n, 18, "length must count Unicode scalars, not bytes"),
        other => panic!("expected Int, got {other:?}"),
    }
}
