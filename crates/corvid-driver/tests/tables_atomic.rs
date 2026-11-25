//! B09 P7a — `atomic:` blocks commit their table writes together or not
//! at all, on SQLite and, when `CORVID_TEST_POSTGRES_URL` is set, on
//! Postgres: a stale write's `?` undoes the write before it, a runtime
//! failure undoes the block, a recorded run replays without the
//! database, and concurrent atomic writers on one SQLite file neither
//! deadlock nor lose a write.

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

table Shelf:
    key shelf: Int
    title: String

table Ledger:
    key entry: Int
    note: String

agent put(row: Shelf) -> Int:
    return match Shelf.insert(row):
        Ok(_) -> 1
        Err(_) -> 0

agent titles() -> String:
    out = ""
    for shelf in Shelf.all():
        out = out + shelf.title + " r" + shelf.revision.to_string() + ","
    return out

agent rename_pair(left: String, right: String, left_seen: Int, right_seen: Int) -> Result<String, Option<Shelf>>:
    atomic:
        first = Shelf.save(Shelf(1, left, left_seen))?
        second = Shelf.save(Shelf(2, right, right_seen))?
        logged = Ledger.insert(Ledger(left_seen, left + "+" + right))
    return Ok(first.title + "+" + second.title)

agent describe(outcome: Result<String, Option<Shelf>>) -> String:
    return match outcome:
        Ok(text) -> text
        Err(_) -> "conflict, nothing kept"

agent trap_after_write(zero: Int) -> String:
    atomic:
        moved = Shelf.save(Shelf(1, "Trapped", 2))
        broken = 10 / zero
    return "unreachable"

agent main() -> String:
    seeded = put(Shelf(1, "Odyssey")) + put(Shelf(2, "Persuasion"))
    first = describe(rename_pair("Iliad", "Emma", 1, 1))
    stale = describe(rename_pair("Lost", "Gone", 2, 1))
    return seeded.to_string() + " | " + first + " | " + stale + " | " + titles()
"#
    )
}

const EXPECTED: &str = "2 | Iliad+Emma | conflict, nothing kept | Iliad r2,Emma r2,";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_atomic_block_commits_together_or_not_at_all() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        assert_eq!(out, Value::String(EXPECTED.into()), "on {}", database.label());

        // A runtime failure inside the block undoes its write.
        let error = run_ir_with_runtime(&ir, Some("trap_after_write"), vec![Value::Int(0)], &runtime)
            .await
            .expect_err("dividing by zero fails");
        assert!(!format!("{error}").is_empty());
        let after = run_ir_with_runtime(&ir, Some("titles"), vec![], &runtime).await.expect("titles");
        assert_eq!(after, Value::String("Iliad r2,Emma r2,".into()), "the failed block kept nothing on {}", database.label());
        database.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_atomic_run_replays_without_the_database() {
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
        assert!(raw.contains("table:library.begin"), "transactions are recorded on {}", database.label());
        assert!(raw.contains("table:library.commit") && raw.contains("table:library.rollback"), "both endings are recorded");

        database.remove().await;
        let replay = Runtime::builder().replay_from(&trace).build();
        let replayed = run_ir_with_runtime(&ir, Some("main"), vec![], &replay).await.expect("replay from the trace");
        assert_eq!(replayed, recorded, "on {}", database.label());
        assert!(database.is_gone().await, "a replay never opens the database on {}", database.label());
        database.finish().await;
    }
}

/// Two runtimes write to one SQLite file at once, each in its own
/// `atomic:` blocks. A transaction holds the file's writer turn, so the
/// other waits in the queue instead of deadlocking on SQLite's lock,
/// and every write lands.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_atomic_writers_on_one_file_neither_deadlock_nor_lose_writes() {
    let database = TestDatabase::sqlite();
    let source = format!(
        "{}\nagent log_many(start: Int, count: Int) -> Int:\n    written = 0\n    for i in range(start, start + count):\n        atomic:\n            entry = Ledger.insert(Ledger(i, \"entry\"))\n        written = written + 1\n    return written\n\nagent entries() -> Int:\n    total = 0\n    for row in Ledger.all():\n        total = total + 1\n    return total\n",
        program(&database.settings())
    );
    database.create_tables(&source).await;
    let ir = std::sync::Arc::new(compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}")));
    let writer = |start: i64| {
        let ir = ir.clone();
        tokio::spawn(async move {
            let runtime = Runtime::builder().build();
            run_ir_with_runtime(&ir, Some("log_many"), vec![Value::Int(start), Value::Int(25)], &runtime).await
        })
    };
    let (a, b) = tokio::time::timeout(std::time::Duration::from_secs(60), async { tokio::join!(writer(100), writer(200)) })
        .await
        .expect("concurrent atomic writers finish without deadlocking");
    assert_eq!(a.unwrap().expect("first writer"), Value::Int(25));
    assert_eq!(b.unwrap().expect("second writer"), Value::Int(25));
    let runtime = Runtime::builder().build();
    let total = run_ir_with_runtime(&ir, Some("entries"), vec![], &runtime).await.expect("count");
    assert_eq!(total, Value::Int(50), "every write landed");
}
