//! B09 — every value of a refined field holds its refinement, however it
//! is built. A constant out of range is a compile error; a computed one
//! stops the run where the record is built or the field assigned, with
//! the field, the refinement and the value named; a stored row that
//! breaks one is refused when it is read.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const TYPES: &str = "
type Search:
    query: String where len_between(1, 200)
    limit: Int where between(1, 50)
";

fn compile_errors(body: &str) -> Vec<String> {
    match compile_to_ir(&format!("{TYPES}\n{body}")) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

async fn run(body: &str, args: Vec<Value>) -> Result<Value, String> {
    let ir = compile_to_ir(&format!("{TYPES}\n{body}")).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    run_ir_with_runtime(&ir, Some("main"), args, &Runtime::builder().build()).await.map_err(|e| e.to_string())
}

#[test]
fn a_constant_out_of_range_is_a_compile_error_wherever_it_is_given() {
    let constructor = compile_errors("agent main() -> Int:\n    search = Search(\"dune\", 500)\n    return search.limit\n");
    assert!(
        constructor.iter().any(|m| m == "`Search.limit` must be between(1, 50), but this value is 500"),
        "{constructor:#?}"
    );
    let literal = compile_errors("agent main() -> Int:\n    search = Search { query: \"\", limit: 5 }\n    return search.limit\n");
    assert!(
        literal.iter().any(|m| m == "`Search.query` must be len_between(1, 200), but this value is \"\""),
        "{literal:#?}"
    );
    let negative = compile_errors("agent main() -> Int:\n    search = Search(\"dune\", -1)\n    return search.limit\n");
    assert!(negative.iter().any(|m| m.contains("but this value is -1")), "{negative:#?}");
    let fine = compile_errors("agent main() -> Int:\n    search = Search(\"dune\", 50)\n    return search.limit\n");
    assert!(fine.is_empty(), "{fine:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computed_value_out_of_range_stops_where_the_record_is_built() {
    let body = "agent main(n: Int) -> Int:\n    search = Search(\"dune\", n)\n    return search.limit\n";
    assert_eq!(run(body, vec![Value::Int(20)]).await, Ok(Value::Int(20)));
    let stopped = run(body, vec![Value::Int(500)]).await.unwrap_err();
    assert!(stopped.contains("`Search.limit` must be between(1, 50), but this value is 500"), "{stopped}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_record_update_and_a_field_assignment_are_checked_too() {
    let update = "agent main(n: Int) -> Int:\n    base = Search(\"dune\", 10)\n    wider = Search { limit: n, ..base }\n    return wider.limit\n";
    assert_eq!(run(update, vec![Value::Int(40)]).await, Ok(Value::Int(40)));
    let stopped = run(update, vec![Value::Int(51)]).await.unwrap_err();
    assert!(stopped.contains("`Search.limit` must be between(1, 50), but this value is 51"), "{stopped}");

    let assigned = "agent main(n: Int) -> Int:\n    search = Search(\"dune\", 10)\n    search.limit = n\n    return search.limit\n";
    let stopped = run(assigned, vec![Value::Int(0)]).await.unwrap_err();
    assert!(stopped.contains("`Search.limit` must be between(1, 50), but this value is 0"), "{stopped}");
    let grown = "agent main(n: Int) -> Int:\n    search = Search(\"dune\", 45)\n    search.limit += n\n    return search.limit\n";
    let stopped = run(grown, vec![Value::Int(10)]).await.unwrap_err();
    assert!(stopped.contains("but this value is 55"), "{stopped}");
}

/// A stored row that breaks its table's refinement was written outside
/// this program; reading it is refused rather than handed on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stored_row_that_breaks_a_refinement_is_refused_when_read() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = dir.path().join("library.sqlite").to_string_lossy().replace('\\', "/");
    let source = format!(
        "database library:\n    engine: sqlite\n    location: \"{database}\"\n\ntable Shelf:\n    key shelf: Int\n    title: String where len_between(1, 10)\n\nagent main() -> String:\n    return match Shelf.get(1):\n        Some(row) -> row.title\n        None -> \"none\"\n"
    );
    let connection = rusqlite::Connection::open(&database).expect("open");
    connection
        .execute_batch(
            "CREATE TABLE \"Shelf\" (\"shelf\" INTEGER PRIMARY KEY NOT NULL, \"title\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);
             INSERT INTO \"Shelf\" VALUES (1, 'a title far longer than ten', 1);",
        )
        .expect("seed a row written outside the program");
    drop(connection);
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let error = run_ir_with_runtime(&ir, Some("main"), vec![], &Runtime::builder().build()).await.unwrap_err().to_string();
    assert!(error.contains("a stored `Shelf` row breaks `Shelf.title` must be len_between(1, 10)"), "{error}");
}

/// Native code does not check refinements yet, so a program that gives a
/// refined field a computed value runs on the interpreter, where it is
/// checked. A constant, which the checker proved, stays native.
#[test]
fn a_computed_refined_value_keeps_the_program_on_the_interpreter() {
    let ir = |body: &str| compile_to_ir(&format!("{TYPES}\n{body}")).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let constant = ir("agent main() -> Int:\n    search = Search(\"dune\", 20)\n    return search.limit\n");
    assert!(corvid_driver::native_ability(&constant).is_ok());
    let computed = ir("agent main(n: Int) -> Int:\n    search = Search(\"dune\", n)\n    return search.limit\n");
    let reason = corvid_driver::native_ability(&computed).expect_err("a computed refined value is not native yet");
    assert!(reason.to_string().contains("`Search.limit` is given a computed value"), "{reason}");
    let assigned = ir("agent main(n: Int) -> Int:\n    search = Search(\"dune\", 20)\n    search.limit = n\n    return search.limit\n");
    assert!(corvid_driver::native_ability(&assigned).is_err());
}
