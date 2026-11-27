//! Lineage from real runs (L1). `corvid run` runs a program, which records
//! its trace, and `corvid trace lineage`, `corvid observe list` and
//! `corvid observe show` read that trace through its derived lineage. No
//! fixture is involved: every span comes from the program running.

use std::path::{Path, PathBuf};
use std::process::Command;

const PROGRAM: &str = r#"
database library:
    engine: sqlite
    location: "{database}"

table Shelf:
    key shelf: Int
    title: String

agent shelve(id: Int, title: String) -> String:
    return match Shelf.insert(Shelf(id, title)):
        Ok(stored) -> stored.title
        Err(Taken(holder)) -> "taken by " + holder.title

agent main() -> String:
    first = shelve(1, "Reading")
    again = shelve(1, "Other")
    found = match Shelf.get(1):
        Some(row) -> row.title
        None -> "absent"
    return first + " | " + again + " | " + found
"#;

fn corvid(args: &[&str], cwd: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_corvid")).args(args).current_dir(cwd).output().expect("run corvid");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "`corvid {}` failed\nstdout:\n{stdout}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// A project whose program has run once with `corvid run`; its trace
/// directory and the run's id.
fn project_after_one_run() -> (tempfile::TempDir, PathBuf, String) {
    let project = tempfile::tempdir().expect("tempdir");
    let database = project.path().join("library.sqlite");
    let source = PROGRAM.replace("{database}", &database.to_string_lossy().replace('\\', "/"));
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.cor"), &source).unwrap();
    let tables = corvid_driver::table_schemas(&source, Path::new("main.cor")).unwrap_or_else(|e| panic!("must compile: {e:#?}"));
    let sql = tables
        .iter()
        .map(|table| corvid_runtime::tables::create_table_sql_on(table, corvid_runtime::tables::TableEngine::Sqlite))
        .collect::<Vec<_>>()
        .join("\n");
    let connection = rusqlite::Connection::open(&database).expect("open");
    corvid_runtime::tables::register_sqlite_collations(&connection).expect("collations");
    connection.execute_batch(&sql).expect("create tables");
    drop(connection);

    let out = corvid(&["run", "src/main.cor"], project.path());
    assert!(out.contains("Reading | taken by Reading | Reading"), "the program ran: {out}");
    let trace_dir = project.path().join("target/trace");
    let traces: Vec<PathBuf> = std::fs::read_dir(&trace_dir)
        .expect("corvid run records a trace")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    assert_eq!(traces.len(), 1, "one run, one trace: {traces:?}");
    let run_id = traces[0].file_stem().unwrap().to_string_lossy().into_owned();
    (project, trace_dir, run_id)
}

#[test]
fn a_real_run_renders_as_a_lineage_tree_with_its_database_steps() {
    let (project, trace_dir, run_id) = project_after_one_run();
    let tree = corvid(&["trace", "lineage", &run_id, "--trace-dir", trace_dir.to_str().unwrap()], project.path());
    let lines: Vec<&str> = tree.lines().map(str::trim_start).collect();
    assert!(!tree.contains("incomplete"), "a real run's lineage validates:\n{tree}");
    assert!(lines[0].contains("main") && lines[0].contains("[ok]"), "the run is the root:\n{tree}");
    let steps: Vec<&str> = lines[1..].iter().map(|line| line.split(" [").next().unwrap()).collect();
    assert_eq!(
        steps,
        vec!["- db Shelf.insert", "- db Shelf.insert", "- db Shelf.get"],
        "each table operation is a database step, in the order it ran:\n{tree}"
    );
    assert!(tree.lines().skip(1).all(|line| line.starts_with("  ")), "the steps sit under the run:\n{tree}");
    assert!(
        lines[1..].iter().all(|line| line.contains(" waited=") && line.contains(" ran=")),
        "each database step shows its wait for a turn and its running time (L2):\n{tree}"
    );
}

/// The timing a run records is evidence, not a result: the same trace
/// replays with the database gone.
#[test]
fn a_run_with_database_timing_still_replays_without_the_database() {
    let (project, trace_dir, run_id) = project_after_one_run();
    let trace = trace_dir.join(format!("{run_id}.jsonl"));
    let text = std::fs::read_to_string(&trace).unwrap();
    assert_eq!(text.matches("\"kind\":\"db_timing\"").count(), 3, "one timing per table operation:\n{text}");
    std::fs::remove_file(project.path().join("library.sqlite")).unwrap();
    let replayed = corvid(&["replay", trace.to_str().unwrap(), "--source", "src/main.cor"], project.path());
    assert!(replayed.contains("Reading | taken by Reading | Reading"), "replay serves the recorded rows:\n{replayed}");
}

#[test]
fn observe_lists_and_shows_a_real_run() {
    let (project, trace_dir, run_id) = project_after_one_run();
    let dir = trace_dir.to_str().unwrap();
    let list = corvid(&["observe", "list", "--trace-dir", dir], project.path());
    assert!(!list.contains("no lineage traces found"), "observe finds the run:\n{list}");
    assert!(list.contains(&run_id) && list.contains("main"), "observe names the run and its agent:\n{list}");
    let show = corvid(&["observe", "show", &run_id, "--trace-dir", dir], project.path());
    assert!(show.contains("Shelf.insert") && show.contains("Shelf.get"), "observe shows the run's steps:\n{show}");
}
