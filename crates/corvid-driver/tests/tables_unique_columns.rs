//! B09 P10 — `unique` rules on SQLite and, when `CORVID_TEST_POSTGRES_URL`
//! is set, on Postgres. An insert that would repeat a unique value is a
//! conflict holding the row that has it; a save that would take another
//! row's value changes nothing and holds that row, as distinct from a save
//! that lost a race on its own row. A rule over two columns lets either
//! repeat alone, and a missing value never conflicts.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database site:
{settings}

table Member:
    key id: Int
    email: String
    nickname: Option<String>
    unique email
    unique nickname

table Page:
    key id: Int
    tenant: String
    slug: String
    unique tenant, slug

agent join(id: Int, email: String, nickname: Option<String>) -> String:
    return match Member.insert(Member(id, email, nickname)):
        Ok(member) -> "+" + member.id.to_string()
        Err(Taken(holder)) -> " taken by " + holder.id.to_string()

agent rename(id: Int, email: String) -> String:
    return match Member.get(id):
        Some(member) -> outcome(id, Member.save(Member {{ email: email, ..member }}))
        None -> " none"

fn outcome(id: Int, saved: Result<Member, Conflict<Member>>) -> String:
    return match saved:
        Ok(member) -> " now " + member.email
        Err(Changed(current)) -> " changed meanwhile"
        Err(Deleted) -> " gone"
        Err(Taken(holder)) -> " held by " + holder.id.to_string()

agent publish(id: Int, tenant: String, slug: String) -> String:
    return match Page.insert(Page(id, tenant, slug)):
        Ok(page) -> "+" + page.id.to_string()
        Err(Taken(holder)) -> " taken by " + holder.id.to_string()

agent main() -> String:
    members = join(1, "ana@example.com", Some("ana")) + join(2, "bo@example.com", None) + join(3, "ana@example.com", None) + join(4, "cy@example.com", Some("ana")) + join(5, "di@example.com", None)
    renamed = rename(2, "ana@example.com") + rename(2, "bo2@example.com")
    stale = outcome(2, Member.save(Member(2, "old@example.com", None, 1)))
    pages = publish(1, "acme", "home") + publish(2, "beta", "home") + publish(3, "acme", "about") + publish(4, "acme", "home")
    return members + " |" + renamed + stale + " | " + pages
"#
    )
}

// Member 5 has no nickname, like member 2: a missing value never conflicts.
const EXPECTED: &str = "+1+2 taken by 1 taken by 1+5 | held by 1 now bo2@example.com changed meanwhile | +1+2+3 taken by 1";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unique_rules_turn_repeated_values_into_typed_conflicts() {
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

fn errors(table: &str) -> String {
    let source = format!("database site:\n    engine: sqlite\n    location: \"site.sqlite\"\n\n{table}\nagent main() -> Int:\n    return 0\n");
    match compile_to_ir(&source) {
        Ok(_) => panic!("must be refused:\n{source}"),
        Err(errors) => format!("{errors:#?}"),
    }
}

#[test]
fn a_unique_line_names_real_comparable_columns_once() {
    let base = "table Member:\n    key id: Int\n    email: String\n    tags: List<String>\n";
    for (line, expected) in [
        ("    unique mail\n", "`mail` is not a column of the table"),
        ("    unique tags\n", "`tags` is stored as JSON"),
        ("    unique email, email\n", "`email` is named twice"),
        ("    unique id\n", "the key's columns are already unique"),
        ("    unique email\n    unique email\n", "the same columns are already unique on another line"),
    ] {
        let refused = errors(&format!("{base}{line}"));
        assert!(refused.contains(expected), "{line}: {refused}");
    }
}

#[test]
fn a_field_may_still_be_named_unique() {
    let source = "database site:\n    engine: sqlite\n    location: \"site.sqlite\"\n\ntable Flag:\n    key id: Int\n    unique: Bool\n\nagent main() -> Int:\n    return 0\n";
    compile_to_ir(source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
}
