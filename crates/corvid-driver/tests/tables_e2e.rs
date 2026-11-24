//! B09 P1 and P4 — typed table operations run end to end through the
//! interpreter on SQLite and, when `CORVID_TEST_POSTGRES_URL` is set, on
//! Postgres: insert, get, save and delete, with revision-checked writes
//! whose conflicts come back as typed `Result` values.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database library:
{settings}
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
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let runtime = Runtime::builder().build();
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        let Value::String(text) = out else { panic!("expected text, got {out:?}") };
        assert_eq!(
            text.as_ref(),
            "inserted r1 | taken by Reading | Reading fiction r1 | saved r2 | conflict with r2 Novels | \
             conflict with r2 Novels | deleted Novels | absent | conflict with nothing",
            "on {}",
            database.label()
        );
        database.finish().await;
    }
}

/// P1b: a recorded run replays from its trace alone. The database is
/// removed before the replay, and the replay neither needs it nor
/// recreates it, yet every read and every write outcome comes back as
/// recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_run_replays_its_table_operations_without_the_database() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));

        let trace_dir = tempfile::tempdir().expect("trace dir");
        let record = Runtime::builder().trace_to(trace_dir.path()).build();
        let recorded = run_ir_with_runtime(&ir, Some("main"), vec![], &record).await.expect("recorded run");
        let trace = record.tracer().path().to_path_buf();
        drop(record);
        let raw = std::fs::read_to_string(&trace).expect("trace readable");
        assert!(raw.contains("table:Shelf.save"), "table operations are recorded on {}", database.label());

        database.remove().await;
        let replay = Runtime::builder().replay_from(&trace).build();
        let replayed = run_ir_with_runtime(&ir, Some("main"), vec![], &replay).await.expect("replay from the trace");
        assert_eq!(replayed, recorded, "on {}", database.label());
        assert!(database.is_gone().await, "a replay never opens the database on {}", database.label());
        database.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_table_names_the_migration_commands() {
    for database in TestDatabase::every_engine().await {
        let ir = compile_to_ir(&program(&database.settings())).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let error = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect_err("no table yet");
        let message = format!("{error}");
        assert!(message.contains("corvid migrate plan"), "on {}: {message}", database.label());
        database.finish().await;
    }
}
/// P4: `tls: required` verifies the server's certificate, so a server
/// with a self-signed certificate (the local test server's) is refused
/// before any row is read, with the declaration named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_required_tls_connection_refuses_an_unverified_server() {
    for database in TestDatabase::every_engine().await {
        if !matches!(database, TestDatabase::Postgres { .. }) {
            database.finish().await;
            continue;
        }
        let source = program(&database.settings().replace("tls: disabled", "tls: required"));
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let error = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect_err("an unverified server is refused");
        let message = format!("{error}");
        assert!(message.contains("tls: required"), "{message}");
        database.finish().await;
    }
}
