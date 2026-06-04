//! Compile-time + run-time verification that `#[tool]` produces the
//! contract this macro layer promises:
//!
//!   1. The user's `async fn` remains callable as plain Rust.
//!   2. A `#[no_mangle] pub extern "C" fn __corvid_tool_<name>` wrapper
//!      exists with the expected typed-ABI signature.
//!   3. A `ToolMetadata` entry is visible via `inventory::iter`.
//!
//! End-to-end invocation of the wrapper (which would need the runtime
//! bridge + tokio + the C runtime linked) is handled elsewhere — this
//! test only verifies the macro contract, not dispatch.

use corvid_runtime::{abi::ToolMetadata, inventory};

// Bring the proc-macro into scope with its canonical name.
use corvid_macros::tool;

// ---- (1) Declarations used by the tests below ----

/// Simple scalar round-trip: Int in, Int out. The most common tool shape.
#[tool("double_it")]
async fn double_it(n: i64) -> i64 {
    n * 2
}

/// Boolean flip. Verifies the `bool` ABI path compiles.
#[tool("flip")]
async fn flip(b: bool) -> bool {
    !b
}

/// Float input, Float output. Verifies the `f64` ABI path compiles.
#[tool("round_trip_float")]
async fn round_trip_float(x: f64) -> f64 {
    x + 1.0
}

/// Zero-arg tool returning Int — same shape as the narrow
/// bridge used to support directly. Preserves that capability.
#[tool("zero_arg_answer")]
async fn zero_arg_answer() -> i64 {
    42
}

// ---- Slice `35V2-P42-G0-tools-3b`: struct params/returns ----
//
// The receipt-returning tools in the reference apps
// (`ShareAnswerToChatReceipt`, the per-app `execute_approved_*`
// returns) have non-scalar signatures. The pre-3b macro aborted on
// these at `abi_type_for`. Post-3b, the macro detects a non-scalar
// arg or return and skips the typed C-ABI wrapper entirely, emitting
// only the JSON wrapper + inventory entry (with `symbol = ""` to
// signal "no direct-dispatch wrapper exists"). The tool is reachable
// only through the cdylib registry path that
// `35V2-P42-G0-tools-2b` made target-conditional.

/// A struct that round-trips through the JSON wrapper.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Receipt {
    pub label: String,
    pub delivered: bool,
    pub count: i64,
}

/// Struct return: scalar in, struct out. The receipt-shape that
/// drove this slice — every `execute_approved_*` flow ends in a
/// receipt return.
#[tool("emit_receipt")]
async fn emit_receipt(label: String) -> Receipt {
    Receipt {
        label,
        delivered: true,
        count: 1,
    }
}

/// Struct param: struct in, scalar out. The symmetric direction —
/// some `policy_*` helpers take a structured input.
#[tool("consume_receipt")]
async fn consume_receipt(r: Receipt) -> bool {
    r.delivered && r.count >= 1
}

/// Struct param AND struct return. Exercises both directions in
/// one macro expansion.
#[tool("amend_receipt")]
async fn amend_receipt(r: Receipt) -> Receipt {
    Receipt {
        label: r.label,
        delivered: r.delivered,
        count: r.count + 1,
    }
}

// ---- (2) The user's async fns remain callable as plain Rust ----

#[tokio::test]
async fn user_async_fn_still_callable_directly() {
    assert_eq!(double_it(7).await, 14);
    assert!(!flip(true).await);
    assert_eq!(round_trip_float(1.5).await, 2.5);
    assert_eq!(zero_arg_answer().await, 42);
}

// ---- (3) `inventory` sees every `#[tool]` metadata entry ----

#[test]
fn inventory_collects_every_tool() {
    let names: Vec<&'static str> = inventory::iter::<ToolMetadata>()
        .into_iter()
        .map(|m| m.name)
        .collect();

    for expected in ["double_it", "flip", "round_trip_float", "zero_arg_answer"] {
        assert!(
            names.contains(&expected),
            "inventory missing `{expected}`; saw {names:?}"
        );
    }
}

#[test]
fn metadata_arity_matches_declared_signature() {
    let by_name: std::collections::HashMap<&'static str, &'static ToolMetadata> =
        inventory::iter::<ToolMetadata>()
            .into_iter()
            .map(|m| (m.name, m))
            .collect();

    assert_eq!(by_name.get("double_it").unwrap().arity, 1);
    assert_eq!(by_name.get("flip").unwrap().arity, 1);
    assert_eq!(by_name.get("round_trip_float").unwrap().arity, 1);
    assert_eq!(by_name.get("zero_arg_answer").unwrap().arity, 0);
}

#[test]
fn metadata_symbol_follows_convention() {
    let by_name: std::collections::HashMap<&'static str, &'static ToolMetadata> =
        inventory::iter::<ToolMetadata>()
            .into_iter()
            .map(|m| (m.name, m))
            .collect();

    // The symbol is what Cranelift codegen will emit a direct call to —
    // stability across refactors matters. Locked convention here so a
    // future macro change that breaks it gets caught.
    assert_eq!(
        by_name.get("double_it").unwrap().symbol,
        "__corvid_tool_double_it"
    );
    assert_eq!(
        by_name.get("zero_arg_answer").unwrap().symbol,
        "__corvid_tool_zero_arg_answer"
    );
}

// ---- Slice `35V2-P42-G0-tools-3b` regression coverage ----

#[tokio::test]
async fn user_struct_signature_fns_still_callable_directly() {
    // The user's `async fn` is preserved unchanged regardless of
    // signature shape — the macro never blocks the Rust call site.
    let r = emit_receipt("hello".to_string()).await;
    assert_eq!(r.label, "hello");
    assert!(r.delivered);
    assert_eq!(r.count, 1);

    assert!(consume_receipt(r.clone()).await);
    let amended = amend_receipt(r).await;
    assert_eq!(amended.count, 2);
}

#[test]
fn struct_signature_tools_register_in_inventory_with_empty_symbol_marker() {
    // Three struct-signature tools shipped above; each must appear
    // in `inventory::iter` and each must carry the empty-string
    // `symbol` marker that signals "no typed wrapper exists; route
    // only through `json_dispatch`."
    let by_name: std::collections::HashMap<&'static str, &'static ToolMetadata> =
        inventory::iter::<ToolMetadata>()
            .into_iter()
            .map(|m| (m.name, m))
            .collect();

    for tool_name in ["emit_receipt", "consume_receipt", "amend_receipt"] {
        let meta = by_name.get(tool_name).unwrap_or_else(|| {
            panic!(
                "inventory missing struct-signature `{tool_name}`; saw {:?}",
                by_name.keys().collect::<Vec<_>>()
            )
        });
        assert_eq!(
            meta.symbol, "",
            "struct-signature `{tool_name}` MUST carry the empty-string `symbol` \
             marker; got `{}`. If this assertion fails, either (a) the macro \
             regressed back to emitting a typed wrapper for a non-scalar \
             signature (which would silently link an ill-typed direct-call \
             symbol), or (b) the `symbol` field's invariant changed and the \
             cdylib codegen must be updated to match. Read the `signature_is_all_scalar` \
             rule in `crates/corvid-macros/src/lib.rs` before touching this.",
            meta.symbol
        );
    }
}

#[test]
fn scalar_signature_tools_keep_typed_wrapper_symbol() {
    // Regression: the slice `G0-tools-3b` rewiring must NOT change
    // the scalar-tool symbol convention — codegen still direct-calls
    // `__corvid_tool_<name>` for scalar tools on native-binary
    // targets (`G0-tools-2b`'s target-conditional dispatch).
    let by_name: std::collections::HashMap<&'static str, &'static ToolMetadata> =
        inventory::iter::<ToolMetadata>()
            .into_iter()
            .map(|m| (m.name, m))
            .collect();

    for (tool_name, expected_symbol) in [
        ("double_it", "__corvid_tool_double_it"),
        ("flip", "__corvid_tool_flip"),
        ("round_trip_float", "__corvid_tool_round_trip_float"),
        ("zero_arg_answer", "__corvid_tool_zero_arg_answer"),
    ] {
        let meta = by_name.get(tool_name).unwrap();
        assert_eq!(
            meta.symbol, expected_symbol,
            "scalar-signature `{tool_name}` lost its typed wrapper symbol — \
             the slice `G0-tools-3b` rewiring regressed the scalar path."
        );
    }
}

#[test]
fn struct_signature_tools_carry_correct_arity() {
    // `emit_receipt(String) -> Receipt` is arity 1;
    // `consume_receipt(Receipt) -> bool` is arity 1;
    // `amend_receipt(Receipt) -> Receipt` is arity 1.
    // The macro's arity counter must work regardless of whether the
    // typed wrapper was emitted.
    let by_name: std::collections::HashMap<&'static str, &'static ToolMetadata> =
        inventory::iter::<ToolMetadata>()
            .into_iter()
            .map(|m| (m.name, m))
            .collect();

    assert_eq!(by_name.get("emit_receipt").unwrap().arity, 1);
    assert_eq!(by_name.get("consume_receipt").unwrap().arity, 1);
    assert_eq!(by_name.get("amend_receipt").unwrap().arity, 1);
}
