//! B09 P1 — from declaration to running program: `corvid migrate plan`
//! writes the migration that creates a declared table, `corvid migrate
//! up` applies it, and `corvid run` reads and writes the table.

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

fn source(database: &str, extra_field: &str, extra_arg: &str) -> String {
    format!(
        r#"
database library:
    engine: sqlite
    location: "{database}"
    tables: Shelf

table Shelf:
    key shelf: Int
    title: String
    note: Option<String>{extra_field}

agent main() -> String:
    added = match Shelf.insert(Shelf(1, "Reading", None{extra_arg})):
        Ok(row) -> row.revision
        Err(row) -> 0 - row.revision
    return match Shelf.get(1):
        Some(row) -> row.title + " r" + added.to_string()
        None -> "absent"
"#
    )
}

#[test]
fn a_planned_migration_creates_the_table_a_program_then_uses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let database = root.join("library.sqlite").to_string_lossy().replace('\\', "/");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.cor"), source(&database, "", "")).unwrap();

    let planned = ok(&corvid(&["migrate", "plan", "src/main.cor"], root));
    assert!(planned.contains("creating Shelf"), "{planned}");
    let migration = std::fs::read_to_string(root.join("migrations/001_create_shelf.sql")).expect("migration written");
    assert!(migration.contains("CREATE TABLE \"Shelf\""), "{migration}");
    assert!(migration.contains("\"revision\" INTEGER NOT NULL"), "{migration}");

    let again = ok(&corvid(&["migrate", "plan", "src/main.cor"], root));
    assert!(again.contains("already create every declared table"), "{again}");

    ok(&corvid(&["migrate", "up", "--database", &database, "--state", "target/state.json"], root));
    let run = ok(&corvid(&["run", "--target", "interp", "src/main.cor"], root));
    // `corvid run` prints a String result quoted.
    assert_eq!(run.trim(), "\"Reading r1\"");

    // A changed declaration is refused until P3 plans changes.
    std::fs::write(root.join("src/main.cor"), source(&database, "\n    owner: String", ", \"ana\"")).unwrap();
    let changed = corvid(&["migrate", "plan", "src/main.cor"], root);
    assert!(!changed.status.success());
    let stderr = String::from_utf8_lossy(&changed.stderr);
    assert!(stderr.contains("owner") && stderr.contains("B09 P3"), "{stderr}");
}
