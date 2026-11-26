use super::llm_dispatch::{
    build_system_prompt, call_llm_once, call_llm_structured, format_instruction_bool, format_instruction_float,
    format_instruction_int, parse_bool, parse_float, parse_int, prompt_max_retries,
    trace_mock_llm_attempt, using_env_mock_llm,
};
use super::*;
use crate::llm::mock::{bench_mock_dispatch_ns, bench_prompt_wait_ns, env_mock_string_reply_sync};
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
//   4. After max retries, panic with a clear message including the
//      last LLM response — compiled binary aborts with stderr trail
//      so the user can see what went wrong.
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
#[no_mangle]
pub unsafe extern "C" fn corvid_prompt_call_int(
    prompt_name: CorvidString,
    signature: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> i64 {
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name_ref = unsafe { borrow_corvid_string(&prompt_name) };
    let rendered_ref = unsafe { borrow_corvid_string(&rendered) };
    let model_ref = unsafe { borrow_corvid_string(&model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };

    if using_env_mock_llm() {
        let state = bridge();
        let max_retries = prompt_max_retries();
        let mut last_response: Option<String> = None;
        for _attempt in 0..=max_retries {
            match env_mock_string_reply_sync(prompt_name_ref) {
                Some(reply) => {
                    let reply_text = unsafe { borrow_corvid_string(&reply) }.to_owned();
                    let parsed = parse_int(&reply_text);
                    if let Some(value) = parsed {
                        trace_mock_llm_attempt(
                            state,
                            prompt_name_ref,
                            model_ref,
                            rendered_ref,
                            &llm_args,
                            serde_json::Value::from(value),
                        );
                        unsafe { release_string(reply) };
                        record_json_bridge_ns(
                            bridge_start,
                            prompt_wait_before,
                            mock_dispatch_before,
                        );
                        return value;
                    }
                    trace_mock_llm_attempt(
                        state,
                        prompt_name_ref,
                        model_ref,
                        rendered_ref,
                        &llm_args,
                        serde_json::Value::String(reply_text.clone()),
                    );
                    unsafe { release_string(reply) };
                    last_response = Some(reply_text);
                }
                None => break,
            }
        }
        if last_response.is_some() {
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            panic!(
                "corvid prompt `{prompt_name_ref}`: could not parse Int from env-mock response after {} attempts. Last response: {:?}",
                max_retries + 1,
                last_response.as_deref().unwrap_or("(none)")
            );
        }
    }

    let state = bridge();
    let prompt_name = prompt_name_ref.to_owned();
    let signature = unsafe { read_corvid_string(signature) };
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };

    let max_retries = prompt_max_retries();
    let mut last_response: Option<String> = None;
    for attempt in 0..=max_retries {
        let sys = build_system_prompt(
            &signature,
            format_instruction_int(),
            attempt,
            last_response.as_deref(),
        );
        match call_llm_once(state, &prompt_name, &model, &rendered, &llm_args, &sys) {
            Err(e) => {
                if attempt == max_retries {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    panic!(
                        "corvid prompt `{prompt_name}` (model `{model}`): adapter failed after {} attempts: {e}",
                        attempt + 1
                    );
                }
                continue;
            }
            Ok(text) => match parse_int(&text) {
                Some(v) => {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    return v;
                }
                None => last_response = Some(text),
            },
        }
    }
    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
    panic!(
        "corvid prompt `{prompt_name}` (model `{model}`): could not parse Int from LLM response after {} attempts. Last response: {:?}",
        max_retries + 1,
        last_response.as_deref().unwrap_or("(none)")
    );
}

#[no_mangle]
pub unsafe extern "C" fn corvid_prompt_call_bool(
    prompt_name: CorvidString,
    signature: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> bool {
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name_ref = unsafe { borrow_corvid_string(&prompt_name) };
    let rendered_ref = unsafe { borrow_corvid_string(&rendered) };
    let model_ref = unsafe { borrow_corvid_string(&model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };

    if using_env_mock_llm() {
        let state = bridge();
        let max_retries = prompt_max_retries();
        let mut last_response: Option<String> = None;
        for _attempt in 0..=max_retries {
            match env_mock_string_reply_sync(prompt_name_ref) {
                Some(reply) => {
                    let reply_text = unsafe { borrow_corvid_string(&reply) }.to_owned();
                    let parsed = parse_bool(&reply_text);
                    if let Some(value) = parsed {
                        trace_mock_llm_attempt(
                            state,
                            prompt_name_ref,
                            model_ref,
                            rendered_ref,
                            &llm_args,
                            serde_json::Value::from(value),
                        );
                        unsafe { release_string(reply) };
                        record_json_bridge_ns(
                            bridge_start,
                            prompt_wait_before,
                            mock_dispatch_before,
                        );
                        return value;
                    }
                    trace_mock_llm_attempt(
                        state,
                        prompt_name_ref,
                        model_ref,
                        rendered_ref,
                        &llm_args,
                        serde_json::Value::String(reply_text.clone()),
                    );
                    unsafe { release_string(reply) };
                    last_response = Some(reply_text);
                }
                None => break,
            }
        }
        if last_response.is_some() {
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            panic!(
                "corvid prompt `{prompt_name_ref}`: could not parse Bool from env-mock response after {} attempts. Last: {:?}",
                max_retries + 1,
                last_response.as_deref().unwrap_or("(none)")
            );
        }
    }

    let state = bridge();
    let prompt_name = prompt_name_ref.to_owned();
    let signature = unsafe { read_corvid_string(signature) };
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };

    let max_retries = prompt_max_retries();
    let mut last_response: Option<String> = None;
    for attempt in 0..=max_retries {
        let sys = build_system_prompt(
            &signature,
            format_instruction_bool(),
            attempt,
            last_response.as_deref(),
        );
        match call_llm_once(state, &prompt_name, &model, &rendered, &llm_args, &sys) {
            Err(e) => {
                if attempt == max_retries {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    panic!(
                        "corvid prompt `{prompt_name}`: adapter failed after {} attempts: {e}",
                        attempt + 1
                    );
                }
                continue;
            }
            Ok(text) => match parse_bool(&text) {
                Some(v) => {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    return v;
                }
                None => last_response = Some(text),
            },
        }
    }
    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
    panic!(
        "corvid prompt `{prompt_name}`: could not parse Bool from LLM response after {} attempts. Last: {:?}",
        max_retries + 1,
        last_response.as_deref().unwrap_or("(none)")
    );
}

#[no_mangle]
pub unsafe extern "C" fn corvid_prompt_call_float(
    prompt_name: CorvidString,
    signature: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> f64 {
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name_ref = unsafe { borrow_corvid_string(&prompt_name) };
    let rendered_ref = unsafe { borrow_corvid_string(&rendered) };
    let model_ref = unsafe { borrow_corvid_string(&model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };

    if using_env_mock_llm() {
        let state = bridge();
        let max_retries = prompt_max_retries();
        let mut last_response: Option<String> = None;
        for _attempt in 0..=max_retries {
            match env_mock_string_reply_sync(prompt_name_ref) {
                Some(reply) => {
                    let reply_text = unsafe { borrow_corvid_string(&reply) }.to_owned();
                    let parsed = parse_float(&reply_text);
                    if let Some(value) = parsed {
                        trace_mock_llm_attempt(
                            state,
                            prompt_name_ref,
                            model_ref,
                            rendered_ref,
                            &llm_args,
                            serde_json::Number::from_f64(value)
                                .map(serde_json::Value::Number)
                                .unwrap_or_else(|| serde_json::Value::String(value.to_string())),
                        );
                        unsafe { release_string(reply) };
                        record_json_bridge_ns(
                            bridge_start,
                            prompt_wait_before,
                            mock_dispatch_before,
                        );
                        return value;
                    }
                    trace_mock_llm_attempt(
                        state,
                        prompt_name_ref,
                        model_ref,
                        rendered_ref,
                        &llm_args,
                        serde_json::Value::String(reply_text.clone()),
                    );
                    unsafe { release_string(reply) };
                    last_response = Some(reply_text);
                }
                None => break,
            }
        }
        if last_response.is_some() {
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            panic!(
                "corvid prompt `{prompt_name_ref}`: could not parse Float from env-mock response after {} attempts. Last: {:?}",
                max_retries + 1,
                last_response.as_deref().unwrap_or("(none)")
            );
        }
    }

    let state = bridge();
    let prompt_name = prompt_name_ref.to_owned();
    let signature = unsafe { read_corvid_string(signature) };
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };

    let max_retries = prompt_max_retries();
    let mut last_response: Option<String> = None;
    for attempt in 0..=max_retries {
        let sys = build_system_prompt(
            &signature,
            format_instruction_float(),
            attempt,
            last_response.as_deref(),
        );
        match call_llm_once(state, &prompt_name, &model, &rendered, &llm_args, &sys) {
            Err(e) => {
                if attempt == max_retries {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    panic!(
                        "corvid prompt `{prompt_name}`: adapter failed after {} attempts: {e}",
                        attempt + 1
                    );
                }
                continue;
            }
            Ok(text) => match parse_float(&text) {
                Some(v) => {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    return v;
                }
                None => last_response = Some(text),
            },
        }
    }
    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
    panic!(
        "corvid prompt `{prompt_name}`: could not parse Float from LLM response after {} attempts. Last: {:?}",
        max_retries + 1,
        last_response.as_deref().unwrap_or("(none)")
    );
}

#[no_mangle]
pub unsafe extern "C" fn corvid_prompt_call_string(
    prompt_name: CorvidString,
    signature: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) -> CorvidString {
    use crate::abi::IntoCorvidAbi;
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name_ref = unsafe { borrow_corvid_string(&prompt_name) };
    let rendered_ref = unsafe { borrow_corvid_string(&rendered) };
    let model_ref = unsafe { borrow_corvid_string(&model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };

    if using_env_mock_llm() {
        let state = bridge();
        if let Some(text) = env_mock_string_reply_sync(prompt_name_ref) {
            trace_mock_llm_attempt(
                state,
                prompt_name_ref,
                model_ref,
                rendered_ref,
                &llm_args,
                serde_json::Value::String(unsafe { borrow_corvid_string(&text) }.to_owned()),
            );
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            return text;
        }
    }

    let state = bridge();
    let prompt_name = prompt_name_ref.to_owned();
    let signature = unsafe { read_corvid_string(signature) };
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };

    // String return: no parse-retry loop. Whatever the LLM returns
    // IS the String. We still call once, and on adapter failure we
    // panic with a clear message — adapter errors are infrastructure
    // problems, not response-format problems.
    let sys = if using_env_mock_llm() {
        String::new()
    } else {
        format!(
            "You are a function with signature `{signature}`. Return the appropriate string value as your full response — no quotes around the value, no explanation, no formatting markers."
        )
    };
    match call_llm_once(state, &prompt_name, &model, &rendered, &llm_args, &sys) {
        Ok(text) => {
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            text.into_corvid_abi()
        }
        Err(e) => panic!("corvid prompt `{prompt_name}` (model `{model}`): adapter failed: {e}"),
    }
}


/// A native prompt whose reply is not a scalar (native scalar boundaries
/// B1b): a record, a list, an `Option`, a `Result`, a sum type or a
/// text-held scalar such as `Decimal`. The model is shown `schema_json`,
/// the public-form schema the interpreter shows it, and the reply is read
/// with the public decoder through the plan the compiler embedded, so
/// both tiers accept and refuse the same replies. A reply that does not
/// decode is retried with the reason; the decoded value is written to
/// `payload`.
///
/// # Safety
/// The strings must be live Corvid strings; `args_ptr` holds `argc`
/// trace slots tagged by `arg_types`; `plan`, `plan_len` and `typeinfos`
/// as for `corvid_json_decode_typed`; `payload` a writable slot.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn corvid_prompt_call_typed(
    prompt_name: CorvidString,
    // The scalar bridges' system-prompt signature; a structured request
    // carries the schema instead, as the interpreter's does.
    _signature: CorvidString,
    rendered: CorvidString,
    model: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
    schema_json: CorvidString,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const std::ffi::c_void,
    payload: *mut i64,
) -> i64 {
    let bridge_start = Instant::now();
    let prompt_wait_before = bench_prompt_wait_ns();
    let mock_dispatch_before = bench_mock_dispatch_ns();
    let prompt_name_ref = unsafe { borrow_corvid_string(&prompt_name) };
    let rendered_ref = unsafe { borrow_corvid_string(&rendered) };
    let model_ref = unsafe { borrow_corvid_string(&model) };
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let llm_args = unsafe { crate::native_trace::decode_trace_values(arg_tags, argc, args_ptr) };
    let decode = |text: &str| unsafe {
        crate::ffi_bridge::typed_json::native_value_from_model_reply(text, plan, plan_len, typeinfos)
    };
    let max_retries = prompt_max_retries();
    let mut last_failure: Option<String> = None;

    if using_env_mock_llm() {
        let state = bridge();
        for _attempt in 0..=max_retries {
            let Some(reply) = env_mock_string_reply_sync(prompt_name_ref) else { break };
            let reply_text = unsafe { borrow_corvid_string(&reply) }.to_owned();
            unsafe { release_string(reply) };
            // Recorded as the JSON it is, as the interpreter records a
            // structured reply; text that is not JSON is recorded as text.
            let reply_json = serde_json::from_str(reply_text.trim()).unwrap_or(serde_json::Value::String(reply_text.clone()));
            trace_mock_llm_attempt(state, prompt_name_ref, model_ref, rendered_ref, &llm_args, reply_json);
            match decode(&reply_text) {
                Ok(bits) => {
                    unsafe { payload.write(bits) };
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    return 0;
                }
                Err(reason) => last_failure = Some(format!("{reply_text} ({reason})")),
            }
        }
        if let Some(failure) = last_failure {
            record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
            panic!(
                "corvid prompt `{prompt_name_ref}`: the env-mock reply does not decode after {} attempts. Last: {failure}",
                max_retries + 1
            );
        }
    }

    // The request the interpreter makes: the rendered prompt and the
    // reply's schema as structured output.
    let state = bridge();
    let prompt_name = prompt_name_ref.to_owned();
    let rendered = unsafe { read_corvid_string(rendered) };
    let model = unsafe { read_corvid_string(model) };
    let schema_text = unsafe { read_corvid_string(schema_json) };
    let schema: serde_json::Value = serde_json::from_str(&schema_text)
        .unwrap_or_else(|err| panic!("internal compiler error: prompt `{prompt_name}` schema is not JSON: {err}"));
    for attempt in 0..=max_retries {
        match call_llm_structured(state, &prompt_name, &model, &rendered, &llm_args, &schema) {
            Err(e) => {
                if attempt == max_retries {
                    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                    panic!("corvid prompt `{prompt_name}`: adapter failed after {} attempts: {e}", attempt + 1);
                }
            }
            Ok(value) => {
                // A text adapter answers with the JSON as a string.
                let text = match value {
                    serde_json::Value::String(text) => text,
                    other => other.to_string(),
                };
                match decode(&text) {
                    Ok(bits) => {
                        unsafe { payload.write(bits) };
                        record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
                        return 0;
                    }
                    Err(reason) => last_failure = Some(format!("{text} ({reason})")),
                }
            }
        }
    }
    record_json_bridge_ns(bridge_start, prompt_wait_before, mock_dispatch_before);
    panic!(
        "corvid prompt `{prompt_name}`: the reply does not decode as the declared type after {} attempts. Last: {}",
        max_retries + 1,
        last_failure.as_deref().unwrap_or("(none)")
    );
}
