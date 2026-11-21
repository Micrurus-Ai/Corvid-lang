use super::{
    assert_no_leaks, assert_parity_bool_without_tools, compile_without_tools_lib, entry_name,
    ir_of, QueuedMockAdapter,
};
use corvid_runtime::{ProgrammaticApprover, Runtime};
use corvid_vm::Value;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

#[test]
fn append_grows_shared_lists_and_preserves_iteration_order() {
    assert_parity_bool_without_tools(r#"
agent main() -> Bool:
    words: List<String> = []
    alias = words
    word = "co" + "rvid"
    i = 0
    while i < 65:
        words.append(word)
        i = i + 1
    for item in words:
        alias.append(item)
    count = alias.length() == 130 and words.length() == 130
    contents = alias[64] == word and words[129] == word
    numbers: List<Float> = []
    numbers.append(3)
    numbers.append(1.5)
    flags = [false]
    flags.append(true)
    return count and contents and numbers[0] == 3.0 and numbers[1] == 1.5 and flags[1]
"#, true);
}

#[test]
fn string_map_assignment_evaluates_key_then_value_before_updating() {
    assert_parity_bool_without_tools(r#"
agent key(log: List<Int>) -> String:
    log.append(1)
    return "own" + "er"

agent replacement(names: Map<String, String>, log: List<Int>) -> String:
    log.append(2)
    names["extra"] = "inserted"
    return "new"

agent main() -> Bool:
    names = {"owner": "first", "owner": "second"}
    log: List<Int> = []
    names[key(log)] = replacement(names, log)
    same = "sa" + "me"
    names[same] = same
    stored = match names[same]:
        Some(value) -> value
        None -> ""
    updated = match names["owner"]:
        Some(value) -> value
        None -> ""
    ordered = log.length() == 2 and log[0] == 1 and log[1] == 2
    return ordered and stored == same and updated == "new"
"#, true);
}

#[test]
fn record_field_updates_and_nested_control_flow() {
    assert_nested_parity(include_str!("record_updates.cor"));
}

#[test]
fn typed_map_records_outlive_the_source_map() {
    assert_parity_bool_without_tools(include_str!("map_records.cor"), true);
}

#[test]
fn typed_map_nested_collections_and_options() {
    assert_nested_parity(include_str!("map_nested.cor"));
}

#[test]
fn integer_maps_preserve_zero_and_snapshot_values() {
    assert_nested_parity(include_str!("map_counters.cor"));
}

#[test]
fn string_map_iteration_and_membership_preserve_order() {
    assert_parity_bool_without_tools(include_str!("string_map_iteration.cor"), true);
}

#[test]
fn string_map_assignment_updates_alias_and_preserves_previous_value() {
    assert_parity_bool_without_tools(include_str!("string_map_assignment.cor"), true);
}

#[test]
fn string_map_brackets_borrow_keys_and_preserve_evaluation_order() {
    assert_parity_bool_without_tools(r#"
agent names(log: List<Int>) -> Map<String, String>:
    log.append(1)
    return {"id": "old", "blank": "", "id": "A" + "da"}

agent key(log: List<Int>) -> String:
    log.append(2)
    return "i" + "d"

agent read(table: Map<String, String>, id: String) -> String:
    return match table[id]:
        Some(value) -> value
        None -> "missing"

agent main() -> Bool:
    log: List<Int> = []
    saved = names(log)[key(log)]
    name = match saved:
        Some(value) -> value
        None -> "missing"
    order = log.length() == 2 and log[0] == 1 and log[1] == 2
    table = names(log)
    absent = "un" + "known"
    blank = read(table, "blank")
    missing = read(table, absent)
    return order and name == "Ada" and blank == "" and missing == "missing"
"#, true);
}

#[test]
fn string_map_lookup_preserves_values_and_absence() {
    assert_parity_bool_without_tools(include_str!("string_map.cor"), true);
}

#[test]
fn list_last_matches_owned_and_scalar_values() {
    assert_parity_bool_without_tools(include_str!("list_last.cor"), true);
}

#[test]
fn list_first_matches_owned_strings() {
    assert_parity_bool_without_tools(include_str!("first_label.cor"), true);
}

#[test]
fn list_first_distinguishes_missing_item_from_missing_inner_value() {
    assert_parity_bool_without_tools(r#"
agent value_or_zero(item: Option<Int>) -> Int:
    return match item:
        Some(n) -> n
        None -> 0

agent read(items: List<Option<Int>>) -> Int:
    return match items.first():
        None -> -1
        Some(item) -> value_or_zero(item)

agent main() -> Bool:
    missing: List<Option<Int>> = []
    item: Option<Int> = None
    missing.append(item)
    found: List<Option<Int>> = [Some(7)]
    empty_list = read([])
    empty_item = read(missing)
    present_item = read(found)
    return empty_list == -1 and empty_item == 0 and present_item == 7
"#, true);
}

#[test]
fn list_first_matches_wrapped_values() {
    assert_parity_bool_without_tools(r#"
agent integer(items: List<Int>) -> Int:
    return match items.first():
        Some(n) -> n + 1
        _ -> -1

agent flag(items: List<Bool>) -> Bool:
    return match items.first():
        None -> true
        Some(value) -> value

agent decimal(items: List<Float>) -> Float:
    return match items.first():
        Some(value) -> value
        None -> 1.0

agent main() -> Bool:
    ints = integer([0]) == 1 and integer([]) == -1
    flags = flag([]) and not flag([false])
    return ints and flags and decimal([0.0]) == 0.0
"#, true);
}

#[test]
fn nested_match_calls_transform_numbers_before_append() {
    assert_parity_bool_without_tools(r#"
agent normalize(number: Int) -> Int:
    if number < 0:
        return -number
    return number

agent record_number(numbers: List<Int>, number: Int) -> Nothing:
    numbers.append(number)

agent main() -> Bool:
    numbers: List<Int> = []
    errors: List<String> = []
    for text in ["-3", "invalid", "2"]:
        match text.parse_int():
            Ok(number) -> record_number(numbers, normalize(number))
            Err(_) -> errors.append(text)
    return numbers.length() == 2 and numbers[0] == 3 and numbers[1] == 2 and errors.length() == 1
"#, true);
}

#[test]
fn nested_match_calls_preserve_order_and_heap_values() {
    assert_nested_parity(include_str!("nested_match_calls.cor"));
}

fn assert_nested_parity(source: &'static str) {
    // Match the CLI stack budget when interpreting nested calls in debug builds.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || assert_parity_bool_without_tools(source, true))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn result_match_calls_return_values_and_update_captured_lists() {
    assert_parity_bool_without_tools(r#"
agent record(words: List<String>, first: String, second: String) -> String:
    words.append(first)
    return second

agent choose(input: Result<String, String>, words: List<String>, fallback: String) -> String:
    return match input:
        Err(reason) -> record(words, reason, reason)
        _ -> record(words, fallback, "ok")

agent main() -> Bool:
    words: List<String> = []
    fallback = "fall" + "back"
    failure = choose(Err("mis" + "sing"), words, fallback)
    success = choose(Ok("unused"), words, fallback)
    return words.length() == 2 and words[0] == failure and failure == "missing" and words[1] == fallback and success == "ok"
"#, true);
}

#[test]
fn match_appends_borrowed_string_payload_or_capture() {
    assert_parity_bool_without_tools(r#"
agent collect(value: Result<String, String>, words: List<String>, fallback: String) -> Nothing:
    match value:
        Err(reason) -> words.append(reason)
        _ -> words.append(fallback)

agent main() -> Bool:
    words: List<String> = []
    fallback = "fall" + "back"
    collect(Err("mis" + "sing"), words, fallback)
    collect(Ok("unused"), words, fallback)
    return words.length() == 2 and words[0] == "missing" and words[1] == fallback
"#, true);
}

#[test]
fn parses_collections_in_input_order() {
    assert_parity_bool_without_tools(include_str!("parse_numbers.cor"), true);
}

#[test]
fn string_parse_int_preserves_values_and_errors() {
    assert_parity_bool_without_tools(r#"
agent number(text: String) -> Int:
    return match text.parse_int():
        Ok(n) -> n
        Err(_) -> 0

agent error_message(text: String) -> String:
    return match text.parse_int():
        Ok(_) -> ""
        Err(message) -> message

agent main() -> Bool:
    spaced = number(" +41 ") == 41
    high = number("9223372036854775807") == 9223372036854775807
    low = number("-9223372036854775808") == -9223372036854775807 - 1
    invalid = " bad" + " input "
    exact_error = error_message(invalid) == "not an integer: ` bad input `"
    overflow = error_message("9223372036854775808") == "not an integer: `9223372036854775808`"
    underflow = error_message("-9223372036854775809") == "not an integer: `-9223372036854775809`"
    empty = error_message("") == "not an integer: ``"
    temporary = match ("-" + "12").parse_int():
        Ok(n) -> n
        Err(_) -> 0
    bounds = spaced and high and low
    errors = exact_error and overflow and underflow and empty
    return bounds and errors and temporary == -12
"#, true);
}

#[test]
fn inline_result_arguments_infer_both_variants() {
    assert_parity_bool_without_tools(r#"
agent next_count(value: Result<Int, String>) -> Int:
    return match value:
        Ok(count) -> count + 1
        Err(_) -> 0

agent scale(value: Result<Bool, Float>) -> Float:
    return match value:
        Err(amount) -> amount * 0.5
        Ok(_) -> 0.0

agent main() -> Bool:
    count = next_count(Ok(41))
    fallback = next_count(Err("missing"))
    scaled = scale(Err(5))
    return count == 42 and fallback == 0 and scaled == 2.5
"#, true);
}

#[test]
fn result_match_extracts_selected_payload_or_default() {
    assert_parity_bool_without_tools(r#"
agent choose_value(good: Bool) -> Result<String, String>:
    if good:
        return Ok("ready")
    return Err("missing")

agent extract(value: Result<String, String>) -> String:
    return match value:
        Err(message) -> message
        Ok(text) -> text

agent main() -> Bool:
    value = choose_value(true)
    first = extract(value)
    second = match value:
        Ok(text) -> text
        _ -> "fallback"
    fallback = match choose_value(false):
        Ok(text) -> text
        Err(_) -> "fallback"
    return first == "ready" and second == "ready" and fallback == "fallback" and extract(choose_value(false)) == "missing"
"#, true);
}

#[test]
fn result_match_computes_with_scalar_payloads() {
    assert_parity_bool_without_tools(r#"
agent quantity(valid: Bool) -> Result<Int, Float>:
    if valid:
        return Ok(4)
    return Err(2.5)

agent score(value: Result<Int, Float>, offset: Int) -> Float:
    return match value:
        Err(scale) -> scale * 2.0
        Ok(count) -> (count + offset) * 0.5

agent increment(value: Result<Int, Float>) -> Int:
    return match value:
        Ok(count) -> count + 1
        _ -> 0

agent main() -> Bool:
    good = quantity(true)
    bad = quantity(false)
    safe = match good:
        Ok(n) -> n > 0 or 1 / (n - n) > 0
        _ -> false
    increments = increment(good) == 5 and increment(bad) == 0
    scores = score(good, 2) == 3.0 and score(bad, 2) == 5.0
    return safe and increments and scores
"#, true);
}

#[track_caller]
fn assert_parity_bool_with_mock_llm_queue(
    src: &str,
    prompt_name: &str,
    replies: Vec<serde_json::Value>,
    expected: bool,
    model: &str,
) {
    let ir = ir_of(src);

    let mut queued = HashMap::new();
    queued.insert(prompt_name.to_string(), replies.clone().into());
    let runtime = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .llm(Arc::new(QueuedMockAdapter::new(model, queued)))
        .default_model(model)
        .build();
    let interp_value = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { corvid_vm::run_agent(&ir, entry_name(&ir), vec![], &runtime).await })
        .expect("interpreter run");
    assert_eq!(
        interp_value,
        Value::Bool(expected),
        "interpreter result mismatch for src:\n{src}"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_path = tmp.path().join("prog");
    let produced = compile_without_tools_lib(&ir, &bin_path);
    let output = Command::new(&produced)
        .env("CORVID_DEBUG_ALLOC", "1")
        .env("CORVID_APPROVE_AUTO", "1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env(
            "CORVID_TEST_MOCK_LLM_REPLIES",
            serde_json::json!({ prompt_name: replies }).to_string(),
        )
        .env("CORVID_MODEL", model)
        .output()
        .expect("run compiled binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "compiled binary exited non-zero: status={:?} stdout={stdout} stderr={stderr} src=\n{src}",
        output.status.code()
    );
    let printed = stdout.trim().lines().next().unwrap_or("");
    let compiled_bool = match printed {
        "true" | "1" => true,
        "false" | "0" => false,
        other => panic!(
            "expected `true` / `false` / `1` / `0` for Bool, got `{other}`; src=\n{src}"
        ),
    };
    assert_eq!(
        compiled_bool, expected,
        "compiled result mismatch for src:\n{src}\nstderr: {stderr}"
    );
    assert_no_leaks(&stderr, src);
}



#[test]
fn nullable_option_string_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<String>:\n    if flag:\n        return Some(\"hi\")\n    return None\n\nagent main() -> Bool:\n    value = maybe(true)\n    return value != None\n",
        true,
    );
}

#[test]
fn nullable_option_string_none_compares_equal_to_none() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<String>:\n    if flag:\n        return Some(\"hi\")\n    return None\n\nagent main() -> Bool:\n    value = maybe(false)\n    return value == None\n",
        true,
    );
}

#[test]
fn nullable_option_string_try_propagates_some_and_none() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<String>:\n    if flag:\n        return Some(\"hi\")\n    return None\n\nagent unwrap(flag: Bool) -> Option<String>:\n    value = maybe(flag)?\n    return Some(value)\n\nagent main() -> Bool:\n    return unwrap(false) == None and unwrap(true) != None\n",
        true,
    );
}

#[test]
fn wide_option_int_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<Int>:\n    if flag:\n        return Some(7)\n    return None\n\nagent main() -> Bool:\n    value = maybe(true)\n    return value != None\n",
        true,
    );
}

#[test]
fn wide_option_int_try_propagates_some_and_none() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<Int>:\n    if flag:\n        return Some(7)\n    return None\n\nagent unwrap(flag: Bool) -> Option<Int>:\n    value = maybe(flag)?\n    return Some(value + 1)\n\nagent main() -> Bool:\n    return unwrap(false) == None and unwrap(true) != None\n",
        true,
    );
}

#[test]
fn wide_option_int_try_propagates_into_different_outer_option_type() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<Int>:\n    if flag:\n        return Some(7)\n    return None\n\nagent widen(flag: Bool) -> Option<Bool>:\n    value = maybe(flag)?\n    return Some(value > 0)\n\nagent main() -> Bool:\n    return widen(false) == None and widen(true) != None\n",
        true,
    );
}

#[test]
fn nullable_option_string_try_propagates_into_wide_outer_option_type() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<String>:\n    if flag:\n        return Some(\"hi\")\n    return None\n\nagent widen(flag: Bool) -> Option<Bool>:\n    value = maybe(flag)?\n    return Some(value == \"hi\")\n\nagent main() -> Bool:\n    return widen(false) == None and widen(true) != None\n",
        true,
    );
}

#[test]
fn native_option_retry_retries_until_some() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Option<Int>:\n    value = probe()\n    if value == \"ok\":\n        return Some(7)\n    return None\n\nagent main() -> Bool:\n    outcome = try fetch() on error retry 3 times backoff linear 0\n    return probe() == \"marker\"\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("ok"),
            serde_json::json!("marker"),
        ],
        true,
        "mock-1",
    );
}

#[test]
fn native_option_retry_returns_none_after_exhausting_attempts() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Option<Int>:\n    value = probe()\n    if value == \"ok\":\n        return Some(7)\n    return None\n\nagent main() -> Bool:\n    outcome = try fetch() on error retry 3 times backoff exponential 0\n    return outcome == None and probe() == \"marker\"\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("marker"),
        ],
        true,
        "mock-1",
    );
}

#[test]
fn native_nested_option_int_distinguishes_none_from_some_none() {
    assert_parity_bool_without_tools(
        "agent fetch(mode: Int) -> Option<Option<Int>>:\n    if mode == 0:\n        return None\n    if mode == 1:\n        return Some(None)\n    return Some(Some(7))\n\nagent main() -> Bool:\n    first = fetch(0)\n    second = fetch(1)\n    third = fetch(2)\n    return first == None and second != None and third != None\n",
        true,
    );
}

#[test]
fn native_nested_option_try_propagates_outer_none_and_preserves_inner_option() {
    assert_parity_bool_without_tools(
        "agent fetch(mode: Int) -> Option<Option<Int>>:\n    if mode == 0:\n        return None\n    if mode == 1:\n        return Some(None)\n    return Some(Some(7))\n\nagent inspect(mode: Int) -> Option<Bool>:\n    value = fetch(mode)?\n    return Some(value == None or value != None)\n\nagent main() -> Bool:\n    return inspect(0) == None and inspect(1) != None and inspect(2) != None\n",
        true,
    );
}

#[test]
fn wide_option_bool_none_compares_equal_to_none() {
    assert_parity_bool_without_tools(
        "agent maybe(flag: Bool) -> Option<Bool>:\n    if flag:\n        return Some(true)\n    return None\n\nagent main() -> Bool:\n    return maybe(false) == None and maybe(true) != None\n",
        true,
    );
}

#[test]
fn native_result_string_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<String, String>:\n    if flag:\n        return Ok(\"hi\")\n    return Err(\"no\")\n\nagent main() -> Bool:\n    first = fetch(true)\n    second = fetch(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_string_try_propagates_ok_and_err() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<String, String>:\n    if flag:\n        return Ok(\"hi\")\n    return Err(\"no\")\n\nagent forward(flag: Bool) -> Result<String, String>:\n    value = fetch(flag)?\n    return Ok(value)\n\nagent main() -> Bool:\n    first = forward(true)\n    second = forward(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_string_try_propagates_into_different_ok_type() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<String, String>:\n    if flag:\n        return Ok(\"hi\")\n    return Err(\"no\")\n\nagent widen(flag: Bool) -> Result<Bool, String>:\n    value = fetch(flag)?\n    return Ok(true)\n\nagent main() -> Bool:\n    first = widen(true)\n    second = widen(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_retry_retries_until_success() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Result<String, String>:\n    value = probe()\n    if value == \"ok\":\n        return Ok(value)\n    return Err(value)\n\nagent main() -> Bool:\n    outcome = try fetch() on error retry 3 times backoff linear 0\n    return probe() == \"marker\"\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("ok"),
            serde_json::json!("marker"),
        ],
        true,
        "mock-1",
    );
}

#[test]
fn native_result_retry_returns_last_error_value_without_propagating() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Result<String, String>:\n    value = probe()\n    if value == \"ok\":\n        return Ok(value)\n    return Err(value)\n\nagent main() -> Bool:\n    outcome = try fetch() on error retry 3 times backoff exponential 0\n    return probe() == \"marker\"\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("marker"),
        ],
        true,
        "mock-1",
    );
}

#[test]
fn native_result_option_int_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<Option<Int>, String>:\n    if flag:\n        return Ok(Some(7))\n    return Err(\"no\")\n\nagent main() -> Bool:\n    first = fetch(true)\n    second = fetch(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_option_int_try_propagates_ok_and_err() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<Option<Int>, String>:\n    if flag:\n        return Ok(Some(7))\n    return Err(\"no\")\n\nagent forward(flag: Bool) -> Result<Option<Int>, String>:\n    value = fetch(flag)?\n    return Ok(value)\n\nagent main() -> Bool:\n    first = forward(true)\n    second = forward(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_option_int_retry_retries_until_success() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Result<Option<Int>, String>:\n    value = probe()\n    if value == \"ok\":\n        return Ok(Some(7))\n    return Err(value)\n\nagent main() -> Bool:\n    outcome = try fetch() on error retry 3 times backoff linear 0\n    return probe() == \"marker\"\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("ok"),
            serde_json::json!("marker"),
        ],
        true,
        "mock-1",
    );
}

#[test]
fn native_result_struct_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "type Boxed:\n    value: Int\n\nagent fetch(flag: Bool) -> Result<Boxed, String>:\n    if flag:\n        return Ok(Boxed(7))\n    return Err(\"no\")\n\nagent main() -> Bool:\n    first = fetch(true)\n    second = fetch(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_struct_try_propagates_ok_and_err() {
    assert_parity_bool_without_tools(
        "type Boxed:\n    value: Int\n\nagent fetch(flag: Bool) -> Result<Boxed, String>:\n    if flag:\n        return Ok(Boxed(7))\n    return Err(\"no\")\n\nagent forward(flag: Bool) -> Result<Boxed, String>:\n    value = fetch(flag)?\n    return Ok(value)\n\nagent main() -> Bool:\n    first = forward(true)\n    second = forward(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_list_int_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<List<Int>, String>:\n    if flag:\n        return Ok([1, 2, 3])\n    return Err(\"no\")\n\nagent main() -> Bool:\n    first = fetch(true)\n    second = fetch(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_list_int_try_propagates_ok_and_err() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<List<Int>, String>:\n    if flag:\n        return Ok([1, 2, 3])\n    return Err(\"no\")\n\nagent forward(flag: Bool) -> Result<List<Int>, String>:\n    value = fetch(flag)?\n    return Ok(value)\n\nagent main() -> Bool:\n    first = forward(true)\n    second = forward(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_nested_ok_round_trips_through_native_agents() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<Result<Int, String>, String>:\n    if flag:\n        return Ok(Ok(7))\n    return Err(\"no\")\n\nagent main() -> Bool:\n    first = fetch(true)\n    second = fetch(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_nested_ok_try_propagates_ok_and_err() {
    assert_parity_bool_without_tools(
        "agent fetch(flag: Bool) -> Result<Result<Int, String>, String>:\n    if flag:\n        return Ok(Ok(7))\n    return Err(\"no\")\n\nagent forward(flag: Bool) -> Result<Result<Int, String>, String>:\n    value = fetch(flag)?\n    return Ok(value)\n\nagent main() -> Bool:\n    first = forward(true)\n    second = forward(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_nested_error_try_propagates_into_different_ok_type() {
    assert_parity_bool_without_tools(
        "agent inner_error() -> Result<String, Bool>:\n    return Err(false)\n\nagent fetch(flag: Bool) -> Result<Int, Result<String, Bool>>:\n    if flag:\n        return Ok(7)\n    return Err(inner_error())\n\nagent widen(flag: Bool) -> Result<Bool, Result<String, Bool>>:\n    value = fetch(flag)?\n    return Ok(value == 7)\n\nagent main() -> Bool:\n    first = widen(true)\n    second = widen(false)\n    return true\n",
        true,
    );
}

#[test]
fn native_result_retry_then_try_propagates_into_different_ok_type() {
    assert_parity_bool_with_mock_llm_queue(
        "prompt probe() -> String:\n    \"Probe\"\n\nagent fetch() -> Result<String, String>:\n    value = probe()\n    if value == \"ok\":\n        return Ok(value)\n    return Err(value)\n\nagent widen() -> Result<Bool, String>:\n    attempt = try fetch() on error retry 3 times backoff linear 0\n    value = attempt?\n    return Ok(value == \"ok\")\n\nagent main() -> Bool:\n    first = widen()\n    return true\n",
        "probe",
        vec![
            serde_json::json!("bad"),
            serde_json::json!("bad"),
            serde_json::json!("ok"),
        ],
        true,
        "mock-1",
    );
}






#[test]
fn list_updates_evaluate_each_operand_once() {
    assert_nested_parity(include_str!("list_updates.cor"));
}

#[test]
fn match_arithmetic_evaluates_calls_in_the_selected_arm() {
    assert_nested_parity(include_str!("match_arithmetic.cor"));
}

#[test]
fn guarded_patterns_preserve_order_and_propagated_errors() {
    assert_nested_parity(include_str!("guarded_patterns.cor"));
}
