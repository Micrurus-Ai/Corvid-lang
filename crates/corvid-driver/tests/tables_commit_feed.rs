//! B14 RT-2 — the commit feed live subscriptions listen to. A table
//! write outside an `atomic:` block is announced once it succeeds; a
//! block announces its tables when it commits and nothing when it rolls
//! back; a conflict writes nothing and announces nothing. Each commit
//! names the rows it wrote and the revision each reached, so a
//! subscription can send a removal with the revision of the write that
//! removed the row.

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

agent rework(card: Int) -> String:
    return match Card.get(card):
        Some(found) -> rename_then_delete(Card {{ title: "renamed", ..found }})
        None -> "missing"

agent rename_then_delete(changed: Card) -> String:
    return match Card.save(changed):
        Ok(stored) -> remove(stored)
        Err(conflict) -> "conflict"

agent remove(stored: Card) -> String:
    return match Card.delete(stored):
        Ok(row) -> "deleted"
        Err(conflict) -> "conflict"
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

/// The tables of each commit, in order.
fn tables(commits: &[TableCommit]) -> Vec<Vec<&str>> {
    commits.iter().map(|c| c.tables.iter().map(String::as_str).collect()).collect()
}

/// The revision a commit reports for a row of `table` keyed `card`.
fn revision(commit: &TableCommit, table: &str, card: i64) -> Option<i64> {
    commit.rows.get(table)?.revision(&format!("[{card}]"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_writes_are_announced_with_their_rows_and_undone_ones_are_not() {
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
    let commits = drain(&mut feed);
    assert_eq!(tables(&commits), [["Card"]], "a write is announced once it succeeds");
    assert_eq!(commits[0].database, "board");
    assert_eq!(revision(&commits[0], "Card", 1), Some(1), "an insert reaches revision 1");

    let taken = run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(1)], &runtime).await.expect("runs");
    assert_eq!(taken, Value::String("taken".into()));
    assert!(drain(&mut feed).is_empty(), "a conflict wrote nothing, so nothing is announced");

    run_ir_with_runtime(&ir, Some("both"), vec![Value::Int(2)], &runtime).await.expect("runs");
    let commits = drain(&mut feed);
    assert_eq!(tables(&commits), [["Card", "Note"]], "a block announces its tables once, at commit");
    assert_eq!((revision(&commits[0], "Card", 2), revision(&commits[0], "Note", 2)), (Some(1), Some(1)), "with every row it wrote");

    let failed = run_ir_with_runtime(&ir, Some("undone"), vec![Value::Int(3), Value::Int(0)], &runtime).await;
    assert!(failed.is_err(), "dividing by zero fails the block");
    assert!(drain(&mut feed).is_empty(), "a rolled-back block announces nothing");

    let reworked = run_ir_with_runtime(&ir, Some("rework"), vec![Value::Int(1)], &runtime).await.expect("runs");
    assert_eq!(reworked, Value::String("deleted".into()));
    let commits = drain(&mut feed);
    assert_eq!(tables(&commits), [["Card"], ["Card"]], "a save and a delete, each its own commit");
    assert_eq!(revision(&commits[0], "Card", 1), Some(2), "a save reaches the row's next revision");
    assert_eq!(revision(&commits[1], "Card", 1), Some(3), "and a delete the revision after its last");
}

/// The key a commit names a row by is the key a live snapshot gives the
/// same row (`LiveRow::from_json` on its public JSON), for a text key
/// and a key of two fields too, so a removal finds its revision.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_commit_keys_a_row_as_the_live_snapshot_does() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = dir.path().join("tags.sqlite").to_string_lossy().replace('\\', "/");
    let connection = rusqlite::Connection::open(&database).expect("open");
    connection
        .execute_batch(
            "CREATE TABLE \"Tag\" (\"name\" TEXT PRIMARY KEY NOT NULL, \"label\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);
             CREATE TABLE \"Seat\" (\"row_name\" TEXT NOT NULL, \"number\" INTEGER NOT NULL, \"holder\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL, PRIMARY KEY (\"row_name\", \"number\"));",
        )
        .expect("create tables");
    drop(connection);
    let source = format!(
        r#"
database venue:
    engine: sqlite
    location: "{database}"

table Tag:
    key name: String
    label: String

table Seat:
    key row_name: String
    key number: Int
    holder: String

agent setup() -> Int:
    tagged = Tag.insert(Tag("café \"quoted\"", "first"))
    seated = Seat.insert(Seat("A", 12, "Ada"))
    return 0

agent tags() -> String:
    return Tag.all().to_json()

agent seats() -> String:
    return Seat.all().to_json()
"#
    );
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("{errors:#?}"));
    let runtime = Runtime::builder().build();
    let mut feed = runtime.subscribe_table_commits();
    run_ir_with_runtime(&ir, Some("setup"), vec![], &runtime).await.expect("runs");
    let commits = drain(&mut feed);

    for (agent, table, keys) in [("tags", "Tag", vec!["name"]), ("seats", "Seat", vec!["row_name", "number"])] {
        let Value::String(json) = run_ir_with_runtime(&ir, Some(agent), vec![], &runtime).await.expect("runs") else {
            panic!("{agent} returns text")
        };
        let keys: Vec<String> = keys.into_iter().map(String::from).collect();
        let rows: Vec<serde_json::Value> = serde_json::from_str(&json).expect("a JSON list");
        let snapshot = corvid_runtime::tables::LiveRow::from_json(rows[0].clone(), &keys).expect("a live row");
        let written = commits.iter().find_map(|c| c.rows.get(table)).expect("the commit names the table's row");
        assert_eq!(written.revision(&snapshot.key), Some(1), "{table}: the commit's key is the snapshot's key {}", snapshot.key);
    }
}
