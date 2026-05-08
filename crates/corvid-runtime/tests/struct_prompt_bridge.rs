//! Integration tests for `corvid_prompt_call_struct`.
//!
//! Lives in a separate test binary because the env-mock LLM
//! infrastructure (`CORVID_TEST_MOCK_LLM`, `CORVID_TEST_MOCK_LLM_REPLIES`)
//! reads its env vars exactly once into a `OnceLock`-backed cache.
//! Setting the vars before any other test pokes the runtime would
//! work, but cargo runs tests in parallel and we cannot rely on
//! test ordering. A separate integration binary guarantees the
//! fixture state is isolated per-process — env vars are read before
//! any other test in this binary touches the runtime, and the three
//! scenarios share that single fixture state by drawing replies
//! from a queue keyed on prompt name.
//!
//! All three scenarios run inside one `#[test] fn` so the env-mock
//! reply queues, the decoder behaviour-mode counter, and the
//! runtime initialisation share a single deterministic ordering.
//! Splitting them into separate `#[test] fn`s would let cargo
//! parallelise within the binary and race on the static counters.

use corvid_runtime::abi::CorvidString;
use corvid_runtime::ffi_bridge::{
    corvid_prompt_call_struct, corvid_runtime_embed_init_default, string_from_rust,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn cs(s: &str) -> CorvidString {
    string_from_rust(s.to_owned())
}

// Scenario discriminator. The mock decoder consults this to decide
// which behaviour mode to perform.
//   0 = decoder always succeeds (returns 42)
//   1 = decoder fails twice then succeeds (returns 99 on the third call)
//
// The "decoder always fails → bridge panics" scenario is not
// unit-testable through the C ABI: all four `corvid_prompt_call_*`
// bridges use `extern "C"` (not `extern "C-unwind"`) for stable
// codegen ABI compatibility, and a Rust panic crossing an
// `extern "C"` boundary aborts the process rather than unwinding.
// `std::panic::catch_unwind` cannot catch it. The end-to-end
// integration test in commit 4 exercises the panic path naturally
// when a misconfigured mock causes the bridge to exhaust its
// retries — there the abort terminates the compiled binary with
// the canonical "could not decode Struct" message on stderr,
// which is exactly the behaviour users observe at runtime.
static SCENARIO: AtomicUsize = AtomicUsize::new(0);
static DECODER_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn mock_decoder(_text: CorvidString) -> i64 {
    // The bridge passes the LLM response wrapped in a fresh
    // CorvidString (refcount 1) and releases it after the decoder
    // returns, per the +0 ABI. The decoder borrows; it does not
    // need to release. We don't introspect the text in this test —
    // the call-count and return-value invariants below pin all the
    // bridge semantics that matter (which decoder slot is invoked,
    // how many retry attempts happen, what the bridge returns on
    // success).
    let count = DECODER_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    match SCENARIO.load(Ordering::SeqCst) {
        0 => 42,
        _ => {
            if count >= 2 {
                99
            } else {
                0
            }
        }
    }
}

#[test]
fn struct_bridge_drives_decoder_through_retry_loop() {
    // SAFETY of env::set_var: this happens before any other thread
    // is spawned. The runtime hasn't been initialised yet, so no
    // worker thread is reading env vars concurrently. The integration
    // test runs as a fresh process, so this is the first env-var
    // mutation in this binary.
    unsafe {
        std::env::set_var("CORVID_TEST_MOCK_LLM", "1");
        // Three prompt names, each with enough replies to cover the
        // worst-case retry sequence. CORVID_PROMPT_MAX_RETRIES default
        // is 3, so 4 attempts are possible. We queue 4 replies for
        // every prompt; unused replies are harmless.
        std::env::set_var(
            "CORVID_TEST_MOCK_LLM_REPLIES",
            r#"{
                "always_succeeds":  ["{\"k\":1}", "{\"k\":2}", "{\"k\":3}", "{\"k\":4}"],
                "fails_twice":      ["{\"k\":1}", "{\"k\":2}", "{\"k\":3}", "{\"k\":4}"]
            }"#,
        );
    }

    // Idempotent — returns 0 if the runtime is already initialised
    // (some other test in this binary may have done so first; here
    // we're the only test, so this call is the actual init).
    corvid_runtime_embed_init_default();

    // ---- Scenario 0: decoder succeeds on first attempt -------------
    SCENARIO.store(0, Ordering::SeqCst);
    DECODER_CALL_COUNT.store(0, Ordering::SeqCst);
    let result = unsafe {
        corvid_prompt_call_struct(
            cs("always_succeeds"),
            cs("always_succeeds() -> Decision"),
            cs("rendered body"),
            cs(""),
            cs(""),
            0,
            0,
            cs(r#"{"type":"object"}"#),
            mock_decoder,
        )
    };
    assert_eq!(result, 42, "decoder success path returns the decoder's ptr");
    assert_eq!(
        DECODER_CALL_COUNT.load(Ordering::SeqCst),
        1,
        "decoder called exactly once on first-attempt success"
    );

    // ---- Scenario 1: decoder fails twice then succeeds -------------
    SCENARIO.store(1, Ordering::SeqCst);
    DECODER_CALL_COUNT.store(0, Ordering::SeqCst);
    let result = unsafe {
        corvid_prompt_call_struct(
            cs("fails_twice"),
            cs("fails_twice() -> Decision"),
            cs("rendered body"),
            cs(""),
            cs(""),
            0,
            0,
            cs(r#"{"type":"object"}"#),
            mock_decoder,
        )
    };
    assert_eq!(result, 99, "retry path returns the eventually-decoded ptr");
    assert_eq!(
        DECODER_CALL_COUNT.load(Ordering::SeqCst),
        3,
        "decoder called 3 times: 2 failures + 1 success"
    );

}
