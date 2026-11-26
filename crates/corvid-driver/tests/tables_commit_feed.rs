//! B14 RT-2 — the commit feed live subscriptions listen to. A table
//! write outside an `atomic:` block is announced once it succeeds; a
//! block announces its tables when it commits and nothing when it rolls
//! back; a conflict writes nothing and announces nothing.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::tables::TableCommit;
use corvid_runtime::Runtime;
use corvid_vm::Value;

fn program(database: &str) -> String {
    format!(
        r#"
database board:
    engine: sqlite
    location: "{database}"

table Card:
    key card: Int
    title: String

table Note:
    key id: Int
    text: String

agent add(card: Int) -> String:
    return match Card.insert(Card(card, "first")):
        Ok(row) -> "added"
        Err(Taken(holder)) -> "taken"

agent both(card: Int) -> Int:
    atomic:
        made = Card.insert(Card(card, "pair"))
        noted = Note.insert(Note(card, "pair"))
    return card

agent undone(card: Int, zero: Int) -> Int:
    total = 0
    atomic:
        made = Card.insert(Card(card, "lost"))
        total = 1 / zero
    return total
"#
    )
}

fn drain(receiver: &mut tokio::sync::broadcast::Receiver<TableCommit>) -> Vec<TableCommit> {
    let mut seen = Vec::new();
    while let Ok(commit) = receiver.try_recv() {
        seen.push(commit);
    }
    seen
}

fn commit(tables: &[&str]) -> TableCommit {
    TableCommit { database: "board".into(), tables: tables.iter().map(|t| t.to_string()).collect() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_writes_are_announced_and_undone_ones_are_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = dir.path().join("board.sqlite").to_string_lossy().replace('\\', "/");
    let connection = rusqlite::Connection::open(&database).expect("open");
    connection
        .execute_batch(
            "CREATE TABLE \"Card\" (\"card\" INTEGER PRIMARY KEY NOT NULL, \"title\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);
             CREATE TABLE \"Note\" (\"id\" INTEGER PRIMARY KEY NOT NULL, \"text\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);",
        )
        .expect("create tables");
    drop(connection);
    let ir = compile_to_ir(&program(&database)).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let runtime = Runtime::builder().build();
    let mut feed = runtime.subscribe_table_commits();

    let added = run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(1)], &runtime).await.expect("runs");
    assert_eq!(added, Value::String("added".into()));
    assert_eq!(drain(&mut feed), [commit(&["Card"])], "a write is announced once it succeeds");

    let taken = run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(1)], &runtime).await.expect("runs");
    assert_eq!(taken, Value::String("taken".into()));
    assert!(drain(&mut feed).is_empty(), "a conflict wrote nothing, so nothing is announced");

    run_ir_with_runtime(&ir, Some("both"), vec![Value::Int(2)], &runtime).await.expect("runs");
    assert_eq!(drain(&mut feed), [commit(&["Card", "Note"])], "a block announces its tables once, at commit");

    let failed = run_ir_with_runtime(&ir, Some("undone"), vec![Value::Int(3), Value::Int(0)], &runtime).await;
    assert!(failed.is_err(), "dividing by zero fails the block");
    assert!(drain(&mut feed).is_empty(), "a rolled-back block announces nothing");
}
