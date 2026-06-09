use crate::errors::RuntimeError;
use rusqlite::{params_from_iter, Connection};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

// ============================================================================
// Phase 33S3 — opaque DbHandle for the executing SQLite surface.
//
// `DbHandleInner` is the payload of `corvid_vm::Value::DbHandle`. It
// lives in `corvid-runtime` (not `corvid-vm`) for two reasons:
//
//   1. The runtime's `DbHandleRegistry` is the sole authority for
//      allocating handle ids. Putting `DbHandleInner` here means the
//      same crate that owns the registry also owns the type that
//      wraps a registered id; the VM just imports the type.
//
//   2. `Runtime::db_open_tool` must MINT an `Arc<DbHandleInner>` and
//      hand it back to the interpreter (the dispatch path that
//      produces a `Value::DbHandle`). The runtime cannot construct
//      a `Value`, but it CAN construct an `Arc<DbHandleInner>` —
//      the interpreter then wraps the Arc in the Value variant.
//      This keeps the dependency direction clean: corvid-runtime
//      is lower than corvid-vm, so the Arc-shaped boundary is the
//      narrow waist between the two layers.
// ============================================================================

/// Phase 33S3a/b — the opaque, refcounted payload behind
/// `corvid_vm::Value::DbHandle`. Holds the registry slot id the
/// runtime uses to look up the actual `rusqlite::Connection` plus
/// the original opening `path` for diagnostics. Constructed only
/// by [`DbHandleRegistry::open`]; user code reaches this through
/// the typed-Value dispatch surface (`Runtime::db_open_tool`),
/// not directly. The "opaque" half of the brief's promise: with
/// no public constructor outside this module, no `From<u64>` or
/// `Default` impl, and the VM-level JSON marshalling refusing to
/// reconstruct a handle from JSON (`corvid_vm::conv::json_to_value`
/// rejects `Type::DbHandle`), user code structurally cannot
/// fabricate a SQLite connection.
#[derive(Debug)]
pub struct DbHandleInner {
    /// Slot key into `DbHandleRegistry`'s connection table. The
    /// registry is the sole authority for allocating these; the
    /// VM cannot mint an id, and there is no integer→DbHandleInner
    /// conversion exposed in this crate's public API.
    pub handle_id: u64,
    /// Original path the handle was opened against. `":memory:"`
    /// for ephemeral databases; an `[io] root`-relative resolved
    /// absolute path otherwise. Used purely for diagnostics
    /// (e.g. "no recorded db_query event for handle opened at
    /// `./data/app.sqlite`").
    pub path: String,
}

impl DbHandleInner {
    /// Construct a new `DbHandleInner`. Public so the VM's
    /// `Value::DbHandle` test fixtures can build handles without
    /// going through a real connection, and so 33S3b's dispatch
    /// path can mint one after `DbHandleRegistry::open` allocates
    /// a slot. Production user code reaches this through
    /// `Runtime::db_open_tool` (33S3b), never directly.
    pub fn new(handle_id: u64, path: impl Into<String>) -> Self {
        Self {
            handle_id,
            path: path.into(),
        }
    }
}

// ============================================================================
// Phase 33S3b — DbHandleRegistry.
//
// The executing `db_open` / `db_query` / `db_execute` stdlib tools
// (declared in `std/db.cor`) need a process-wide table mapping
// handle ids to live `rusqlite::Connection`s. The Corvid program
// receives a `Value::DbHandle(Arc<DbHandleInner>)`; the registry
// is the authority that translates the handle's `handle_id` back
// to a usable connection for `query` / `execute`.
//
// Lifetime: the registry holds `Arc<Mutex<Connection>>` per slot.
// Multiple Corvid handles to the same connection (clones of one
// `Value::DbHandle`) share the same Arc. When the last Corvid
// reference drops, the `Arc<DbHandleInner>` is dropped — 33S3c
// will wire a runtime-callback closer that releases the registry
// slot at that moment (completing the "refcounted" half of the
// brief's promise). 33S3b's slot release is bounded by the
// `DbHandleRegistry`'s own drop (runtime drop).
//
// Replay quarantine: `quarantine_writes` is the same shape as
// `IoRuntime::quarantine_writes` and `StoreManager::quarantine_writes`
// — when set, `execute` returns `QuarantineViolation { surface:
// "db", .. }`. Reads (`query`) pass through during replay because
// SQLite reads don't escape the process; the substitution path
// for db_query lives at the dispatch level (33S3c integrates the
// trace-substitution layer alongside the io / http precedents).
// ============================================================================

/// Registry of open SQLite connections keyed by handle id. Held
/// on `Runtime` (single instance per runtime, shared across
/// clones via internal `Arc`). The `Runtime::db_open_tool` /
/// `db_query_tool` / `db_execute_tool` dispatch methods are the
/// public surface that talks to this registry on the program's
/// behalf.
#[derive(Clone, Default)]
pub struct DbHandleRegistry {
    inner: std::sync::Arc<DbHandleRegistryInner>,
}

#[derive(Default)]
struct DbHandleRegistryInner {
    next_id: std::sync::atomic::AtomicU64,
    slots: std::sync::RwLock<
        std::collections::HashMap<u64, std::sync::Arc<Mutex<Connection>>>,
    >,
    /// Phase 33S3b — write-quarantine flag. Set by
    /// `RuntimeBuilder::build` when entering a Substitute-mode
    /// replay; `execute` then short-circuits with
    /// `QuarantineViolation { surface: "db", .. }`. Reads pass
    /// through (sqlite reads don't escape the process; the
    /// substitution path for replay lives at the dispatch level
    /// in 33S3c).
    quarantine_writes: std::sync::atomic::AtomicBool,
}

impl DbHandleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a SQLite connection at `path` and register it under a
    /// freshly-allocated handle id. The path is assumed to already
    /// be resolved through any `IoToolPolicy` confinement check
    /// the caller cares about — this method does not consult any
    /// policy. The documented special case `":memory:"` opens an
    /// in-memory database and skips path resolution at the
    /// dispatch layer.
    pub fn open(&self, path: &str) -> Result<std::sync::Arc<DbHandleInner>, RuntimeError> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory().map_err(|err| {
                RuntimeError::Other(format!("std.db sqlite open `:memory:` failed: {err}"))
            })?
        } else {
            Connection::open(path).map_err(|err| {
                RuntimeError::Other(format!("std.db sqlite open `{path}` failed: {err}"))
            })?
        };
        let id = self
            .inner
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner
            .slots
            .write()
            .map_err(|err| {
                RuntimeError::Other(format!("std.db registry poisoned: {err}"))
            })?
            .insert(id, std::sync::Arc::new(Mutex::new(conn)));
        Ok(std::sync::Arc::new(DbHandleInner::new(id, path)))
    }

    /// Run a parameterised SELECT against the connection at the
    /// supplied handle id. Returns rows + row_count. Parameter
    /// binding goes through `rusqlite::params_from_iter` over the
    /// typed `DbValue` enum — there is no string interpolation
    /// path; a literal `"'; DROP TABLE users; --"` inside `params`
    /// is bound as data and stored verbatim, not parsed as SQL.
    /// 33S3b's plumbing test pins this property.
    pub fn query(
        &self,
        handle_id: u64,
        sql: &str,
        params: &[DbValue],
    ) -> Result<DbQueryRows, RuntimeError> {
        let conn = self.lookup(handle_id)?;
        let conn = conn.lock().map_err(|err| {
            RuntimeError::Other(format!("std.db connection mutex poisoned: {err}"))
        })?;
        let sql_params = params.iter().map(db_value_to_sql_value).collect::<Vec<_>>();
        let mut stmt = conn.prepare(sql).map_err(redacted_sql_error)?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let rows = stmt
            .query_map(params_from_iter(sql_params), |row| {
                let mut cells = BTreeMap::new();
                for (index, name) in column_names.iter().enumerate() {
                    let value = row.get_ref(index)?;
                    let db_value = db_value_from_ref(value);
                    cells.insert(
                        name.clone(),
                        DbCell {
                            kind: db_value.kind().to_string(),
                            value: db_value,
                            redacted: false,
                        },
                    );
                }
                Ok(cells)
            })
            .map_err(redacted_sql_error)?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row.map_err(redacted_sql_error)?);
        }
        Ok(DbQueryRows {
            row_count: collected.len(),
            rows: collected,
        })
    }

    /// Run a parameterised INSERT / UPDATE / DELETE / DDL against
    /// the connection at the supplied handle id. Refused with
    /// `QuarantineViolation { surface: "db", .. }` when the
    /// registry is in replay write-quarantine mode. Parameter
    /// binding goes through `params_from_iter` over typed
    /// `DbValue`s — the same injection-resistant path as `query`.
    pub fn execute(
        &self,
        handle_id: u64,
        sql: &str,
        params: &[DbValue],
    ) -> Result<DbExecuteResult, RuntimeError> {
        if self
            .inner
            .quarantine_writes
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(RuntimeError::QuarantineViolation {
                surface: "db".to_string(),
                detail: format!(
                    "blocked an unrecorded `db_execute` call against handle {handle_id} \
                     during replay-mode quarantine. A replayed run must not mutate the \
                     database; if the program's recorded trace does not carry the \
                     equivalent execute event, the replay diverges rather than re-issuing \
                     the SQL."
                ),
            });
        }
        let conn = self.lookup(handle_id)?;
        let conn = conn.lock().map_err(|err| {
            RuntimeError::Other(format!("std.db connection mutex poisoned: {err}"))
        })?;
        let sql_params = params.iter().map(db_value_to_sql_value).collect::<Vec<_>>();
        let rows_affected = conn
            .execute(sql, params_from_iter(sql_params))
            .map_err(redacted_sql_error)?;
        Ok(DbExecuteResult {
            rows_affected: rows_affected as u64,
        })
    }

    /// Flip into replay-quarantine mode. Subsequent `execute`
    /// calls fail closed with `QuarantineViolation { surface:
    /// "db", .. }`. Called by `RuntimeBuilder::build` when
    /// entering Substitute-mode replay.
    pub fn quarantine_writes(&self) {
        self.inner
            .quarantine_writes
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// True when this registry refuses live `execute` calls.
    /// Test helper + introspection.
    pub fn is_write_quarantined(&self) -> bool {
        self.inner
            .quarantine_writes
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn lookup(
        &self,
        handle_id: u64,
    ) -> Result<std::sync::Arc<Mutex<Connection>>, RuntimeError> {
        self.inner
            .slots
            .read()
            .map_err(|err| {
                RuntimeError::Other(format!("std.db registry poisoned: {err}"))
            })?
            .get(&handle_id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Other(format!(
                    "std.db dispatch received an unregistered handle id `{handle_id}` — \
                     the handle was either forged or already released. Handles can \
                     only be minted by `db_open` and remain valid for the runtime's \
                     lifetime."
                ))
            })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Bool(bool),
}

impl DbValue {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Integer(_) => "Int",
            Self::Float(_) => "Float",
            Self::Text(_) => "String",
            Self::Bool(_) => "Bool",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbCell {
    pub kind: String,
    pub value: DbValue,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DbQueryRows {
    pub rows: Vec<BTreeMap<String, DbCell>>,
    pub row_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbExecuteResult {
    pub rows_affected: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbDecodeError {
    pub field_path: String,
    pub expected_type: String,
    pub received_kind: String,
    pub message: String,
}

pub struct SqliteDbRuntime {
    conn: Mutex<Connection>,
}

pub struct PostgresDbRuntime {
    client: Mutex<postgres::Client>,
}

impl SqliteDbRuntime {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let conn = Connection::open(path.as_ref()).map_err(|err| {
            RuntimeError::Other(format!(
                "std.db sqlite open failed for `{}`: {err}",
                path.as_ref().display()
            ))
        })?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn open_in_memory() -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory()
            .map_err(|err| RuntimeError::Other(format!("std.db sqlite open failed: {err}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn execute(&self, sql: &str, params: &[DbValue]) -> Result<DbExecuteResult, RuntimeError> {
        let sql_params = params.iter().map(db_value_to_sql_value).collect::<Vec<_>>();
        let rows_affected = self
            .conn
            .lock()
            .unwrap()
            .execute(sql, params_from_iter(sql_params))
            .map_err(redacted_sql_error)?;
        Ok(DbExecuteResult {
            rows_affected: rows_affected as u64,
        })
    }

    pub fn execute_batch_transaction(&self, statements: &[&str]) -> Result<DbExecuteResult, RuntimeError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(redacted_sql_error)?;
        let mut rows_affected = 0_u64;
        for statement in statements {
            rows_affected = rows_affected.saturating_add(
                tx.execute(statement, [])
                    .map_err(redacted_sql_error)? as u64,
            );
        }
        tx.commit().map_err(redacted_sql_error)?;
        Ok(DbExecuteResult { rows_affected })
    }

    pub fn query(&self, sql: &str, params: &[DbValue]) -> Result<DbQueryRows, RuntimeError> {
        let conn = self.conn.lock().unwrap();
        let sql_params = params.iter().map(db_value_to_sql_value).collect::<Vec<_>>();
        let mut stmt = conn.prepare(sql).map_err(redacted_sql_error)?;
        let column_names = stmt
            .column_names()
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let rows = stmt
            .query_map(params_from_iter(sql_params), |row| {
                let mut cells = BTreeMap::new();
                for (index, name) in column_names.iter().enumerate() {
                    let value = row.get_ref(index)?;
                    let db_value = db_value_from_ref(value);
                    cells.insert(
                        name.clone(),
                        DbCell {
                            kind: db_value.kind().to_string(),
                            value: db_value,
                            redacted: false,
                        },
                    );
                }
                Ok(cells)
            })
            .map_err(redacted_sql_error)?;
        let mut collected = Vec::new();
        for row in rows {
            collected.push(row.map_err(redacted_sql_error)?);
        }
        Ok(DbQueryRows {
            row_count: collected.len(),
            rows: collected,
        })
    }
}

impl PostgresDbRuntime {
    pub fn connect(dsn: &str) -> Result<Self, RuntimeError> {
        let client = postgres::Client::connect(dsn, postgres::NoTls)
            .map_err(redacted_postgres_error)?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }

    pub fn execute(&self, sql: &str, params: &[DbValue]) -> Result<DbExecuteResult, RuntimeError> {
        let params = params
            .iter()
            .map(db_value_to_postgres_param)
            .collect::<Vec<_>>();
        let param_refs = params
            .iter()
            .map(|value| value as &(dyn postgres::types::ToSql + Sync))
            .collect::<Vec<_>>();
        let rows_affected = self
            .client
            .lock()
            .unwrap()
            .execute(sql, &param_refs)
            .map_err(redacted_postgres_error)?;
        Ok(DbExecuteResult { rows_affected })
    }

    pub fn query(&self, sql: &str, params: &[DbValue]) -> Result<DbQueryRows, RuntimeError> {
        let params = params
            .iter()
            .map(db_value_to_postgres_param)
            .collect::<Vec<_>>();
        let param_refs = params
            .iter()
            .map(|value| value as &(dyn postgres::types::ToSql + Sync))
            .collect::<Vec<_>>();
        let rows = self
            .client
            .lock()
            .unwrap()
            .query(sql, &param_refs)
            .map_err(redacted_postgres_error)?;
        let mut collected = Vec::new();
        for row in rows {
            let mut cells = BTreeMap::new();
            for column in row.columns() {
                let name = column.name().to_string();
                let value = postgres_cell_value(&row, &name);
                cells.insert(
                    name,
                    DbCell {
                        kind: value.kind().to_string(),
                        value,
                        redacted: false,
                    },
                );
            }
            collected.push(cells);
        }
        Ok(DbQueryRows {
            row_count: collected.len(),
            rows: collected,
        })
    }
}

pub fn decode_string(
    row: &BTreeMap<String, DbCell>,
    field: &str,
) -> Result<String, DbDecodeError> {
    let cell = row.get(field).ok_or_else(|| DbDecodeError {
        field_path: field.to_string(),
        expected_type: "String".to_string(),
        received_kind: "missing".to_string(),
        message: "missing column".to_string(),
    })?;
    match &cell.value {
        DbValue::Text(value) => Ok(value.clone()),
        other => Err(DbDecodeError {
            field_path: field.to_string(),
            expected_type: "String".to_string(),
            received_kind: other.kind().to_string(),
            message: "wrong value kind".to_string(),
        }),
    }
}

pub fn decode_i64(row: &BTreeMap<String, DbCell>, field: &str) -> Result<i64, DbDecodeError> {
    let cell = row.get(field).ok_or_else(|| DbDecodeError {
        field_path: field.to_string(),
        expected_type: "Int".to_string(),
        received_kind: "missing".to_string(),
        message: "missing column".to_string(),
    })?;
    match &cell.value {
        DbValue::Integer(value) => Ok(*value),
        other => Err(DbDecodeError {
            field_path: field.to_string(),
            expected_type: "Int".to_string(),
            received_kind: other.kind().to_string(),
            message: "wrong value kind".to_string(),
        }),
    }
}

fn db_value_to_sql_value(value: &DbValue) -> rusqlite::types::Value {
    match value {
        DbValue::Null => rusqlite::types::Value::Null,
        DbValue::Integer(value) => rusqlite::types::Value::Integer(*value),
        DbValue::Float(value) => rusqlite::types::Value::Real(*value),
        DbValue::Text(value) => rusqlite::types::Value::Text(value.clone()),
        DbValue::Bool(value) => rusqlite::types::Value::Integer(i64::from(*value)),
    }
}

#[derive(Debug)]
enum PostgresParam {
    Null(Option<String>),
    Integer(i64),
    Float(f64),
    Text(String),
    Bool(bool),
}

impl postgres::types::ToSql for PostgresParam {
    fn to_sql(
        &self,
        ty: &postgres::types::Type,
        out: &mut postgres::types::private::BytesMut,
    ) -> Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            Self::Null(value) => value.to_sql(ty, out),
            Self::Integer(value) => value.to_sql(ty, out),
            Self::Float(value) => value.to_sql(ty, out),
            Self::Text(value) => value.to_sql(ty, out),
            Self::Bool(value) => value.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &postgres::types::Type) -> bool {
        true
    }

    postgres::types::to_sql_checked!();
}

fn db_value_to_postgres_param(value: &DbValue) -> PostgresParam {
    match value {
        DbValue::Null => PostgresParam::Null(None),
        DbValue::Integer(value) => PostgresParam::Integer(*value),
        DbValue::Float(value) => PostgresParam::Float(*value),
        DbValue::Text(value) => PostgresParam::Text(value.clone()),
        DbValue::Bool(value) => PostgresParam::Bool(*value),
    }
}

fn postgres_cell_value(row: &postgres::Row, name: &str) -> DbValue {
    if let Ok(value) = row.try_get::<_, Option<String>>(name) {
        return value.map(DbValue::Text).unwrap_or(DbValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<i64>>(name) {
        return value.map(DbValue::Integer).unwrap_or(DbValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<i32>>(name) {
        return value
            .map(|value| DbValue::Integer(i64::from(value)))
            .unwrap_or(DbValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<f64>>(name) {
        return value.map(DbValue::Float).unwrap_or(DbValue::Null);
    }
    if let Ok(value) = row.try_get::<_, Option<bool>>(name) {
        return value.map(DbValue::Bool).unwrap_or(DbValue::Null);
    }
    DbValue::Text("<unsupported:redacted>".to_string())
}

fn db_value_from_ref(value: rusqlite::types::ValueRef<'_>) -> DbValue {
    match value {
        rusqlite::types::ValueRef::Null => DbValue::Null,
        rusqlite::types::ValueRef::Integer(value) => DbValue::Integer(value),
        rusqlite::types::ValueRef::Real(value) => DbValue::Float(value),
        rusqlite::types::ValueRef::Text(value) => {
            DbValue::Text(String::from_utf8_lossy(value).to_string())
        }
        rusqlite::types::ValueRef::Blob(_) => DbValue::Text("<blob:redacted>".to_string()),
    }
}

fn redacted_sql_error(err: rusqlite::Error) -> RuntimeError {
    RuntimeError::Other(format!("std.db sqlite error: {err}; values redacted"))
}

fn redacted_postgres_error(err: postgres::Error) -> RuntimeError {
    RuntimeError::Other(format!("std.db postgres error: {err}; values redacted"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_execute_query_and_decode_round_trip() {
        let db = SqliteDbRuntime::open_in_memory().expect("sqlite");
        db.execute(
            "create table users(id integer primary key, email text not null)",
            &[],
        )
        .expect("create");
        let inserted = db
            .execute(
                "insert into users(id, email) values (?1, ?2)",
                &[
                    DbValue::Integer(7),
                    DbValue::Text("dev@example.com".to_string()),
                ],
            )
            .expect("insert");
        assert_eq!(inserted.rows_affected, 1);

        let rows = db
            .query(
                "select id, email from users where id = ?1",
                &[DbValue::Integer(7)],
            )
            .expect("query");
        assert_eq!(rows.row_count, 1);
        assert_eq!(decode_i64(&rows.rows[0], "id").unwrap(), 7);
        assert_eq!(
            decode_string(&rows.rows[0], "email").unwrap(),
            "dev@example.com"
        );
    }

    #[test]
    fn sqlite_transaction_rolls_back_on_failure() {
        let db = SqliteDbRuntime::open_in_memory().expect("sqlite");
        db.execute("create table tasks(id integer primary key)", &[])
            .expect("create");
        let failed = db.execute_batch_transaction(&[
            "insert into tasks(id) values (1)",
            "insert into missing_table(id) values (2)",
        ]);
        assert!(failed.is_err());
        let rows = db.query("select id from tasks", &[]).expect("query");
        assert_eq!(rows.row_count, 0);
    }

    #[test]
    fn sqlite_decode_reports_missing_and_wrong_kind() {
        let db = SqliteDbRuntime::open_in_memory().expect("sqlite");
        db.execute("create table users(id integer primary key)", &[])
            .expect("create");
        db.execute("insert into users(id) values (?1)", &[DbValue::Integer(1)])
            .expect("insert");
        let rows = db.query("select id from users", &[]).expect("query");

        let missing = decode_string(&rows.rows[0], "email").expect_err("missing");
        assert_eq!(missing.received_kind, "missing");
        let wrong = decode_string(&rows.rows[0], "id").expect_err("wrong kind");
        assert_eq!(wrong.received_kind, "Int");
    }

    #[test]
    fn postgres_runtime_uses_real_driver_and_redacts_connection_error() {
        let err = match PostgresDbRuntime::connect(
            "host=127.0.0.1 port=1 user=corvid dbname=corvid connect_timeout=1",
        ) {
            Ok(_) => panic!("port 1 should not accept postgres"),
            Err(err) => err,
        };
        let rendered = err.to_string();
        assert!(rendered.contains("std.db postgres error"), "{rendered}");
        assert!(rendered.contains("values redacted"), "{rendered}");
    }

    // -------- Phase 33S3b — DbHandleRegistry plumbing tests --------

    /// 33S3b — the registry round-trips a :memory: connection
    /// end-to-end: open, create, parameterized insert, query.
    /// This is the plumbing-layer equivalent of 33S2a's
    /// HttpEgressPolicy unit tests — load-bearing acceptance
    /// that the dispatch-level methods actually work against
    /// rusqlite.
    #[test]
    fn db_handle_registry_round_trip_against_memory_database() {
        let reg = DbHandleRegistry::new();
        let handle = reg.open(":memory:").expect("open :memory:");
        assert_eq!(handle.path, ":memory:");
        let id = handle.handle_id;

        reg.execute(
            id,
            "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
            &[],
        )
        .expect("create");
        let inserted = reg
            .execute(
                id,
                "INSERT INTO users(id, email) VALUES (?, ?)",
                &[DbValue::Integer(1), DbValue::Text("alice@example.com".into())],
            )
            .expect("insert");
        assert_eq!(inserted.rows_affected, 1);
        let rows = reg
            .query(id, "SELECT email FROM users WHERE id = ?", &[DbValue::Integer(1)])
            .expect("select");
        assert_eq!(rows.row_count, 1);
        let email = match rows.rows[0].get("email").map(|c| &c.value) {
            Some(DbValue::Text(s)) => s.clone(),
            other => panic!("expected Text email, got {other:?}"),
        };
        assert_eq!(email, "alice@example.com");
    }

    /// 33S3b — **the load-bearing injection-proof test**. A
    /// parameter whose `DbValue::Text` carries SQL syntax —
    /// `"'; DROP TABLE users; --"` — is bound as DATA and stored
    /// verbatim. The table still exists after the insert; the
    /// stored email is the exact metacharacter-laden string,
    /// not interpolated as SQL.
    ///
    /// This proves the structural property the executing SQLite
    /// surface advertises: `params_from_iter` never sees the
    /// string as SQL because the parameter binding goes through
    /// rusqlite's typed value path, not through string
    /// concatenation. There is no `format!("...{}...")` anywhere
    /// on the dispatch path; the typechecker's `List<DbParam>`
    /// signature forces every user value through the typed
    /// constructors.
    #[test]
    fn db_param_text_with_sql_metacharacters_is_bound_as_data() {
        let reg = DbHandleRegistry::new();
        let handle = reg.open(":memory:").expect("open :memory:");
        let id = handle.handle_id;

        reg.execute(
            id,
            "CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
            &[],
        )
        .expect("create");

        let attack = "'; DROP TABLE users; --";
        reg.execute(
            id,
            "INSERT INTO users(id, email) VALUES (?, ?)",
            &[DbValue::Integer(1), DbValue::Text(attack.into())],
        )
        .expect("insert with attack string");

        // The table must still exist (DROP would have removed it
        // if the string had been interpolated as SQL).
        let count = reg
            .query(id, "SELECT count(*) AS c FROM users", &[])
            .expect("count query — proves the table survived");
        assert_eq!(count.row_count, 1);
        let n = match count.rows[0].get("c").map(|c| &c.value) {
            Some(DbValue::Integer(n)) => *n,
            other => panic!("expected Integer count, got {other:?}"),
        };
        assert_eq!(n, 1, "the attack string must not have dropped the table");

        // The stored email must be the EXACT metacharacter string
        // — not parsed, not escaped, not transformed.
        let rows = reg
            .query(id, "SELECT email FROM users WHERE id = ?", &[DbValue::Integer(1)])
            .expect("select stored email");
        let email = match rows.rows[0].get("email").map(|c| &c.value) {
            Some(DbValue::Text(s)) => s.clone(),
            other => panic!("expected Text email, got {other:?}"),
        };
        assert_eq!(
            email, attack,
            "the stored string must be the verbatim parameter, never interpolated"
        );
    }

    /// 33S3b — write-quarantine refuses `execute` calls during
    /// Substitute-mode replay. The `quarantine_writes` flag is
    /// the same shape as `IoRuntime::quarantine_writes` and
    /// `StoreManager::quarantine_writes`; this test pins the
    /// SQLite parallel.
    #[test]
    fn db_handle_registry_quarantine_blocks_execute_with_db_surface_violation() {
        let reg = DbHandleRegistry::new();
        let handle = reg.open(":memory:").expect("open :memory:");
        reg.quarantine_writes();
        assert!(reg.is_write_quarantined());

        let err = reg
            .execute(
                handle.handle_id,
                "INSERT INTO users(id) VALUES (?)",
                &[DbValue::Integer(1)],
            )
            .expect_err("execute must refuse during write-quarantine");
        match err {
            RuntimeError::QuarantineViolation { surface, .. } => {
                assert_eq!(surface, "db", "quarantine surface must be `db`");
            }
            other => panic!("expected QuarantineViolation, got {other:?}"),
        }
    }

    /// 33S3b — write-quarantine does NOT block `query` (reads
    /// pass through during replay; the trace-substitution layer
    /// 33S3c integrates is the upper gate). This test pins the
    /// read-pass-through property so a future refactor can't
    /// silently start blocking reads.
    #[test]
    fn db_handle_registry_quarantine_does_not_block_query() {
        let reg = DbHandleRegistry::new();
        let handle = reg.open(":memory:").expect("open :memory:");
        reg.execute(
            handle.handle_id,
            "CREATE TABLE t(x INTEGER)",
            &[],
        )
        .expect("setup table");
        reg.execute(
            handle.handle_id,
            "INSERT INTO t(x) VALUES (?)",
            &[DbValue::Integer(42)],
        )
        .expect("setup row");

        reg.quarantine_writes();

        // Query passes through.
        let rows = reg
            .query(handle.handle_id, "SELECT x FROM t", &[])
            .expect("query must pass through during write-quarantine");
        assert_eq!(rows.row_count, 1);
    }

    /// 33S3b — looking up an unregistered handle id is a
    /// structured error. Names the property: handles are
    /// minted only by `db_open` and cannot be forged.
    #[test]
    fn db_handle_registry_rejects_unregistered_handle_id() {
        let reg = DbHandleRegistry::new();
        let err = reg
            .query(99999, "SELECT 1", &[])
            .expect_err("unregistered id must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("unregistered handle id") && msg.contains("99999"),
            "diagnostic must name the property; got: {msg}"
        );
    }
}
