//! B09 P1, P3 and P4b — from declaration to running program: `corvid
//! migrate plan` writes the migration that creates a declared table and,
//! as the declaration changes, the migrations that change it; `corvid
//! migrate up` applies them, and `corvid run` reads the same row
//! throughout. The same steps run on SQLite and, when
//! `CORVID_TEST_POSTGRES_URL` and `CORVID_TEST_POSTGRES_SCRATCH_URL` are
//! set, on Postgres, where the planner works against the scratch
//! database and leaves it empty.

use std::path::Path;
use std::process::{Command, Output};

const SCRATCH_ENV: &str = "CORVID_TEST_POSTGRES_SCRATCH_URL";

/// The engine one run of the lifecycle uses.
enum Engine {
    Sqlite { database: String },
    Postgres { admin: postgres::Client, env: String, url: String },
}

impl Engine {
    fn settings(&self) -> String {
        match self {
            Engine::Sqlite { database } => format!("    engine: sqlite\n    location: \"{database}\""),
            Engine::Postgres { env, .. } => format!("    engine: postgres\n    location: env(\"{env}\")\n    tls: disabled\n    connections: 4"),
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Engine::Sqlite { .. } => "sqlite",
            Engine::Postgres { .. } => "postgres",
        }
    }
}

fn corvid(args: &[&str], cwd: &Path, engine: &Engine) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_corvid"));
    command.args(args).current_dir(cwd);
    if let Engine::Postgres { env, url, .. } = engine {
        command.env(env, url);
    }
    command.output().expect("run corvid")
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

fn source(settings: &str, fields: &str, main: &str) -> String {
    format!(
        r#"
database library:
{settings}

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

fn lifecycle(engine: &Engine) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let write = |fields: &str, main: &str| std::fs::write(root.join("src/main.cor"), source(&engine.settings(), fields, main)).unwrap();
    let plan = || match engine {
        Engine::Sqlite { .. } => corvid(&["migrate", "plan", "src/main.cor"], root, engine),
        Engine::Postgres { .. } => corvid(&["migrate", "plan", "src/main.cor", "--scratch", SCRATCH_ENV], root, engine),
    };
    let up = || match engine {
        Engine::Sqlite { database } => ok(&corvid(&["migrate", "up", "--database", database, "--state", "target/state.json"], root, engine)),
        Engine::Postgres { .. } => ok(&corvid(&["migrate", "up", "--program", "src/main.cor", "--state", "target/state.json"], root, engine)),
    };
    let run = || ok(&corvid(&["run", "--target", "interp", "src/main.cor"], root, engine));
    let on = engine.label();

    // Create the table and store a row.
    let first = "    title: String\n    note: Option<String>\n    legacy: String\n    count: Option<Int>";
    let insert = "    added = match Shelf.insert(Shelf(1, \"Reading\", None, \"old\", Some(3))):\n        Ok(row) -> row.revision\n        Err(row) -> 0 - row.revision\n    return \"stored r\" + added.to_string()";
    write(first, insert);
    let planned = ok(&plan());
    assert!(planned.contains("for Shelf"), "on {on}: {planned}");
    let created = std::fs::read_to_string(root.join("migrations/001_create_shelf.sql")).expect("migration written");
    assert!(created.contains("CREATE TABLE \"Shelf\""), "on {on}: {created}");
    assert!(ok(&plan()).contains("already match every declared table"), "on {on}");
    up();
    // `corvid run` prints a String result quoted.
    assert_eq!(run().trim(), "\"stored r1\"", "on {on}");

    // A required column has no value for the stored row.
    write(&format!("{first}\n    owner: String"), READ_TITLE);
    let stderr = refused(&plan());
    assert!(stderr.contains("required column `owner`") && stderr.contains("as an `Option` first"), "on {on}: {stderr}");

    // Dropping a column without `retired` would lose its data.
    let renamed = "    name: String renamed from title\n    note: Option<String>\n    count: Option<Int>";
    write(renamed, READ_NAME);
    let stderr = refused(&plan());
    assert!(stderr.contains("stores column `legacy`") && stderr.contains("retired legacy"), "on {on}: {stderr}");

    // Declared, the rename and the drop are planned and the row keeps its value.
    write(&format!("{renamed}\n    retired legacy"), READ_NAME);
    ok(&plan());
    let change = std::fs::read_to_string(root.join("migrations/002_change_shelf.sql")).expect("change written");
    assert!(change.contains("RENAME COLUMN \"title\" TO \"name\""), "on {on}: {change}");
    assert!(change.contains("DROP COLUMN \"legacy\""), "on {on}: {change}");
    up();
    assert_eq!(run().trim(), "\"Reading r1\"", "on {on}");

    // An exact type change keeps every row: SQLite rebuilds the table,
    // Postgres alters the column.
    write("    name: String renamed from title\n    note: Option<String>\n    count: Option<Float>\n    retired legacy", READ_NAME);
    ok(&plan());
    let widened = std::fs::read_to_string(root.join("migrations/003_change_shelf.sql")).expect("change written");
    let expected = match engine {
        Engine::Sqlite { .. } => "CAST(\"count\" AS REAL)",
        Engine::Postgres { .. } => "ALTER COLUMN \"count\" TYPE double precision",
    };
    assert!(widened.contains(expected), "on {on}: {widened}");
    up();
    assert_eq!(run().trim(), "\"Reading r1\"", "on {on}");

    // A lossy change is refused.
    write("    name: Int renamed from title\n    note: Option<String>\n    count: Option<Float>\n    retired legacy", "    return \"x\"");
    let stderr = refused(&plan());
    assert!(stderr.contains("`name` changes from `String` to `Int`"), "on {on}: {stderr}");

    // A money column is added (on Postgres, its domain first), filled, and
    // compared by value, and the planner then reads the stored column
    // back as the same type.
    let priced = "    name: String renamed from title\n    note: Option<String>\n    count: Option<Float>\n    price: Option<Decimal>\n    retired legacy";
    let fill = "    stored: Decimal = 10.0\n    ten: Decimal = 10\n    for row in Shelf.all():\n        saved = Shelf.save(Shelf { price: Some(stored), ..row })\n    out = \"\"\n    for row in Shelf.where(fn (s) -> s.price == Some(ten)):\n        out = out + row.name + \" \" + row.price.unwrap_or(ten).to_string()\n    return out";
    write(priced, fill);
    ok(&plan());
    let added = std::fs::read_to_string(root.join("migrations/004_change_shelf.sql")).expect("change written");
    let expected = match engine {
        Engine::Sqlite { .. } => "ADD COLUMN \"price\" DECIMAL TEXT COLLATE corvid_decimal",
        Engine::Postgres { .. } => "CREATE DOMAIN corvid_decimal AS numeric",
    };
    assert!(added.contains(expected), "on {on}: {added}");
    up();
    assert_eq!(run().trim(), "\"Reading 10.0\"", "on {on}");
    assert!(ok(&plan()).contains("already match every declared table"), "on {on}");
}

#[test]
fn planned_migrations_follow_a_table_through_its_changes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let database = dir.path().join("library.sqlite").to_string_lossy().replace('\\', "/");
    lifecycle(&Engine::Sqlite { database });
}

/// The planner compiles the program as `corvid check` does, so a module
/// that imports the standard library and a sibling module plans its
/// tables instead of being refused for imports it cannot resolve.
#[test]
fn planning_resolves_the_programs_imports() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("labels.cor"), "public type Label:\n    text: String\n").unwrap();
    std::fs::write(
        src.join("store.cor"),
        r#"import "std:ui" as ui
import "./labels" use Label

database library:
    engine: sqlite
    location: "library.sqlite"

table Shelf:
    key shelf: Int
    title: String

public agent heading(label: Label) -> String:
    return ui.text(label.text).html
"#,
    )
    .unwrap();
    let engine = Engine::Sqlite { database: "library.sqlite".into() };
    ok(&corvid(&["check", "src/store.cor"], dir.path(), &engine));
    let planned = ok(&corvid(&["migrate", "plan", "src/store.cor", "--dry-run"], dir.path(), &engine));
    assert!(planned.contains("CREATE TABLE \"Shelf\""), "{planned}");
}

/// Indexes follow the queries (B09 P9): the planner creates the index a
/// query needs, replaces it when the query changes, drops it when no
/// query needs it, and SQLite's own query plan uses it.
#[test]
fn indexes_follow_the_queries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let engine = Engine::Sqlite { database: "jobs.sqlite".into() };
    let write = |query: &str| {
        std::fs::write(
            root.join("src/main.cor"),
            format!(
                "database work:\n    engine: sqlite\n    location: \"jobs.sqlite\"\n\ntable Job:\n    key job: Int\n    owner: String\n    paused: Bool\n    budget: Int\n\nagent latest(owner: String) -> List<Job>:\n{query}\n\nagent main() -> Int:\n    return 0\n"
            ),
        )
        .unwrap()
    };
    let plan = || ok(&corvid(&["migrate", "plan", "src/main.cor"], root, &engine));
    let migration = |n: &str| {
        let path = std::fs::read_dir(root.join("migrations")).unwrap().map(|e| e.unwrap().path()).find(|p| p.file_name().unwrap().to_string_lossy().starts_with(n)).unwrap();
        std::fs::read_to_string(path).unwrap()
    };

    write("    return Job.where(fn (j) -> j.owner == owner and not j.paused).order_by(fn (j) -> j.job).descending().take(1)");
    plan();
    let first = migration("001");
    assert!(first.contains("CREATE INDEX IF NOT EXISTS \"Job_by_owner_job\" ON \"Job\" (\"owner\", \"job\");"), "{first}");
    assert!(first.contains("for the queries in `latest`"), "{first}");
    assert!(plan().contains("already match every declared table"));
    ok(&corvid(&["migrate", "up", "--dir", "migrations", "--database", "jobs.sqlite"], root, &engine));
    let db = rusqlite::Connection::open(root.join("jobs.sqlite")).unwrap();
    let detail: String = db
        .query_row("EXPLAIN QUERY PLAN SELECT * FROM \"Job\" WHERE \"owner\" = 'ana' ORDER BY \"Job\".\"job\" DESC", [], |row| row.get(3))
        .unwrap();
    assert!(detail.contains("Job_by_owner_job"), "{detail}");

    // The query changes, so its index does.
    write("    return Job.where(fn (j) -> j.owner == owner and j.budget >= 3)");
    plan();
    let second = migration("002");
    assert!(second.contains("DROP INDEX IF EXISTS \"Job_by_owner_job\";"), "{second}");
    assert!(second.contains("CREATE INDEX IF NOT EXISTS \"Job_by_owner_budget\""), "{second}");

    // A query by key needs no index of its own.
    write("    return Job.where(fn (j) -> j.job == 1)");
    plan();
    let third = migration("003");
    assert!(third.contains("DROP INDEX IF EXISTS \"Job_by_owner_budget\";") && !third.contains("CREATE INDEX"), "{third}");
    assert!(plan().contains("already match every declared table"));
}

/// One database shared by import (B09 P11): planning the program from
/// its entry point writes one migration for the tables of every module
/// that lives in the database, and the program then runs against it.
#[test]
fn tables_from_several_modules_are_planned_into_one_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let engine = Engine::Sqlite { database: "library.sqlite".into() };
    std::fs::write(root.join("src/storage.cor"), "public database library:\n    engine: sqlite\n    location: \"library.sqlite\"\n").unwrap();
    std::fs::write(
        root.join("src/shelves.cor"),
        "import \"./storage\" use library\n\ntable Shelf in library:\n    key id: Int\n    title: String\n\npublic agent shelve(id: Int, title: String) -> Int:\n    return match Shelf.insert(Shelf(id, title)):\n        Ok(row) -> row.id\n        Err(_) -> 0\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/notes.cor"),
        "import \"./storage\" use library\n\ntable Note in library:\n    key id: Int\n    owner: String\n\npublic agent note(id: Int, owner: String) -> Int:\n    return match Note.insert(Note(id, owner)):\n        Ok(row) -> row.id\n        Err(_) -> 0\n\npublic agent count_for(owner: String) -> Int:\n    count = 0\n    for row in Note.where(fn (n) -> n.owner == owner):\n        count = count + 1\n    return count\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.cor"),
        "import \"./shelves\" use shelve\nimport \"./notes\" use note, count_for\n\nagent main() -> Int:\n    a = shelve(1, \"Novels\")\n    b = note(1, \"ana\")\n    c = note(2, \"ana\")\n    return a + b + c + count_for(\"ana\")\n",
    )
    .unwrap();
    let planned = ok(&corvid(&["migrate", "plan", "src/main.cor"], root, &engine));
    assert!(planned.contains("plan: wrote") && planned.contains("Note") && planned.contains("Shelf"), "{planned}");
    let migration = std::fs::read_dir(root.join("migrations")).unwrap().next().unwrap().unwrap().path();
    let sql = std::fs::read_to_string(migration).unwrap();
    assert!(sql.contains("CREATE TABLE \"Shelf\"") && sql.contains("CREATE TABLE \"Note\""), "{sql}");
    // The query in another module still gets its index.
    assert!(sql.contains("\"Note_by_owner\""), "{sql}");
    assert!(ok(&corvid(&["migrate", "plan", "src/main.cor"], root, &engine)).contains("already match every declared table"));
    ok(&corvid(&["migrate", "up", "--dir", "migrations", "--database", "library.sqlite"], root, &engine));
    assert_eq!(ok(&corvid(&["run", "--target", "interp", "src/main.cor"], root, &engine)).trim(), "6");
}

/// A `unique` line is planned as a unique index, and adding one to a
/// table whose rows already repeat the value fails `migrate up` by name
/// rather than picking a row to keep (B09 P10).
#[test]
fn a_unique_line_is_planned_and_refused_over_repeated_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let engine = Engine::Sqlite { database: "site.sqlite".into() };
    let write = |extra: &str| {
        std::fs::write(
            root.join("src/main.cor"),
            format!("database site:\n    engine: sqlite\n    location: \"site.sqlite\"\n\ntable Member:\n    key id: Int\n    email: String\n{extra}\nagent main() -> Int:\n    a = Member.insert(Member(1, \"ana@example.com\"))\n    b = Member.insert(Member(2, \"ana@example.com\"))\n    return 0\n"),
        )
        .unwrap()
    };
    let up = || corvid(&["migrate", "up", "--dir", "migrations", "--database", "site.sqlite"], root, &engine);
    write("");
    ok(&corvid(&["migrate", "plan", "src/main.cor"], root, &engine));
    ok(&up());
    ok(&corvid(&["run", "--target", "interp", "src/main.cor"], root, &engine));

    write("    unique email\n");
    ok(&corvid(&["migrate", "plan", "src/main.cor"], root, &engine));
    let path = std::fs::read_dir(root.join("migrations")).unwrap().map(|e| e.unwrap().path()).find(|p| p.to_string_lossy().contains("002")).unwrap();
    let sql = std::fs::read_to_string(path).unwrap();
    assert!(sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS \"Member_unique_email\" ON \"Member\" (\"email\");"), "{sql}");
    let failed = up();
    assert!(!failed.status.success(), "a unique index over repeated rows must not apply");
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("UNIQUE"), "{stderr}");
}

/// A decimal column goes from declaration to a running query through the
/// CLI: the planner writes its collation, `migrate up` creates the table
/// on a connection that knows it, and `corvid run` compares by value.
#[test]
fn a_decimal_column_is_planned_applied_and_queried_by_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.cor"),
        r#"database ledger:
    engine: sqlite
    location: "ledger.sqlite"

table Price:
    key item: String
    amount: Decimal

agent main() -> String:
    a = Price.insert(Price("tea", 9.50))
    b = Price.insert(Price("cake", 10.0))
    out = ""
    for row in Price.where(fn (p) -> p.amount >= 9.5).order_by(fn (p) -> p.amount).descending():
        out = out + row.item + " " + row.amount.to_string() + ","
    return out
"#,
    )
    .unwrap();
    let engine = Engine::Sqlite { database: "ledger.sqlite".into() };
    let planned = ok(&corvid(&["migrate", "plan", "src/main.cor"], dir.path(), &engine));
    assert!(planned.contains("plan: wrote"), "{planned}");
    let migration = std::fs::read_dir(dir.path().join("migrations")).unwrap().next().unwrap().unwrap().path();
    let sql = std::fs::read_to_string(migration).unwrap();
    assert!(sql.contains("\"amount\" DECIMAL TEXT COLLATE corvid_decimal NOT NULL"), "{sql}");
    ok(&corvid(&["migrate", "up", "--dir", "migrations", "--database", "ledger.sqlite"], dir.path(), &engine));
    let out = ok(&corvid(&["run", "--target", "interp", "src/main.cor"], dir.path(), &engine));
    assert!(out.contains("cake 10.0,tea 9.50,"), "{out}");
}

#[test]
fn planned_migrations_follow_a_postgres_table_through_its_changes() {
    let (Ok(base), Ok(scratch)) = (std::env::var("CORVID_TEST_POSTGRES_URL"), std::env::var(SCRATCH_ENV)) else {
        eprintln!("skipping: set CORVID_TEST_POSTGRES_URL and {SCRATCH_ENV} to run the migration lifecycle on Postgres");
        return;
    };
    let mut admin = postgres::Client::connect(&base, postgres::NoTls).expect("connect to the test database");
    let schema = format!("corvid_migrate_test_{}", std::process::id());
    admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}")).unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{separator}options=-csearch_path%3D{schema}");
    let engine = Engine::Postgres { admin, env: "CORVID_TEST_LIBRARY_URL".into(), url };
    lifecycle(&engine);

    let Engine::Postgres { mut admin, .. } = engine else { unreachable!() };
    let mut scratch = postgres::Client::connect(&scratch, postgres::NoTls).expect("connect to the scratch database");
    let leftover: i64 = scratch
        .query_one("SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog', 'information_schema')", &[])
        .unwrap()
        .get(0);
    assert_eq!(leftover, 0, "planning leaves the scratch database empty");

    // A scratch database that holds data is refused, so the planner can
    // never be pointed at a real database by mistake.
    scratch.batch_execute("CREATE TABLE corvid_migrate_test_data (id int)").unwrap();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let settings = "    engine: postgres\n    location: env(\"CORVID_TEST_LIBRARY_URL\")\n    tls: disabled\n    connections: 4";
    std::fs::write(dir.path().join("src/main.cor"), source(settings, "    title: String", "    return \"x\"")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["migrate", "plan", "src/main.cor", "--scratch", SCRATCH_ENV])
        .current_dir(dir.path())
        .output()
        .expect("run corvid");
    scratch.batch_execute("DROP TABLE corvid_migrate_test_data").unwrap();
    let stderr = refused(&out);
    assert!(stderr.contains("is not empty"), "{stderr}");
    admin.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).unwrap();
}
