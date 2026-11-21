//! Regression test for 20l-A: `corvid check` resolves imports.
//!
//! Before 20l-A, `cmd_check` called the path-less driver entry
//! `compile_with_config`, so a missing or misnamed import was not
//! caught — `corvid check` reported "ok" on programs whose `corvid
//! build` would fail. Editor / pre-commit / LSP integrations all
//! reported false positives.
//!
//! This test reproduces the reporter's exact case: a file whose
//! single import does not resolve. `corvid check` must reject it
//! with a "could not be found" diagnostic and a non-zero exit code.

use std::process::Command;
use tempfile::tempdir;

#[test]
fn native_run_rebuilds_after_direct_and_transitive_import_edits() {
    let tmp = tempdir().unwrap();
    let main = tmp.path().join("main.cor");
    let helper = tmp.path().join("helper.cor");
    let math = tmp.path().join("math.cor");
    std::fs::write(&main, "import \"./helper\" use compute\nagent main() -> Int:\n    return compute(4)\n").unwrap();
    let original = "import \"./math\" use scale\npublic fn compute(n: Int) -> Int:\n    return scale(n) + 1\n";
    std::fs::write(&helper, original).unwrap();
    std::fs::write(&math, "public fn scale(n: Int) -> Int:\n    return n * 2\n").unwrap();
    let run = |expected: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .args(["run", "--target", "native"])
            .arg(&main)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
    };
    run("9");
    std::fs::write(&math, "public fn scale(n: Int) -> Int:\n    return n * 3\n").unwrap();
    run("13");
    std::fs::write(&helper, original.replace("+ 1", "+ 2")).unwrap();
    run("14");
    std::fs::write(&helper, original).unwrap();
    run("13");
}

#[test]
fn new_project_checks_and_runs_with_an_incomplete_installation() {
    let tmp = tempdir().unwrap();
    let install = tmp.path().join("install");
    std::fs::create_dir_all(install.join("std")).unwrap();
    std::fs::write(install.join("std/extra.cor"), "# local module\n").unwrap();
    let cli = |cwd: &std::path::Path, args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(cwd)
            .env("CORVID_HOME", &install)
            .args(args)
            .output()
            .unwrap()
    };
    let project = tmp.path().join("hello");
    let created = cli(tmp.path(), &["new", "hello"]);
    assert!(created.status.success(), "{created:?}");
    assert!(project.join("src/std/extra.cor").exists());
    let checked = cli(&project, &["check"]);
    assert!(checked.status.success(), "{checked:?}");
    let greeting = cli(&project, &["run"]);
    assert!(greeting.status.success(), "{greeting:?}");
    assert!(String::from_utf8_lossy(&greeting.stdout).contains("Hello, Corvid!"));
    std::fs::write(project.join("src/main.cor"), "agent main() -> Int:\n    return \"bad\"\n").unwrap();
    let rejected = cli(&project, &["check"]);
    assert!(!rejected.status.success());
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);
    assert!(diagnostic.contains("main.cor:2:") && diagnostic.contains("Int"), "{diagnostic}");
}

#[test]
fn check_rejects_missing_import() {
    let tmp = tempdir().expect("create tempdir");
    let main = tmp.path().join("main.cor");
    std::fs::write(
        &main,
        concat!(
            "import \"./does_not_exist\" use Anything\n",
            "\n",
            "agent main() -> Bool:\n",
            "    return true\n",
        ),
    )
    .expect("write main.cor");

    let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["check"])
        .arg(&main)
        .output()
        .expect("spawn corvid");

    assert!(
        !out.status.success(),
        "check should reject missing import; got status {:?}\nstdout={}\nstderr={}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let combined = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr);
    assert!(
        combined.contains("could not be found"),
        "expected import-not-found diagnostic, got:\n{combined}"
    );
}

#[test]
fn imported_function_values_preserve_identity_and_signature() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(dir.path().join("math.cor"), r#"
public fn double(n: Int) -> Int:
    return n * 2

public agent apply(value: Int, transform: (Int) -> Int) -> Int:
    return transform(value)
"#).unwrap();
    std::fs::write(&main, r#"
import "./math" as m
import "./math" use double as twice

fn increment(n: Int) -> Int:
    return twice(n) + 1

agent main() -> Bool:
    chosen = m.double
    mapped = [1, 2].map(twice)
    return m.apply(3, chosen) == 6 and m.apply(4, increment) == 9 and mapped[1] == 4
"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corvid")).args(["run", "--target", "interp"])
        .arg(&main).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");

    std::fs::write(&main, r#"
import "./math" as m

agent main() -> Int:
    callback: (String) -> Int = m.double
    return callback("x")
"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corvid")).arg("check").arg(&main).output().unwrap();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("(String) -> Int"), "{diagnostic}");
}

#[test]
fn integer_callbacks_cross_modules_and_outlive_captured_locals() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(dir.path().join("ops.cor"), r#"
public fn twice(n: Int) -> Int:
    return n * 2

public agent apply(value: Int, transform: (Int) -> Int) -> Int:
    return transform(value)

public agent make(base: Int, factor: Int) -> (Int) -> Int:
    return fn (n: Int) -> base + twice(n) * factor

public agent reader(values: List<Int>, offset: Int) -> (Int) -> Int:
    return fn (i: Int) -> values[i] + values.length() + offset
"#).unwrap();
    std::fs::write(&main, r#"
import "./ops" as ops

agent main() -> Bool:
    offset = 3
    add: (Int) -> Int = fn (n) -> n + offset
    offset = 10
    a = ops.apply(4, add)
    b = add(1)
    other = ops.make(3, 4)
    c = other(5)
    d = ops.apply(6, ops.twice)
    items = [1, 2]
    mapped = items.map(add)
    empty: List<Int> = []
    empty_result = empty.map(ops.twice)
    source_preserved = items[0] == 1 and items[1] == 2
    read = ops.reader(items, 4)
    items[0] = 10
    items.append(30)
    shared = read(0) == 17 and read(2) == 37
    temporary = ops.reader([8], 2)
    kept = temporary(0) == 11
    enabled = true
    predicate: (Int) -> Bool = fn (n) -> enabled and n >= items[0]
    enabled = false
    selected = [11, 2, 10, 1].filter(predicate)
    filtered = selected.length() == 2 and selected[0] == 11 and selected[1] == 10
    total = selected.fold(0, fn (acc, n) -> acc * 10 + n)
    has = [0, 5].any(fn (i) -> items[i] == 10)
    every = [0, 5].all(fn (i) -> items[i] < 0)
    vacuous = empty.all(predicate) and not empty.any(predicate)
    reduced = total == 120 and empty.fold(17, fn (a, b) -> a + b) == 17
    checks = has and not every and vacuous and reduced
    untouched = checks and source_preserved and filtered and shared and kept and mapped[0] == 4 and mapped[1] == 5
    return untouched and empty_result.length() == 0 and a == 7 and b == 4 and c == 43 and d == 12
"#).unwrap();

    assert_both_targets_without_leaks(&main);
}

fn assert_both_targets_without_leaks(main: &std::path::Path) {
    for target in ["interp", "native"] {
        let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .args(["run", "--target", target]).arg(&main)
            .env("CORVID_DEBUG_ALLOC", "1").output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{target}: {stderr}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
        if target == "native" {
            let count = |label: &str| stderr.lines().find_map(|line| line.strip_prefix(label))
                .unwrap().parse::<usize>().unwrap();
            assert!(count("ALLOCS=") > 0);
            assert_eq!(count("ALLOCS="), count("RELEASES="), "{stderr}");
        }
    }
}

#[test]
fn last_infers_element_types_across_modules() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(dir.path().join("items.cor"), r#"
public fn numbers() -> List<Int>:
    return [3, 0]

public fn labels() -> List<String>:
    return ["first", "la" + "st"]
"#).unwrap();
    std::fs::write(&main, r#"
import "./items" as items

agent observe(trace: List<Int>, value: String) -> String:
    trace.append(1)
    return value

agent accepts(label: Option<String>, enabled: Bool, trace: List<Int>) -> Bool:
    return match label:
        Some(s) if enabled and observe(trace, s) == "last" -> s != "" and s.length() == 4
        Some(s) -> enabled or observe(trace, s) == "other"
        None -> false

agent main() -> Bool:
    number = items.numbers().last()
    label = items.labels().last()
    actual_number = match number:
        Some(n) -> n
        None -> -1
    actual_label = match label:
        Some(s) -> s
        None -> ""
    trace: List<Int> = []
    accepted = accepts(label, true, trace)
    skipped = accepts(label, false, trace)
    fallback = accepts(Some("other"), true, trace)
    absent = accepts(None, true, trace)
    unicode = ("🐧" + "é").length() == 2 and "é".length() == 2 and "".length() == 0
    return unicode and actual_number == 0 and actual_label == "last" and accepted and not skipped and fallback and not absent and trace.length() == 3
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn propagation_releases_values_live_in_enclosing_scopes() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
agent read(text: String) -> Result<Int, String>:
    saved = text + "!"
    if text.length() > 0:
        number = text.parse_int()?
        return Ok(number + saved.length())
    return Ok(0)

agent piece(text: String, item: Option<Int>) -> Option<String>:
    saved = text + "!"
    n = item?
    return Some(saved.substring(0, n))

agent read_rows(rows: List<String>) -> Result<String, String>:
    label = "sc" + "ore"
    for row in rows:
        if row == "skip":
            continue
        if row == "stop":
            break
        count = row.parse_int()?
        if count > 0:
            return Ok(label)
    return Ok("empty")

agent mark(trace: List<Int>, n: Int) -> Int:
    trace.append(n)
    return n

agent total(trace: List<Int>, text: String) -> Result<Int, String>:
    saved = text + "!"
    return Ok(mark(trace, 1) + text.parse_int()? + mark(trace, 2) + saved.length())

agent lazy(text: String, fallback: String, enabled: Bool) -> Result<Bool, String>:
    saved = text + "!"
    required = enabled and text.parse_int()? > 0
    optional = enabled or fallback.parse_int()? == 0
    return Ok(required and optional and saved.length() > 0)

agent main() -> Bool:
    failed = read("bad").is_err()
    recovered = match read("3"):
        Ok(n) -> n == 5
        Err(_) -> false
    missing = piece("foo", None) == None
    found = match piece("foo", Some(2)):
        Some(s) -> s == "fo"
        None -> false
    bad_row = read_rows(["0", "bad"]).is_err()
    exhausted = read_rows([]).is_ok() and read_rows(["0"]).is_ok()
    stopped_rows = read_rows(["skip", "stop", "bad"]).is_ok()
    good_row = match read_rows(["0", "1"]):
        Ok(s) -> s == "score"
        Err(_) -> false
    trace: List<Int> = []
    stopped = total(trace, "bad").is_err()
    sum = match total(trace, "5"):
        Ok(n) -> n == 10
        Err(_) -> false
    order = trace.length() == 3 and trace[0] == 1 and trace[1] == 1 and trace[2] == 2
    yes: Result<Bool, String> = Ok(true)
    no: Result<Bool, String> = Ok(false)
    lazy_paths = lazy("bad", "0", true).is_err() and lazy("1", "bad", false).is_err() and lazy("1", "bad", true) == yes and lazy("bad", "0", false) == no
    return failed and recovered and missing and found and bad_row and good_row and exhausted and stopped_rows and stopped and sum and order and lazy_paths
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn conversions_agree_at_rounding_and_integer_boundaries() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
agent main() -> Bool:
    lower = (-9223372036854775808.0).to_int_truncated()
    upper = 9223372036854774784.0.to_int_truncated()
    bounds = lower == -9223372036854775807 - 1 and upper == 9223372036854774784
    signed = (-3.7).to_int_truncated() == -3 and (-3.7).floor() == -4 and (-3.7).ceil() == -3
    ties = (-2.5).round() == -3 and 2.5.round() == 3 and 0.49999999999999994.round() == 0
    precise = 9007199254740993.to_float().to_int_truncated() == 9007199254740992
    input: Option<Float> = Some(-0.5)
    negative_half = match input:
        Some(n) if n.ceil() == 0 -> n.round() == -1
        _ -> false
    return bounds and signed and ties and precise and negative_half
"#).unwrap();
    for target in ["interp", "native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .args(["run", "--target", target]).arg(&main).output().unwrap();
        assert!(out.status.success(), "{target}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "true");
    }
    for expression in ["9223372036854775808.0.to_int_truncated()", "(-9223372036854777856.0).ceil()", "(huge - huge).round()", "huge.floor()"] {
        let program = format!("agent main() -> Int:\n    huge = 9223372036854775808.0\n    for i in [1, 2, 3, 4, 5]:\n        huge = huge * huge\n    return {expression}\n");
        std::fs::write(&main, program).unwrap();
        for target in ["interp", "native"] {
            let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
                .args(["run", "--target", target]).arg(&main).output().unwrap();
            let message = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success() && message.contains("within Int range"), "{target} {expression}: {message}");
        }
    }
}

#[test]
fn collections_and_tagged_values_compare_by_contents() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
agent main() -> Bool:
    a = [1, 2]
    b = [1, 2]
    x: Option<Int> = Some(0)
    y: Option<Int> = Some(0)
    ok: Result<Int, String> = Ok(0)
    other_ok: Result<Int, String> = Ok(0)
    err: Result<Int, String> = Err("e")
    counts = {"a": 0, "b": 2}
    copy = {"b": 2, "a": 0}
    equal = a == b and x == y and x != None and ok == other_ok and ok != err and counts == copy
    b[1] = 7
    copy["a"] = 1
    return equal and a != b and counts != copy
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn equality_terminates_on_recursive_record_pairs() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
type Node:
    value: Int
    link: Option<Node>

agent main() -> Bool:
    empty: Option<Node> = None
    left = Node(3, empty)
    right = Node(3, empty)
    left.link = Some(left)
    right.link = Some(right)
    equal = left == right
    right.value = 4
    unequal = left != right
    left.link = empty
    right.link = empty
    return equal and unequal
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn records_compare_collection_fields_and_tagged_payloads() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
type Row:
    enabled: Bool
    weight: Float
    labels: List<String>
    counts: Map<String, Int>
    score: Option<Int>
    status: Result<Int, String>

agent main() -> Bool:
    labels = ["one", "two"]
    a = Row(true, -0.0, labels, {"x": 3, "y": 0}, Some(0), Err("bad"))
    b = Row(true, 0.0, ["one", "two"], {"y": 0, "x": 3}, Some(0), Err("b" + "ad"))
    same = a == b
    b.enabled = false
    flag = a != b
    b.enabled = true
    b.score = None
    absent = a != b
    b.score = Some(0)
    b.status = Ok(0)
    tag = a != b
    b.status = Err("bad")
    labels[0] = "other"
    return same and flag and absent and tag and a != b
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn both_targets_compare_nested_records_and_aliases() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
type Item:
    value: Int

type Boxed:
    item: Item

agent main() -> Bool:
    item = Item(3)
    first = Boxed(item)
    alias = first
    independent = Boxed(Item(3))
    shared = Boxed(item)
    equal = first == alias and first == independent and first == shared
    item.value = 9
    return equal and first == shared and first != independent
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn string_search_split_and_replace_agree_across_targets() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
agent rewrite(value: Result<String, String>) -> Result<String, String>:
    return Ok(("🐧" + "🐧").replace("🐧".to_lower(), value?))

agent pieces(value: Option<String>) -> List<String>:
    return match value:
        Some(s) -> s.trim().split("🐧")
        None -> "".split(",")

agent main() -> Bool:
    words = pieces(Some("　🐧猫🐧🐧　"))
    empty = pieces(None)
    fields = words.length() == 4 and words[0] == "" and words[1] == "猫" and words[2] == "" and words[3] == ""
    text = " 猫🐧 ".trim()
    checks = text.contains("猫🐧") and text.starts_with("猫") and text.ends_with("🐧") and not text.contains("犬")
    patterns = text.contains("") and text.starts_with("") and text.ends_with("") and "猫".replace("", "-") == "-猫-"
    replaced = match rewrite(Ok("犬")):
        Ok(s) -> s == "犬犬"
        Err(_) -> false
    failed = rewrite(Err("missing")).is_err()
    return fields and empty.length() == 1 and empty[0] == "" and checks and patterns and replaced and failed
"#).unwrap();
    assert_both_targets_without_leaks(&main);
    std::fs::write(&main, "agent main() -> Int:\n    return \"a\".split(\"\").length()\n").unwrap();
    for target in ["interp", "native"] {
        let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .args(["run", "--target", target]).arg(&main).output().unwrap();
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && diagnostic.contains("non-empty separator"), "{target}: {diagnostic}");
    }
}

#[test]
fn unicode_slices_and_case_conversions_preserve_scalars() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(&main, r#"
agent next(trace: List<Int>, value: Int) -> Int:
    trace.append(value)
    return value

agent cut(code: String) -> Result<String, String>:
    return Ok(("A" + "🐧").substring(0, code.parse_int()?))

agent main() -> Bool:
    text = "A🐧é"
    trace: List<Int> = []
    piece = text.substring(next(trace, 1), next(trace, 2))
    bounds = text.substring(-8, 1) == "A" and text.substring(2, 9223372036854775807) == "é"
    empty = text.substring(3, 2) == "" and text.substring(99, 100) == "" and "".substring(0, 1) == ""
    cases = "straße".to_upper() == "STRASSE" and "ΟΣ".to_lower() == "ος" and "İ".to_lower().length() == 2
    selected = match cut("2"):
        Ok(s) -> s.substring(1, 2).to_lower() == piece
        Err(_) -> false
    failed = cut("invalid").is_err()
    return selected and failed and bounds and empty and cases and trace[0] == 1 and trace[1] == 2 and piece == "🐧"
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn imported_sum_types_preserve_variant_payloads() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.cor");
    std::fs::write(dir.path().join("readings.cor"), r#"
public type Entry:
    value: Int
    label: String

public type Reading:
    | Empty
    | Found(item: Entry, weight: Int)

public agent lookup(n: Int) -> Reading:
    if n < 0:
        return Empty
    return Found(Entry(n, "fo" + "und"), n + 1)

public agent entry(reading: Reading) -> Entry:
    return match reading:
        Found(item, weight) if weight > 1 -> item
        Found(_, _) -> Entry(0, "zero")
        Empty -> Entry(-1, "empty")
"#).unwrap();
    std::fs::write(&main, r#"
import "./readings" as r

agent main() -> Bool:
    reading = r.lookup(7)
    good = r.entry(reading)
    zero = r.entry(r.lookup(0))
    missing = r.entry(r.lookup(-1))
    present = good.value == 7 and good.label == "found"
    same = reading == r.lookup(7) and r.lookup(-1) == r.lookup(-2)
    different = reading != r.lookup(8) and reading != r.lookup(-1)
    return same and different and present and zero.label == "zero" and missing.value == -1 and missing.label == "empty"
"#).unwrap();
    assert_both_targets_without_leaks(&main);
}

#[test]
fn imported_functions_keep_their_signatures_and_run_across_module_boundaries() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("math.cor"), "public fn twice(n: Int) -> Int:\n    return n + n\npublic agent number() -> Int:\n    return 7\n").unwrap();
    let main = dir.path().join("main.cor");
    let invoke = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_corvid"))
        .current_dir(dir.path()).args(args).output().unwrap();
    for (import, name) in [
        ("import \"./math\" as math", "math.twice"),
        ("import \"./math\" use twice as double", "double"),
    ] {
        let program = format!("{import}\nfn answer() -> Int:\n    return {name}(21)\nagent main() -> Int:\n    return answer()\n");
        std::fs::write(&main, &program).unwrap();
        let result = invoke(&["run", "main.cor", "--target", "interp"]);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "42");
        std::fs::write(&main, program.replace("(21)", "(\"bad\")")).unwrap();
        assert!(!invoke(&["check", "main.cor"]).status.success());
    }
    std::fs::write(&main, "import \"./math\" as math\nfn answer() -> Int:\n    return math.number()\n").unwrap();
    let result = invoke(&["check", "main.cor"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("must be effect-free"));
}
