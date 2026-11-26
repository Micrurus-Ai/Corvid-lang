//! A test database on each engine the table tests run on (B09 P4).
//!
//! SQLite always runs, in a temporary file. Postgres runs when
//! `CORVID_TEST_POSTGRES_URL` names a test database; each test gets its
//! own schema there, so tests never see each other's rows, and the
//! schema is dropped at the end. Without the variable the Postgres half
//! prints `skipping:` and the test still checks SQLite.
//!
//! Tables are created from the program's own declarations through the
//! runtime's `CREATE TABLE`, the statement `corvid migrate plan` writes,
//! so a test cannot drift from what a migration would create.

#![allow(dead_code)]

use corvid_runtime::tables::{create_table_sql_on, TableEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

pub enum TestDatabase {
    Sqlite { _dir: tempfile::TempDir, path: std::path::PathBuf },
    Postgres { client: tokio_postgres::Client, schema: String, env: String },
}

impl TestDatabase {
    /// A database on every engine available here.
    pub async fn every_engine() -> Vec<TestDatabase> {
        let mut databases = vec![Self::sqlite()];
        match std::env::var("CORVID_TEST_POSTGRES_URL") {
            Ok(url) => databases.push(Self::postgres(&url).await),
            Err(_) => eprintln!("skipping: set CORVID_TEST_POSTGRES_URL to run the table tests on Postgres too"),
        }
        databases
    }

    pub fn sqlite() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("library.sqlite");
        TestDatabase::Sqlite { _dir: dir, path }
    }

    async fn postgres(url: &str) -> Self {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .unwrap_or_else(|e| panic!("CORVID_TEST_POSTGRES_URL does not connect: {e}"));
        tokio::spawn(connection);
        let schema = format!("corvid_test_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst));
        // The program reaches the schema through `search_path`, so the
        // tables this connection creates must land there too.
        client
            .batch_execute(&format!("CREATE SCHEMA {schema}; SET search_path TO {schema}"))
            .await
            .expect("create the test schema");
        let separator = if url.contains('?') { '&' } else { '?' };
        let env = format!("CORVID_TEST_DATABASE_{}", schema.to_uppercase());
        std::env::set_var(&env, format!("{url}{separator}options=-csearch_path%3D{schema}"));
        TestDatabase::Postgres { client, schema, env }
    }

    pub fn engine(&self) -> TableEngine {
        match self {
            TestDatabase::Sqlite { .. } => TableEngine::Sqlite,
            TestDatabase::Postgres { .. } => TableEngine::Postgres { tls: false, connections: 4 },
        }
    }

    /// The settings a program's `database` block declares for this
    /// database, without `tables:`.
    pub fn settings(&self) -> String {
        match self {
            TestDatabase::Sqlite { path, .. } => {
                format!("    engine: sqlite\n    location: \"{}\"", path.to_string_lossy().replace('\\', "/"))
            }
            TestDatabase::Postgres { env, .. } => {
                format!("    engine: postgres\n    location: env(\"{env}\")\n    tls: disabled\n    connections: 4")
            }
        }
    }

    /// Create the tables `source` declares, with the indexes a migration
    /// would give them, `unique` rules included.
    pub async fn create_tables(&self, source: &str) {
        let schemas = corvid_driver::table_schemas(source, std::path::Path::new("program.cor")).unwrap_or_else(|e| panic!("must compile: {e:#?}"));
        let mut sql = schemas.iter().map(|table| create_table_sql_on(table, self.engine())).collect::<Vec<_>>().join("\n");
        for table in &schemas {
            for (_, statement) in corvid_runtime::tables::plan_index_changes(table, &[]) {
                sql.push_str(&statement);
                sql.push('\n');
            }
        }
        match self {
            TestDatabase::Sqlite { path, .. } => {
                let connection = rusqlite::Connection::open(path).expect("open");
                corvid_runtime::tables::register_sqlite_collations(&connection).expect("register collations");
                connection.execute_batch(&sql).expect("create tables");
            }
            TestDatabase::Postgres { client, .. } => client.batch_execute(&sql).await.expect("create tables"),
        }
    }

    /// Remove the database, so a replay is shown not to need it.
    pub async fn remove(&self) {
        match self {
            TestDatabase::Sqlite { path, .. } => std::fs::remove_file(path).expect("remove the database"),
            TestDatabase::Postgres { client, schema, .. } => {
                client.batch_execute(&format!("DROP SCHEMA {schema} CASCADE")).await.expect("drop the test schema");
            }
        }
    }

    /// Whether the removed database is still gone.
    pub async fn is_gone(&self) -> bool {
        match self {
            TestDatabase::Sqlite { path, .. } => !path.exists(),
            TestDatabase::Postgres { client, schema, .. } => client
                .query_opt("SELECT 1 FROM pg_namespace WHERE nspname = $1", &[schema])
                .await
                .expect("look up the schema")
                .is_none(),
        }
    }

    /// Drop a Postgres test schema; a SQLite file goes with its directory.
    pub async fn finish(self) {
        if let TestDatabase::Postgres { client, schema, .. } = &self {
            client.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).await.expect("drop the test schema");
        }
    }

    pub fn label(&self) -> &'static str {
        self.engine().name()
    }
}
