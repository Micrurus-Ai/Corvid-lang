//! Refined parameters in native code (PR-2): a program with one compiles
//! natively, and the bounds `std/crypto.cor` declares are the limits
//! `corvid-crypto` enforces. Corvid source cannot import a Rust constant,
//! so this test holds the two copies together.

use corvid_driver::{compile_to_ir, native_ability};

#[test]
fn a_refined_parameter_compiles_natively() {
    let ir = compile_to_ir(
        "agent page(limit: Int where between(1, 50)) -> Int:\n    return limit\n\nagent main() -> Int:\n    return page(5)\n",
    )
    .unwrap_or_else(|d| panic!("{d:?}"));
    assert_eq!(native_ability(&ir).map_err(|r| r.to_string()), Ok(()));
}

#[test]
fn std_cryptos_declared_bounds_are_the_crates_limits() {
    let source = std::fs::read_to_string(corvid_test_paths::workspace_root().join("std/crypto.cor")).expect("std/crypto.cor");
    let expect = |declared: &str| assert!(source.contains(declared), "std/crypto.cor declares `{declared}`");
    expect(&format!(
        "random_token(bytes: Int where between({}, {}))",
        corvid_crypto::MIN_TOKEN_BYTES,
        corvid_crypto::MAX_TOKEN_BYTES
    ));
    expect(&format!(
        "memory_kib: Int where between({}, {})",
        corvid_crypto::MIN_MEMORY_KIB,
        corvid_crypto::MAX_MEMORY_KIB
    ));
    expect(&format!("iterations: Int where between(1, {})", u32::MAX));
    expect(&format!("lanes: Int where between(1, {})", corvid_crypto::MAX_LANES));
}
