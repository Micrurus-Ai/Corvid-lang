use super::llm_dispatch::call_llm_structured;
use super::*;
use crate::llm::mock::{bench_mock_dispatch_ns, bench_prompt_wait_ns};
use std::time::Instant;

// Typed prompt-dispatch bridges.
//
// One bridge per return type, mirroring the typed-ABI tool design.
// Each takes 4 CorvidString args (prompt name, signature string,
// rendered prompt body, model name) and returns the typed value.
//
// All four bridges follow the same shape internally:
//   1. Read CorvidString args as Rust Strings (borrow, no refcount poke).
//   2. Build a system prompt: function-signature context +
//      return-type-specific format instruction.
//   3. Loop up to CORVID_PROMPT_MAX_RETRIES (default 3):
//      a. Call the adapter via block_on.
//      b. Parse the response into the typed value.
//      c. On parse success, return.
//      d. On parse failure, capture last response for next retry's
//         stronger system prompt.
//   4. After max retries, stop with a clear message including the
//      last LLM response, or hold the outage when the program has a
//      `try` that catches it (recoverable failures TE-3c).
//
// String returns skip the parse-retry loop entirely (a String response
// is by definition parseable as String). The shape stays uniform so
// codegen has the same call pattern for every return type.
//
// Function-signature context is the inventive piece: the system
// prompt explicitly tells the LLM "you are a function with signature
// X — return the appropriate value." Codegen knows the signature at
// compile time and embeds it as a literal. Same prompt body, much
// better LLM behavior because the model has the type contract.
// ------------------------------------------------------------
/// A native prompt call (native scalar boundaries B1b, and since the
/// scalar-reply slice every prompt): the request the interpreter makes,
/// the rendered prompt and `reply_schema`'s schema (none, for a `String`
/// reply), and the reply read with the public decoder through the plan the
/// compiler embedded. A reply that does not decode is retried exactly as
/// `with repair N` says, with the interpreter's retry text
/// (`corvid_runtime::prompt_repair_rendered`), so both tiers make the same
/// calls and a recording replays on either. The decoded value is written
/// to `payload`; a reply still unusable after the repairs stops the
/// program with the reason.
///
/// # Safety
/// The strings must be live Corvid strings; `args_ptr` holds `argc`
/// trace slots tagged by `arg_types`; `plan`, `plan_len` and `typeinfos`
/// as for `corvid_json_decode_typed`; `payload` a writable slot.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn corvid_prompt_call_typed(
    prompt_name: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
    schema_json: CorvidString,
    repair_attempts: i64,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const std::ffi::c_void,
    payload: *mut i64,
) -> i64 {
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name = unsafe { borrow_corvid_string(&prompt_name) }.to_owned();
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };
    let schema_text = unsafe { read_corvid_string(schema_json) };
    let schema: Option<serde_json::Value> = (!schema_text.is_empty()).then(|| {
        serde_json::from_str(&schema_text)
            .unwrap_or_else(|err| panic!("internal compiler error: prompt `{prompt_name}` schema is not JSON: {err}"))
    });
    let decode = |reply: serde_json::Value| unsafe {
        crate::ffi_bridge::typed_json::native_value_from_model_reply(reply, plan, plan_len, typeinfos)
            .map_err(|reason| format!("prompt `{prompt_name}`: {reason}"))
    };
    let state = bridge();
    let mut attempt_rendered = rendered.clone();
    let mut last_failure = String::new();
    for _attempt in 0..=repair_attempts.max(0) {
        let reply = match call_llm_structured(state, &prompt_name, &model, &attempt_rendered, &llm_args, schema.as_ref()) {
            Ok(reply) => reply,
            Err(e) => {
                record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                return fail(format!("corvid: prompt `{prompt_name}`: {e}"), e.is_outage(), payload);
            }
        };
        match decode(reply.clone()) {
            Ok(bits) => {
                unsafe { payload.write(bits) };
                record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                return 0;
            }
            Err(feedback) => {
                attempt_rendered = crate::prompt_repair_rendered(&rendered, &reply, &feedback);
                last_failure = feedback;
            }
        }
    }
    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
    // A reply that never fits its type is an unusable reply: an outage
    // on both tiers.
    fail(format!("corvid: unusable reply: {last_failure}"), true, payload)
}

/// A failed prompt call: stop the program with `message`, as the
/// interpreter stops with its error, or hold an outage the program can
/// catch and leave zero bits (recoverable failures TE-3c). A panic
/// cannot unwind out of this `extern "C"` bridge.
fn fail(message: String, outage: bool, payload: *mut i64) -> i64 {
    super::outage::fail(message, outage);
    unsafe { payload.write(0) };
    0
}
