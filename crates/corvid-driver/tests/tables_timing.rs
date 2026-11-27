//! Lineage from real runs (L2): every recorded table operation's trace
//! carries the time it waited for a turn and the time its statements ran,
//! on SQLite and, when `CORVID_TEST_POSTGRES_URL` is set, on Postgres.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_trace_schema::TraceEvent;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

const PROGRAM: &str = r#"
database library:
{settings}

table Shelf:
    key shelf: Int
    title: String

agent main() -> String:
    stored = match Shelf.insert(Shelf(1, "Reading")):
        Ok(row) -> row
        Err(Taken(holder)) -> holder
    saved = match Shelf.save(Shelf { title: "Novels", ..stored }):
        Ok(row) -> row.title
        Err(conflict) -> "conflict"
    return saved
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_table_operation_records_its_wait_and_running_time_on_every_engine() {
    for database in TestDatabase::every_engine().await {
        let source = PROGRAM.replace("{settings}", &database.settings());
        database.create_tables(&source).await;
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::builder().trace_to(dir.path()).build();
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        drop(runtime);
        let trace = std::fs::read_dir(dir.path()).unwrap().next().unwrap().unwrap().path();
        let events = corvid_trace_schema::read_events_from_path(&trace).expect("trace reads");

        let mut timed = Vec::new();
        for pair in events.windows(2) {
            if let [TraceEvent::DbTiming { tool, statement_us, statements, .. }, TraceEvent::ToolResult { tool: result_of, .. }] = pair {
                assert_eq!(tool, result_of, "{}: a timing comes just before its own result", database.label());
                assert!(*statements >= 1 && *statement_us > 0, "{}: {tool} ran statements: {pair:?}", database.label());
                timed.push(tool.as_str());
            }
        }
        assert_eq!(timed, vec!["table:Shelf.insert", "table:Shelf.save"], "{}: every operation is timed", database.label());

        let lineage = corvid_runtime::lineage_from_trace(&events);
        assert!(
            lineage.iter().filter(|span| span.kind == corvid_runtime::LineageKind::Db).all(|span| span.db_statement_us > 0),
            "{}: each database span carries its running time: {lineage:#?}",
            database.label()
        );
        database.finish().await;
    }
}
