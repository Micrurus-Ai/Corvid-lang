//! Encryption at rest K2 and K3: a `Secret` column in a database that
//! declares `encryption: key(secret("NAME"))` is stored sealed to its
//! table, column and row, on SQLite and, when `CORVID_TEST_POSTGRES_URL`
//! is set, on Postgres. The database and the trace hold only ciphertext;
//! a value copied to another row does not open; a replay needs the key.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

/// Each test names its own key secret: tests run in parallel and share
/// the environment.
const KEY_SECRET: &str = "CORVID_TEST_BILLING_DATA_KEY";
const PLAINTEXT: &str = "sk_live_corvid_ada_51Hx";

fn program(settings: &str) -> String {
    program_with(settings, KEY_SECRET)
}

fn program_with(settings: &str, key_secret: &str) -> String {
    format!(
        r#"
database billing:
{settings}
    encryption: key(secret("{key_secret}"))

table Customer:
    key id: Int
    name: String
    stripe_key: Secret
    backup_key: Option<Secret>

agent store(id: Int, key: Secret) -> Int:
    return match Customer.insert(Customer(id, "Ada", key, None)):
        Ok(_) -> 1
        Err(_) -> 0

agent holds(id: Int, key: Secret) -> Bool:
    return match Customer.get(id):
        Some(customer) -> customer.stripe_key == key
        None -> false

agent has_backup(id: Int) -> Bool:
    return match Customer.get(id):
        Some(customer) -> match customer.backup_key:
            Some(_) -> true
            None -> false
        None -> false
"#
    )
}

fn secret(text: &str) -> Value {
    Value::Secret(corvid_crypto::Secret::new(text))
}

async fn run(ir: &corvid_ir::IrFile, agent: &str, args: Vec<Value>, runtime: &Runtime) -> Result<Value, String> {
    run_ir_with_runtime(ir, Some(agent), args, runtime).await.map_err(|e| format!("{e:?}"))
}

/// The `stripe_key` column as the database stores it.
async fn stored_column(database: &TestDatabase, id: i64) -> String {
    match database {
        TestDatabase::Sqlite { path, .. } => rusqlite::Connection::open(path)
            .unwrap()
            .query_row("SELECT stripe_key FROM Customer WHERE id = ?1", [id], |row| row.get(0))
            .unwrap(),
        TestDatabase::Postgres { client, .. } => {
            client.query_one(r#"SELECT stripe_key::text FROM "Customer" WHERE id = $1"#, &[&id]).await.unwrap().get(0)
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_secret_column_is_stored_sealed_and_reads_back_as_the_secret() {
    std::env::set_var(KEY_SECRET, corvid_crypto::DataKey::generate_base64().unwrap());
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|e| panic!("{e:#?}"));
        let runtime = Runtime::builder().build();
        assert!(matches!(run(&ir, "store", vec![Value::Int(1), secret(PLAINTEXT)], &runtime).await, Ok(Value::Int(1))));

        let stored = stored_column(&database, 1).await;
        assert!(stored.starts_with("corvid:v1:"), "{}: stored as {stored}", database.label());
        assert!(!stored.contains("sk_live"), "{}: the database holds ciphertext", database.label());

        assert!(matches!(run(&ir, "holds", vec![Value::Int(1), secret(PLAINTEXT)], &runtime).await, Ok(Value::Bool(true))));
        assert!(matches!(run(&ir, "holds", vec![Value::Int(1), secret("sk_live_other")], &runtime).await, Ok(Value::Bool(false))));
        assert!(matches!(run(&ir, "has_backup", vec![Value::Int(1)], &runtime).await, Ok(Value::Bool(false))), "a missing Option<Secret> is NULL");
        database.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sealed_value_copied_to_another_row_does_not_open() {
    std::env::set_var("CORVID_TEST_KEY_MOVED", corvid_crypto::DataKey::generate_base64().unwrap());
    let database = TestDatabase::sqlite();
    let source = program_with(&database.settings(), "CORVID_TEST_KEY_MOVED");
    database.create_tables(&source).await;
    let ir = compile_to_ir(&source).unwrap();
    let runtime = Runtime::builder().build();
    run(&ir, "store", vec![Value::Int(1), secret(PLAINTEXT)], &runtime).await.unwrap();
    run(&ir, "store", vec![Value::Int(2), secret("sk_live_bo")], &runtime).await.unwrap();
    let TestDatabase::Sqlite { path, .. } = &database else { unreachable!() };
    rusqlite::Connection::open(path)
        .unwrap()
        .execute("UPDATE Customer SET stripe_key = (SELECT stripe_key FROM Customer WHERE id = 1) WHERE id = 2", [])
        .unwrap();
    let moved = run(&ir, "holds", vec![Value::Int(2), secret(PLAINTEXT)], &runtime).await.unwrap_err();
    assert!(moved.contains("does not open") && moved.contains("stripe_key"), "{moved}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_trace_holds_ciphertext_and_a_replay_needs_the_key() {
    const KEY_SECRET: &str = "CORVID_TEST_KEY_REPLAY";
    std::env::set_var(KEY_SECRET, corvid_crypto::DataKey::generate_base64().unwrap());
    let database = TestDatabase::sqlite();
    let source = program_with(&database.settings(), KEY_SECRET);
    database.create_tables(&source).await;
    let ir = compile_to_ir(&source).unwrap();
    run(&ir, "store", vec![Value::Int(1), secret(PLAINTEXT)], &Runtime::builder().build()).await.unwrap();

    let traces = tempfile::tempdir().unwrap();
    let recording = Runtime::builder().trace_to(traces.path()).build();
    assert!(matches!(run(&ir, "holds", vec![Value::Int(1), secret(PLAINTEXT)], &recording).await, Ok(Value::Bool(true))));
    let trace_path = recording.tracer().path().to_path_buf();
    drop(recording);
    let trace = std::fs::read_to_string(&trace_path).unwrap();
    assert!(trace.contains("corvid:v1:"), "the read is recorded as the database returned it");
    assert!(!trace.contains("sk_live"), "the trace never holds the secret:\n{trace}");

    let replay = Runtime::builder().replay_from(&trace_path).build();
    assert!(matches!(run(&ir, "holds", vec![Value::Int(1), secret(PLAINTEXT)], &replay).await, Ok(Value::Bool(true))), "a replay with the key reads the row");

    std::env::remove_var(KEY_SECRET);
    let keyless = Runtime::builder().replay_from(&trace_path).build();
    let refused = run(&ir, "holds", vec![Value::Int(1), secret(PLAINTEXT)], &keyless).await.unwrap_err();
    assert!(refused.contains(KEY_SECRET) && refused.contains("not set"), "{refused}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_that_is_not_32_bytes_is_refused_by_name_without_quoting_it() {
    const KEY_SECRET: &str = "CORVID_TEST_KEY_SHORT";
    std::env::set_var(KEY_SECRET, "dG9vLXNob3J0");
    let database = TestDatabase::sqlite();
    let source = program_with(&database.settings(), KEY_SECRET);
    database.create_tables(&source).await;
    let ir = compile_to_ir(&source).unwrap();
    let refused = run(&ir, "store", vec![Value::Int(1), secret(PLAINTEXT)], &Runtime::builder().build()).await.unwrap_err();
    assert!(refused.contains(KEY_SECRET) && refused.contains("32"), "{refused}");
    assert!(!refused.contains("dG9vLXNob3J0"), "{refused}");
}

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

#[test]
fn a_secret_column_needs_a_key_and_is_never_compared_ordered_or_sent_live() {
    let base = program("    engine: sqlite\n    location: \"billing.sqlite\"");
    assert!(errors(&base).is_empty(), "{:#?}", errors(&base));
    let without_key = base.replace(&format!("    encryption: key(secret(\"{KEY_SECRET}\"))\n"), "");
    assert!(errors(&without_key).iter().any(|e| e.contains("stripe_key") && e.contains("encryption: key")), "{:#?}", errors(&without_key));

    let with = |extra: &str| format!("{base}\n{extra}\n");
    let compared = with("agent find(key: Secret) -> Int:\n    return Customer.where(fn (c) -> c.stripe_key == key).count()");
    assert!(errors(&compared).iter().any(|e| e.contains("stored sealed as ciphertext")), "{:#?}", errors(&compared));
    let ordered = with("agent first() -> Int:\n    return Customer.all().order_by(fn (c) -> c.stripe_key).count()");
    assert!(!errors(&ordered).is_empty(), "ordering by a sealed column is refused");
    let unique = base.replace("    backup_key: Option<Secret>\n", "    backup_key: Option<Secret>\n    unique stripe_key\n");
    assert!(errors(&unique).iter().any(|e| e.contains("stored sealed as ciphertext")), "{:#?}", errors(&unique));
    let live = with("server billing_api:\n    live \"/customers\" -> Customer requires anyone:\n        return Customer.all()");
    assert!(errors(&live).iter().any(|e| e.contains("never receives a secret")), "{:#?}", errors(&live));
}

#[test]
fn a_previous_key_waits_for_rotation() {
    let source = program("    engine: sqlite\n    location: \"billing.sqlite\"")
        .replace(&format!("key(secret(\"{KEY_SECRET}\"))"), &format!("key(secret(\"{KEY_SECRET}\"), previous: secret(\"OLD\"))"));
    assert!(errors(&source).iter().any(|e| e.contains("K4")), "{:#?}", errors(&source));
}
