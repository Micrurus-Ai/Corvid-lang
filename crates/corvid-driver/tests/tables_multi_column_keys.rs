//! B09 P7b — a table keyed by two columns, on SQLite and, when
//! `CORVID_TEST_POSTGRES_URL` is set, on Postgres: rows sharing one key
//! column stay apart, `get` takes both values, a repeated key is an
//! insert conflict, saves and deletes are revision-checked on the whole
//! key, and queries come back in key order.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database reviews:
{settings}
    tables: Edit

table Edit:
    key proposal: String
    key shelf: Int
    title: String

agent put(row: Edit) -> String:
    return match Edit.insert(row):
        Ok(stored) -> "added " + stored.proposal + "/" + stored.shelf.to_string()
        Err(existing) -> "taken by " + existing.title

agent find(proposal: String, shelf: Int) -> String:
    return match Edit.get(proposal, shelf):
        Some(edit) -> edit.title + " r" + edit.revision.to_string()
        None -> "none"

agent listed(proposal: String) -> String:
    out = ""
    for edit in Edit.where(fn (e) -> e.proposal == proposal):
        out = out + edit.shelf.to_string() + ":" + edit.title + ","
    return out

agent rename(edit: Edit, title: String) -> String:
    return match Edit.save(Edit {{ title: title, ..edit }}):
        Ok(saved) -> "saved r" + saved.revision.to_string()
        Err(_) -> "conflict"

agent main() -> String:
    a = put(Edit("p1", 2, "Persuasion"))
    b = put(Edit("p1", 1, "Odyssey"))
    c = put(Edit("p2", 1, "Emma"))
    again = put(Edit("p1", 1, "Other"))
    found = find("p1", 1) + " | " + find("p2", 2)
    renamed = rename(Edit("p1", 1, "Odyssey", 1), "Iliad")
    stale = rename(Edit("p1", 1, "Odyssey", 1), "Lost")
    gone = match Edit.delete(Edit("p2", 1, "Emma", 1)):
        Ok(row) -> "deleted " + row.title
        Err(_) -> "kept"
    return a + " | " + b + " | " + c + " | " + again + " | " + found + " | " + renamed + " | " + stale + " | " + gone + " | " + listed("p1") + " | " + find("p2", 1)
"#
    )
}

const EXPECTED: &str = "added p1/2 | added p1/1 | added p2/1 | taken by Odyssey | Odyssey r1 | none | saved r2 | conflict | deleted Emma | 1:Iliad,2:Persuasion, | none";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_table_keyed_by_two_columns_reads_and_writes_by_both() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        assert_eq!(out, Value::String(EXPECTED.into()), "on {}", database.label());
        database.finish().await;
    }
}

#[test]
fn a_key_of_two_columns_is_created_as_one_primary_key() {
    let schemas = corvid_driver::table_schemas(&program("    engine: sqlite\n    location: \"reviews.sqlite\"")).expect("compiles");
    let sql = corvid_runtime::tables::create_table_sql(&schemas[0]);
    assert!(sql.contains("PRIMARY KEY (\"proposal\", \"shelf\")"), "{sql}");
    assert!(sql.contains("\"proposal\" TEXT NOT NULL,"), "{sql}");
}
