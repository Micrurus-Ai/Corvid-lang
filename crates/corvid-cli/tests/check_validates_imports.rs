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

agent main() -> Bool:
    number = items.numbers().last()
    label = items.labels().last()
    actual_number = match number:
        Some(n) -> n
        None -> -1
    actual_label = match label:
        Some(s) -> s
        None -> ""
    return actual_number == 0 and actual_label == "last"
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
fn nested_relative_imports_resolve_and_report_boundary_errors() {
    let dir = tempdir().unwrap();
    let lib = dir.path().join("lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(dir.path().join("math.cor"), "public fn double(n: Int) -> Int:\n    return n * 2\n").unwrap();
    let ops = lib.join("ops.cor");
    std::fs::write(&ops, "import \"../math\" use double\npublic fn answer(n: Int) -> Int:\n    return double(n)\nfn hidden() -> Int:\n    return 0\n").unwrap();
    let main = dir.path().join("main.cor");
    let source = "import \"./lib/ops\" as ops\nagent main() -> Int:\n    return ops.answer(21)\n";
    let invoke = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(args).arg(&main).output().unwrap();
    std::fs::write(&main, source).unwrap();
    for target in ["interp", "native"] {
        let output = invoke(&["run", "--target", target]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");
    }
    let reject = |program: &str, message: &str| {
        std::fs::write(&main, program).unwrap();
        let output = invoke(&["check"]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success() && error.contains(message), "{error}");
    };
    reject(&source.replace("ops.answer(21)", "ops.hidden()"), "is private");
    reject(&source.replace("21", "\"bad\""), "type mismatch in argument 1");
    reject("import \"./lib/ops\" use answer\nfn answer() -> Int:\n    return 0\n", "duplicate declaration");
    std::fs::write(&ops, "import \"../main\" as root\npublic fn answer(n: Int) -> Int:\n    return n\n").unwrap();
    reject(source, "cyclic Corvid import graph");
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
