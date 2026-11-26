//! B14 RT-6 — commits shared between server processes on one Postgres
//! database. Two runtimes stand in for two processes: each has its own
//! instance id and commit feed, exactly as two `corvid serve` processes
//! would. A write committed through one reaches the other's feed; a
//! process does not hear its own commit twice; a rolled-back `atomic:`
//! block reaches nobody; a listener that lost its connection reconnects
//! and says any table may have changed. Runs when `CORVID_TEST_POSTGRES_URL` is set.

#[path = "support/table_engines.rs"]
mod table_engines;

use corvid_ast::DatabaseLocation;
use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::tables::TableCommit;
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::time::Duration;
use table_engines::TestDatabase;
use tokio::sync::broadcast::Receiver;

fn program(settings: &str) -> String {
    format!(
        r#"
database board:
{settings}

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

fn commit(tables: &[&str]) -> TableCommit {
    TableCommit { database: "board".into(), tables: tables.iter().map(|t| t.to_string()).collect() }
}

/// The next commit on a feed, waiting for a notification to arrive.
async fn next(feed: &mut Receiver<TableCommit>) -> Option<TableCommit> {
    tokio::time::timeout(Duration::from_secs(5), feed.recv()).await.ok().and_then(Result::ok)
}

/// Nothing more arrives in a while.
async fn quiet(feed: &mut Receiver<TableCommit>) -> bool {
    tokio::time::timeout(Duration::from_millis(500), feed.recv()).await.is_err()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_commit_in_one_process_reaches_the_others() {
    if std::env::var("CORVID_TEST_POSTGRES_URL").is_err() {
        eprintln!("skipping: set CORVID_TEST_POSTGRES_URL to share commits between processes on Postgres");
        return;
    }
    let database = TestDatabase::every_engine().await.pop().expect("a Postgres database");
    let TestDatabase::Postgres { env, .. } = &database else { panic!("the last engine is Postgres") };
    let location = DatabaseLocation::Env(env.clone());
    let source = program(&database.settings());
    database.create_tables(&source).await;
    let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("{errors:#?}"));

    let writer = Runtime::builder().build();
    let reader = Runtime::builder().build();
    writer.share_table_commits("board", &location, false).await.expect("the writer listens");
    reader.share_table_commits("board", &location, false).await.expect("the reader listens");
    let mut written = writer.subscribe_table_commits();
    let mut read = reader.subscribe_table_commits();

    let added = run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(1)], &writer).await.expect("runs");
    assert_eq!(added, Value::String("added".into()));
    assert_eq!(next(&mut read).await, Some(commit(&["Card"])), "the other process hears the write");
    assert_eq!(next(&mut written).await, Some(commit(&["Card"])), "the writer announces it locally");
    assert!(quiet(&mut written).await, "and does not hear its own commit a second time");

    let taken = run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(1)], &writer).await.expect("runs");
    assert_eq!(taken, Value::String("taken".into()));
    assert!(quiet(&mut read).await, "a conflict wrote nothing, so nobody hears of it");

    let failed = run_ir_with_runtime(&ir, Some("undone"), vec![Value::Int(3), Value::Int(0)], &writer).await;
    assert!(failed.is_err(), "dividing by zero fails the block");
    assert!(quiet(&mut read).await, "a rolled-back block reaches no other process");

    run_ir_with_runtime(&ir, Some("both"), vec![Value::Int(2)], &writer).await.expect("runs");
    assert_eq!(next(&mut read).await, Some(commit(&["Card", "Note"])), "a block reaches the others once, at commit");

    // The other process writes too: sharing goes both ways.
    run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(4)], &reader).await.expect("runs");
    assert_eq!(next(&mut read).await, Some(commit(&["Card"])), "the reader announces its own write locally");
    assert!(quiet(&mut read).await, "once");
    assert_eq!(next(&mut written).await, Some(commit(&["Card", "Note"])), "the writer's own block, announced locally");
    assert_eq!(next(&mut written).await, Some(commit(&["Card"])), "then the reader's write, shared");

    // Cut both listening connections, as a network drop or a database
    // restart would. Each reconnects and says any table may have
    // changed, since it cannot know what was committed while it was away.
    let url = std::env::var("CORVID_TEST_POSTGRES_URL").expect("set above");
    let (admin, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.expect("connect");
    tokio::spawn(connection);
    let cut = admin
        .query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = $1",
            &[&corvid_runtime::tables::LISTENER_NAME],
        )
        .await
        .expect("cut the listeners");
    assert_eq!(cut.len(), 2, "this test's two listeners, and no others");
    assert_eq!(next(&mut read).await, Some(commit(&[])), "after reconnecting: any table may have changed");
    assert_eq!(next(&mut written).await, Some(commit(&[])), "the writer's listener too");

    run_ir_with_runtime(&ir, Some("add"), vec![Value::Int(9)], &writer).await.expect("runs");
    assert_eq!(next(&mut read).await, Some(commit(&["Card"])), "a reconnected listener hears new commits");

    database.finish().await;
}
