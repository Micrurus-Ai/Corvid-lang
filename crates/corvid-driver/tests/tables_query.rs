//! B09 P2 — typed queries run end to end on SQLite through the
//! interpreter: `where` conditions, `order_by`, `descending` and `take`
//! become one parameterized `SELECT`, and a recorded run replays its
//! queries without the database.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const SCHEMA: &str = "CREATE TABLE \"Book\" (\"id\" INTEGER PRIMARY KEY NOT NULL, \"title\" TEXT NOT NULL, \
\"year\" INTEGER NOT NULL, \"rating\" REAL NOT NULL, \"done\" INTEGER NOT NULL, \"due\" TEXT, \
\"revision\" INTEGER NOT NULL)";

fn program(path: &str) -> String {
    format!(
        r#"
database library:
    engine: sqlite
    location: "{path}"
    tables: Book

table Book:
    key id: Int
    title: String
    year: Int
    rating: Float
    done: Bool
    due: Option<String>

agent titles(books: List<Book>) -> String:
    out = ""
    for book in books:
        out = out + book.title + ","
    return out

agent stock() -> Int:
    added = 0
    for row in [Book(1, "Dune", 1965, 4.5, true, None), Book(2, "Neuromancer", 1984, 4.0, false, Some("monday")), Book(3, "Anathem", 2008, 4.5, false, None), Book(4, "Annihilation", 2014, 3.5, true, Some("friday")), Book(5, "Piranesi", 2020, 4.8, false, None)]:
        added = added + put(row)
    return added

agent put(row: Book) -> Int:
    return match Book.insert(row):
        Ok(_) -> 1
        Err(_) -> 0

agent main(since: Int, quote: String) -> String:
    stocked = stock()
    recent = titles(Book.where(fn (b) -> b.year >= since and not b.done).order_by(fn (b) -> b.year).descending())
    top = titles(Book.where(fn (b) -> 4.4 < b.rating).order_by(fn (b) -> b.rating).descending().take(2))
    named = titles(Book.where(fn (b) -> b.title.starts_with("An") or b.title.ends_with("cer") or b.title.contains(quote)))
    undated = titles(Book.where(fn (b) -> b.due == None))
    dated = titles(Book.where(fn (b) -> b.due != None and b.done))
    finished = titles(Book.where(fn (b) -> b.done))
    every = titles(Book.all())
    none = titles(Book.all().take(0))
    return stocked.to_string() + " | " + recent + " | " + top + " | " + named + " | " + undated + " | " + dated + " | " + finished + " | " + every + " | " + none
"#
    )
}

const EXPECTED: &str = "5 | Piranesi,Anathem,Neuromancer, | Piranesi,Dune, | Neuromancer,Anathem,Annihilation,Piranesi, | \
Dune,Anathem,Piranesi, | Annihilation, | Dune,Annihilation, | Dune,Neuromancer,Anathem,Annihilation,Piranesi, | ";

fn args() -> Vec<Value> {
    vec![Value::Int(1980), Value::String("ane".into())]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queries_filter_order_and_limit_in_the_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("library.sqlite").to_string_lossy().replace('\\', "/");
    let runtime = Runtime::builder().build();
    let setup = runtime.db_registry().open(&path).expect("open database");
    runtime.db_registry().execute(setup.handle_id, SCHEMA, &[]).expect("create table");

    let ir = compile_to_ir(&program(&path)).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let out = run_ir_with_runtime(&ir, Some("main"), args(), &runtime).await.expect("program runs");
    let Value::String(text) = out else { panic!("expected text, got {out:?}") };
    assert_eq!(text.as_ref(), EXPECTED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_run_replays_its_queries_without_the_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("library.sqlite");
    let location = path.to_string_lossy().replace('\\', "/");
    let ir = compile_to_ir(&program(&location)).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));

    let trace_dir = tempfile::tempdir().expect("trace dir");
    let record = Runtime::builder().trace_to(trace_dir.path()).build();
    let setup = record.db_registry().open(&location).expect("open database");
    record.db_registry().execute(setup.handle_id, SCHEMA, &[]).expect("create table");
    let recorded = run_ir_with_runtime(&ir, Some("main"), args(), &record).await.expect("recorded run");
    let trace = record.tracer().path().to_path_buf();
    drop(record);
    let raw = std::fs::read_to_string(&trace).expect("trace readable");
    assert!(raw.contains("table:Book.query"), "queries are recorded");
    assert!(raw.contains("LIMIT ?"), "the recorded SQL binds its values");

    std::fs::remove_file(&path).expect("remove the database");
    let replay = Runtime::builder().replay_from(&trace).build();
    let replayed = run_ir_with_runtime(&ir, Some("main"), args(), &replay).await.expect("replay from the trace");
    assert_eq!(replayed, recorded);
    assert!(!path.exists(), "a replay never opens the database");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_negative_take_stops_the_run_with_its_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("library.sqlite").to_string_lossy().replace('\\', "/");
    let src = format!(
        "{}\nagent first(n: Int) -> String:\n    return titles(Book.all().take(n))\n",
        program(&path)
    );
    let runtime = Runtime::builder().build();
    let setup = runtime.db_registry().open(&path).expect("open database");
    runtime.db_registry().execute(setup.handle_id, SCHEMA, &[]).expect("create table");
    let ir = compile_to_ir(&src).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let error = run_ir_with_runtime(&ir, Some("first"), vec![Value::Int(-1)], &runtime)
        .await
        .expect_err("a negative take fails");
    assert!(error.to_string().contains("take(-1)"), "{error}");
}
