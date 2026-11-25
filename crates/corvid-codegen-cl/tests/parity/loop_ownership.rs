//! Ownership of refcounted locals around rebinding, branches and
//! loops on the native tier. Every program runs on the interpreter and
//! as a compiled binary; both must return the same text, and the
//! binary must release every allocation exactly once (the harness
//! fails on a leak, and the runtime aborts on a double free).
//!
//! The shapes come from reading a webhook signature header: a loop
//! over `header.split(",")` that rebinds a string inside an `if`, and
//! one that returns from inside the loop.

use super::scalar_harness::assert_parity_text;

#[test]
fn a_string_rebound_inside_an_if_inside_a_for_loop_is_released_once() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    header = "a=1,t=1732480000,v1=abc"
    timestamp = ""
    for part in header.split(","):
        if part.starts_with("t="):
            timestamp = part.substring(2, part.length())
    return timestamp
"#,
    );
    assert_eq!(out, "1732480000");
}

#[test]
fn a_string_rebound_after_its_last_read_is_released_once() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    s = "first" + "!"
    n = s.length()
    s = "second" + "?"
    s = "third" + "."
    unused = "never" + "read"
    unused = "read" + "never"
    if n == 6:
        return s
    return "wrong length"
"#,
    );
    assert_eq!(out, "third.");
}

#[test]
fn a_value_rebound_from_itself_stays_alive_while_the_new_value_is_built() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    s = "  padded  " + "!"
    s = s.trim()
    t = "q" + "r"
    t = t + "z"
    for x in ["a", "b", "c"]:
        t = t + x
        if x == "b":
            t = t.to_upper()
    return s + "|" + t
"#,
    );
    assert_eq!(out, "padded  !|QRZABc");
}

#[test]
fn a_parameter_rebound_in_the_callee_is_released_once() {
    let out = assert_parity_text(
        r#"
agent with_question_mark(s: String) -> String:
    s = s + "?"
    return s

agent replace(s: String) -> Int:
    s = "fresh" + "value"
    return s.length()

agent main() -> String:
    a = "q" + "r"
    n = replace(a)
    if n == 10:
        return with_question_mark(a) + a
    return "wrong length"
"#,
    );
    assert_eq!(out, "qr?qr");
}

#[test]
fn returning_from_inside_a_for_loop_releases_the_string_it_splits() {
    let out = assert_parity_text(
        r#"
agent first_field(seed: String) -> String:
    header = seed + ",t=9"
    for part in header.split(","):
        return part
    return ""

agent main() -> String:
    return first_field("a=1")
"#,
    );
    assert_eq!(out, "a=1");
}

#[test]
fn returning_from_inside_a_for_loop_releases_locals_the_loop_exit_still_needs() {
    let out = assert_parity_text(
        r#"
agent first_or_prefix(seed: String, header: String) -> String:
    prefix = seed + "="
    for part in header.split(","):
        return part
    return prefix

agent main() -> String:
    return first_or_prefix("t", "a=1,t=2") + "|" + first_or_prefix("t", "")
"#,
    );
    assert_eq!(out, "a=1|");
}

#[test]
fn returning_from_a_branch_after_the_condition_last_reads_the_loop_item_releases_it() {
    let out = assert_parity_text(
        r#"
agent has_signature(header: String) -> Bool:
    for part in header.split(","):
        if part.starts_with("v1="):
            return true
    return false

agent main() -> String:
    if has_signature("t=123,v1=abc,v1=def"):
        if has_signature("t=123"):
            return "both"
        return "first"
    return "none"
"#,
    );
    assert_eq!(out, "first");
}

#[test]
fn records_lists_and_byte_strings_rebound_inside_a_loop_are_released_once() {
    let out = assert_parity_text(
        r#"
type Entry:
    name: String
    count: Int

agent main() -> String:
    best = Entry("none", 0)
    tags = ["start"]
    mark = "".utf8()
    for word in "a,bb,ccc,dd".split(","):
        if word.length() > best.count:
            best = Entry(word, word.length())
            tags = [word, "long"]
            mark = word.utf8()
    return best.name + " " + tags[0] + " " + mark.to_hex()
"#,
    );
    assert_eq!(out, "ccc ccc 636363");
}

#[test]
fn records_lists_and_byte_strings_live_across_a_loop_are_released_on_an_early_return() {
    let out = assert_parity_text(
        r#"
type Entry:
    name: String
    count: Int

agent find(words: List<String>) -> String:
    best = Entry("none", 0)
    tags = ["start"]
    mark = "x".utf8()
    for word in words:
        if word.length() > 2:
            return word
    return best.name + tags[0] + mark.to_hex()

agent main() -> String:
    return find(["a", "bb", "ccc"]) + "|" + find(["a"])
"#,
    );
    assert_eq!(out, "ccc|nonestart78");
}

#[test]
fn a_list_rebound_in_a_nested_loop_and_a_return_from_the_inner_loop_balance() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    seen = ["x"]
    for word in "a,b".split(","):
        for letter in ["p", "q"]:
            if letter == "q":
                seen = [word + letter]
            if word == "b":
                return seen[0] + letter
    return seen[0]
"#,
    );
    assert_eq!(out, "aqp");
}
