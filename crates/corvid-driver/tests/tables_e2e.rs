//! B09 P1 — typed table operations run end to end on SQLite through the
//! interpreter: insert, get, save and delete, with revision-checked
//! writes whose conflicts come back as typed `Result` values.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const SCHEMA: &str = "CREATE TABLE \"Shelf\" (\"shelf\" INTEGER PRIMARY KEY NOT NULL, \"owner\" TEXT NOT NULL, \
\"title\" TEXT NOT NULL, \"note\" TEXT, \"tags\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL)";

fn program(path: &str) -> String {
    format!(
        r#"
database library:
    engine: sqlite
    location: "{path}"
    tables: Shelf

table Shelf:
    key shelf: Int
    owner: String
    title: String
    note: Option<String>
    tags: List<String>

agent inserted(row: Shelf) -> String:
    return match Shelf.insert(row):
        Ok(stored) -> "inserted r" + stored.revision.to_string()
        Err(existing) -> "taken by " + existing.title

agent saved(row: Shelf) -> String:
    return match Shelf.save(row):
        Ok(stored) -> "saved r" + stored.revision.to_string()
        Err(current) -> describe(current)

agent deleted(row: Shelf) -> String:
    return match Shelf.delete(row):
        Ok(gone) -> "deleted " + gone.title
        Err(current) -> describe(current)

agent describe(current: Option<Shelf>) -> String:
    return match current:
        Some(row) -> "conflict with r" + row.revision.to_string() + " " + row.title
        None -> "conflict with nothing"

agent read(id: Int) -> String:
    return match Shelf.get(id):
        Some(row) -> row.title + " " + row.tags[0] + " r" + row.revision.to_string()
        None -> "absent"

agent main() -> String:
    first = inserted(Shelf(1, "ana", "Reading", None, ["fiction"]))
    again = inserted(Shelf {{ shelf: 1, owner: "bo", title: "Other", note: None, tags: [] }})
    found = read(1)
    stale = Shelf(1, "ana", "Reading", None, ["fiction"], 1)
    renamed = saved(Shelf {{ title: "Novels", ..stale }})
    lost = saved(Shelf {{ title: "Lost", ..stale }})
    kept = deleted(stale)
    fresh = Shelf(1, "ana", "Novels", None, ["fiction"], 2)
    gone = deleted(fresh)
    after = read(1)
    orphan = saved(fresh)
    return first + " | " + again + " | " + found + " | " + renamed + " | " + lost + " | " + kept + " | " + gone + " | " + after + " | " + orphan
"#
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_operations_run_with_revision_checked_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("library.sqlite").to_string_lossy().replace('\\', "/");
    let runtime = Runtime::builder().build();
    let setup = runtime.db_registry().open(&path).expect("open database");
    runtime.db_registry().execute(setup.handle_id, SCHEMA, &[]).expect("create table");

    let ir = compile_to_ir(&program(&path)).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
    let Value::String(text) = out else { panic!("expected text, got {out:?}") };
    assert_eq!(
        text.as_ref(),
        "inserted r1 | taken by Reading | Reading fiction r1 | saved r2 | conflict with r2 Novels | \
         conflict with r2 Novels | deleted Novels | absent | conflict with nothing"
    );
}

/// P1b: a recorded run replays from its trace alone. The database is
/// deleted before the replay, and the replay neither needs it nor
/// recreates it, yet every read and every write outcome comes back as
/// recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_run_replays_its_table_operations_without_the_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("library.sqlite");
    let location = path.to_string_lossy().replace('\\', "/");
    let ir = compile_to_ir(&program(&location)).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));

    let trace_dir = tempfile::tempdir().expect("trace dir");
    let record = Runtime::builder().trace_to(trace_dir.path()).build();
    let setup = record.db_registry().open(&location).expect("open database");
    record.db_registry().execute(setup.handle_id, SCHEMA, &[]).expect("create table");
    let recorded = run_ir_with_runtime(&ir, Some("main"), vec![], &record).await.expect("recorded run");
    let trace = record.tracer().path().to_path_buf();
    drop(record);
    let raw = std::fs::read_to_string(&trace).expect("trace readable");
    assert!(raw.contains("table:Shelf.save"), "table operations are recorded");

    std::fs::remove_file(&path).expect("remove the database");
    let replay = Runtime::builder().replay_from(&trace).build();
    let replayed = run_ir_with_runtime(&ir, Some("main"), vec![], &replay).await.expect("replay from the trace");
    assert_eq!(replayed, recorded);
    assert!(!path.exists(), "a replay never opens the database");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_table_names_the_migration_commands() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.sqlite").to_string_lossy().replace('\\', "/");
    let ir = compile_to_ir(&program(&path)).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let runtime = Runtime::builder().build();
    let error = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect_err("no table yet");
    let message = format!("{error}");
    assert!(message.contains("corvid migrate plan"), "{message}");
}
