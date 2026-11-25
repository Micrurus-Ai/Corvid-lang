//! B09 P11b — one database, many modules. `storage.cor` declares a
//! `public database`; `shelves.cor` and `notes.cor` each import it and
//! put a table in it with `in library`; `main.cor` reaches the rows only
//! through the agents those modules export, including from one `atomic:`
//! block that writes to both modules' tables in one transaction.

use std::path::Path;

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

fn write(dir: &Path, name: &str, source: &str) {
    std::fs::write(dir.join(name), source).expect("write module");
}

fn program(dir: &Path, database: &str) -> String {
    write(
        dir,
        "storage.cor",
        &format!("public database library:\n    engine: sqlite\n    location: \"{database}\"\n"),
    );
    write(
        dir,
        "shelves.cor",
        r#"import "./storage" use library

table StoredShelf in library:
    key shelf: Int
    title: String

public agent add_shelf(shelf: Int, title: String) -> String:
    return match StoredShelf.insert(StoredShelf(shelf, title)):
        Ok(row) -> "+" + row.title
        Err(_) -> "taken"

public agent shelf_count() -> Int:
    count = 0
    for row in StoredShelf.all():
        count = count + 1
    return count
"#,
    );
    write(
        dir,
        "notes.cor",
        r#"import "./storage" use library

table Note in library:
    key id: Int
    text: String

public agent add_note(id: Int, text: String) -> String:
    return match Note.insert(Note(id, text)):
        Ok(row) -> "+" + row.text
        Err(_) -> "taken"

public agent note_count() -> Int:
    count = 0
    for row in Note.all():
        count = count + 1
    return count
"#,
    );
    let main = r#"import "./shelves" use add_shelf, shelf_count
import "./notes" use add_note, note_count

agent both(shelf: Int, id: Int) -> String:
    added = ""
    atomic:
        added = add_shelf(shelf, "Poetry") + add_note(id, "shelved")
    return added

agent fails_after_writing(zero: Int) -> Int:
    total = 0
    atomic:
        added = add_shelf(3, "Drama") + add_note(3, "dropped")
        total = 1 / zero
    return total

agent main() -> String:
    first = add_shelf(1, "Novels") + add_note(1, "first")
    together = both(2, 2)
    return first + " | " + together + " | " + shelf_count().to_string() + " shelves, " + note_count().to_string() + " notes"
"#;
    write(dir, "main.cor", main);
    main.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tables_in_several_modules_share_one_imported_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = dir.path().join("library.sqlite");
    let database = database.to_string_lossy().replace('\\', "/");
    let source = program(dir.path(), &database);

    // The migration a planner would write for the two tables.
    let connection = rusqlite::Connection::open(&database).expect("open");
    corvid_runtime::tables::register_sqlite_collations(&connection).expect("collations");
    connection
        .execute_batch(
            "CREATE TABLE \"StoredShelf\" (\"shelf\" INTEGER PRIMARY KEY NOT NULL, \"title\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);
             CREATE TABLE \"Note\" (\"id\" INTEGER PRIMARY KEY NOT NULL, \"text\" TEXT NOT NULL, \"revision\" INTEGER NOT NULL);",
        )
        .expect("create tables");
    drop(connection);

    let ir = compile_to_ir_with_config_at_path(&source, &dir.path().join("main.cor"), None)
        .unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
    assert_eq!(out, Value::String("+Novels+first | +Poetry+shelved | 2 shelves, 2 notes".into()));

    // The imported agents' writes joined the block's transaction, so a
    // failure after them undoes both modules' rows.
    let failed = run_ir_with_runtime(&ir, Some("fails_after_writing"), vec![Value::Int(0)], &runtime).await;
    assert!(failed.is_err(), "dividing by zero fails the block");
    let rows = rusqlite::Connection::open(&database).expect("open");
    let shelves: i64 = rows.query_row("SELECT count(*) FROM \"StoredShelf\"", [], |r| r.get(0)).expect("count");
    let notes: i64 = rows.query_row("SELECT count(*) FROM \"Note\"", [], |r| r.get(0)).expect("count");
    assert_eq!((shelves, notes), (2, 2), "both modules' writes were rolled back");
}

/// An imported agent called inside an `atomic:` block is held to the same
/// rules in its own module: one that calls a tool is refused, by name.
#[test]
fn an_imported_agent_inside_an_atomic_block_is_checked_in_its_module() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "storage.cor", "public database library:\n    engine: sqlite\n    location: \"x.sqlite\"\n");
    write(
        dir.path(),
        "notes.cor",
        "import \"./storage\" use library\n\ntool notify(text: String) -> String\n\ntable Note in library:\n    key id: Int\n    text: String\n\npublic agent add_and_notify(id: Int, text: String) -> String:\n    stored = Note.insert(Note(id, text))\n    return notify(text)\n",
    );
    let source = "import \"./notes\" use add_and_notify\n\nagent main() -> String:\n    sent = \"\"\n    atomic:\n        sent = add_and_notify(1, \"hi\")\n    return sent\n";
    write(dir.path(), "main.cor", source);
    let errors = match compile_to_ir_with_config_at_path(source, &dir.path().join("main.cor"), None) {
        Ok(_) => panic!("a tool reached through an imported agent must be refused inside `atomic:`"),
        Err(errors) => format!("{errors:#?}"),
    };
    assert!(
        errors.contains("a call to tool `notify` (in imported agent `add_and_notify`, called inside the block)"),
        "{errors}"
    );
}

#[test]
fn a_private_database_cannot_be_imported() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "storage.cor", "database library:\n    engine: sqlite\n    location: \"x.sqlite\"\n");
    let source = "import \"./storage\" use library\n\ntable Note in library:\n    key id: Int\n\nagent main() -> Int:\n    return 0\n";
    write(dir.path(), "main.cor", source);
    let errors = match compile_to_ir_with_config_at_path(source, &dir.path().join("main.cor"), None) {
        Ok(_) => panic!("a private database must not be importable"),
        Err(errors) => format!("{errors:#?}"),
    };
    assert!(errors.contains("library"), "{errors}");
}
