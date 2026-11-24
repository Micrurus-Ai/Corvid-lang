//! B09 P1 and P3 — from declaration to running program: `corvid migrate
//! plan` writes the migration that creates a declared table and, as the
//! declaration changes, the migrations that change it; `corvid migrate
//! up` applies them, and `corvid run` reads the same row throughout.

use std::path::Path;
use std::process::{Command, Output};

fn corvid(args: &[&str], cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_corvid")).args(args).current_dir(cwd).output().expect("run corvid")
}

#[track_caller]
fn ok(out: &Output) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "stdout={stdout}\nstderr={}", String::from_utf8_lossy(&out.stderr));
    stdout
}

#[track_caller]
fn refused(out: &Output) -> String {
    assert!(!out.status.success(), "the plan must be refused: {}", String::from_utf8_lossy(&out.stdout));
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn source(database: &str, fields: &str, main: &str) -> String {
    format!(
        r#"
database library:
    engine: sqlite
    location: "{database}"
    tables: Shelf

table Shelf:
    key shelf: Int
{fields}

agent main() -> String:
{main}
"#
    )
}

const READ_TITLE: &str = "    return match Shelf.get(1):\n        Some(row) -> row.title + \" r\" + row.revision.to_string()\n        None -> \"absent\"";
const READ_NAME: &str = "    return match Shelf.get(1):\n        Some(row) -> row.name + \" r\" + row.revision.to_string()\n        None -> \"absent\"";

#[test]
fn planned_migrations_follow_a_table_through_its_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let database = root.join("library.sqlite").to_string_lossy().replace('\\', "/");
    std::fs::create_dir_all(root.join("src")).unwrap();
    let write = |fields: &str, main: &str| std::fs::write(root.join("src/main.cor"), source(&database, fields, main)).unwrap();
    let plan = || corvid(&["migrate", "plan", "src/main.cor"], root);
    let up = || ok(&corvid(&["migrate", "up", "--database", &database, "--state", "target/state.json"], root));
    let run = || ok(&corvid(&["run", "--target", "interp", "src/main.cor"], root));

    // Create the table and store a row.
    let first = "    title: String\n    note: Option<String>\n    legacy: String\n    count: Option<Int>";
    let insert = "    added = match Shelf.insert(Shelf(1, \"Reading\", None, \"old\", Some(3))):\n        Ok(row) -> row.revision\n        Err(row) -> 0 - row.revision\n    return \"stored r\" + added.to_string()";
    write(first, insert);
    let planned = ok(&plan());
    assert!(planned.contains("for Shelf"), "{planned}");
    let created = std::fs::read_to_string(root.join("migrations/001_create_shelf.sql")).expect("migration written");
    assert!(created.contains("CREATE TABLE \"Shelf\"") && created.contains("\"revision\" INTEGER NOT NULL"), "{created}");
    assert!(ok(&plan()).contains("already match every declared table"));
    up();
    // `corvid run` prints a String result quoted.
    assert_eq!(run().trim(), "\"stored r1\"");

    // A required column has no value for the stored row.
    write(&format!("{first}\n    owner: String"), READ_TITLE);
    let stderr = refused(&plan());
    assert!(stderr.contains("required column `owner`") && stderr.contains("as an `Option` first"), "{stderr}");

    // Dropping a column without `retired` would lose its data.
    let renamed = "    name: String renamed from title\n    note: Option<String>\n    count: Option<Int>";
    write(renamed, READ_NAME);
    let stderr = refused(&plan());
    assert!(stderr.contains("stores column `legacy`") && stderr.contains("retired legacy"), "{stderr}");

    // Declared, the rename and the drop are planned and the row keeps its value.
    write(&format!("{renamed}\n    retired legacy"), READ_NAME);
    ok(&plan());
    let change = std::fs::read_to_string(root.join("migrations/002_change_shelf.sql")).expect("change written");
    assert!(change.contains("RENAME COLUMN \"title\" TO \"name\""), "{change}");
    assert!(change.contains("DROP COLUMN \"legacy\""), "{change}");
    up();
    assert_eq!(run().trim(), "\"Reading r1\"");

    // An exact type change rebuilds the table, keeping every row.
    write("    name: String renamed from title\n    note: Option<String>\n    count: Option<Float>\n    retired legacy", READ_NAME);
    ok(&plan());
    let rebuild = std::fs::read_to_string(root.join("migrations/003_change_shelf.sql")).expect("rebuild written");
    assert!(rebuild.contains("CAST(\"count\" AS REAL)"), "{rebuild}");
    up();
    assert_eq!(run().trim(), "\"Reading r1\"");

    // A lossy change is refused.
    write("    name: Int renamed from title\n    note: Option<String>\n    count: Option<Float>\n    retired legacy", "    return \"x\"");
    let stderr = refused(&plan());
    assert!(stderr.contains("`name` changes from `String` to `Int`"), "{stderr}");
}
