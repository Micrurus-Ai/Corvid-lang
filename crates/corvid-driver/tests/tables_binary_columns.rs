//! B09 P8c — `Bytes` as a binary table column, on SQLite and, when
//! `CORVID_TEST_POSTGRES_URL` is set, on Postgres: the bytes are stored
//! as themselves (`BLOB`, `bytea`), read back exactly, found by equality
//! (the lookup by a token's hash that API keys need), and missing in an
//! optional column. Raw `std.db` queries still report a blob as redacted.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database keys:
{settings}

table ApiKey:
    key name: String
    token_hash: Bytes
    previous: Option<Bytes>

agent issue(name: String, token: String) -> String:
    return match ApiKey.insert(ApiKey(name, token.utf8().sha256(), None)):
        Ok(row) -> "+" + row.name
        Err(_) -> "taken"

agent owner_of(token: String) -> String:
    digest = token.utf8().sha256()
    found = ApiKey.where(fn (k) -> k.token_hash == digest)
    out = ""
    for key in found:
        out = out + key.name + ","
    return out

agent rotate(name: String, token: String) -> String:
    return match ApiKey.get(name):
        Some(key) -> rotated(ApiKey.save(ApiKey {{ token_hash: token.utf8().sha256(), previous: Some(key.token_hash), ..key }}))
        None -> "none"

fn rotated(outcome: Result<ApiKey, Option<ApiKey>>) -> String:
    return match outcome:
        Ok(key) -> "rotated r" + key.revision.to_string()
        Err(_) -> "conflict"

agent read_back(name: String, token: String) -> String:
    return match ApiKey.get(name):
        Some(key) -> key.token_hash.to_hex().length().to_string() + " " + (key.token_hash == token.utf8().sha256()).to_string() + " " + (key.previous == None).to_string()
        None -> "none"

agent main() -> String:
    added = issue("ana", "tok-ana") + issue("bo", "tok-bo")
    before = read_back("bo", "tok-bo")
    again = rotate("bo", "tok-bo-2")
    after = read_back("bo", "tok-bo-2")
    return added + " | " + owner_of("tok-ana") + " | " + owner_of("tok-bo") + " | " + owner_of("tok-bo-2") + " | " + before + " | " + again + " | " + after
"#
    )
}

const EXPECTED: &str = "+ana+bo | ana, |  | bo, | 64 true true | rotated r2 | 64 true false";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bytes_columns_store_find_and_read_back_exactly() {
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
fn bytes_columns_are_blob_and_bytea() {
    for (settings, column) in [
        ("    engine: sqlite\n    location: \"keys.sqlite\"", "\"token_hash\" BLOB NOT NULL"),
        ("    engine: postgres\n    location: env(\"KEYS_URL\")\n    tls: required\n    connections: 4", "\"token_hash\" bytea NOT NULL"),
    ] {
        let source = program(settings);
        let schemas = corvid_driver::table_schemas(&source, std::path::Path::new("keys.cor")).expect("compiles");
        let engine = schemas[0].engine;
        let sql = corvid_runtime::tables::create_table_sql_on(&schemas[0], engine);
        assert!(sql.contains(column), "missing {column} in {sql}");
    }
}

#[test]
fn bytes_have_no_order() {
    let source = format!(
        "{}\nagent q() -> List<ApiKey>:\n    return ApiKey.all().order_by(fn (k) -> k.token_hash)\n",
        program("    engine: sqlite\n    location: \"keys.sqlite\"")
    );
    let errors = match compile_to_ir(&source) {
        Ok(_) => panic!("ordering by bytes must be refused"),
        Err(errors) => format!("{errors:#?}"),
    };
    assert!(errors.contains("`order_by` reads column `token_hash`, a `Bytes`, which has no order"), "{errors}");
}

#[test]
fn raw_queries_still_redact_a_blob() {
    let db = corvid_runtime::db::SqliteDbRuntime::open_in_memory().expect("sqlite");
    db.execute("CREATE TABLE k (h BLOB)", &[]).expect("create");
    db.execute("INSERT INTO k VALUES (?1)", &[corvid_runtime::db::DbValue::Bytes(vec![1, 2, 3])]).expect("insert");
    let rows = db.query("SELECT h FROM k", &[]).expect("query");
    let cell = &rows.rows[0]["h"];
    assert_eq!(cell.value, corvid_runtime::db::DbValue::Text("<blob:redacted>".into()));
}
