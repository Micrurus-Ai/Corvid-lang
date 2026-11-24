//! Table storage (B09): the SQL behind `Shelf.get`, `Shelf.insert`,
//! `Shelf.save` and `Shelf.delete`; queries live in [`query`].
//!
//! Operations run through the same [`crate::db::DbHandleRegistry`] the
//! `std.db` tools use, so they share its per-file writer lock, its
//! parameter-only binding (values never become SQL text) and its
//! replay write quarantine. Rows travel as [`DbValue`]s in column
//! order, with `revision` last; the interpreter converts its own
//! values to and from them.
//!
//! Every write is a compare-and-set on `revision`: `save` and `delete`
//! succeed only when the stored row still has the revision the caller
//! read, and each successful write advances it.

use crate::db::{DbHandleInner, DbValue};
use crate::errors::RuntimeError;
use crate::Runtime;
use corvid_ast::DatabaseLocation;
use corvid_types::Type;
use serde_json::Value as J;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod migration;
mod query;
pub use migration::{plan_table_change, StoredColumn, TableChange};
pub use query::{query_sql, TableFilter, TableQuery};

/// How a field's values are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Integer,
    Real,
    Text,
    /// A `Bool`, stored as 0 or 1.
    Boolean,
    /// Any other type, stored as its public JSON text.
    Json,
}

impl ColumnKind {
    /// The declared SQL type. Each kind has its own, so the stored
    /// schema records how a column is stored and the migration planner
    /// can see a change from `Int` to `Bool` or from `String` to a
    /// JSON-stored type. `BOOLEAN` has SQLite's numeric affinity, which
    /// keeps 0 and 1 as integers; `JSON TEXT` contains `TEXT`, so its
    /// affinity is text and JSON such as `5` is never turned into a
    /// number.
    pub fn sql_type(self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Text => "TEXT",
            Self::Boolean => "BOOLEAN",
            Self::Json => "JSON TEXT",
        }
    }

    /// The kind a stored column's declared type names, if Corvid
    /// created it.
    pub fn from_sql_type(sql_type: &str) -> Option<Self> {
        Some(match sql_type.trim().to_ascii_uppercase().as_str() {
            "INTEGER" => Self::Integer,
            "REAL" => Self::Real,
            "TEXT" => Self::Text,
            "BOOLEAN" => Self::Boolean,
            "JSON TEXT" => Self::Json,
            _ => return None,
        })
    }
}

/// How a field of type `ty` is stored, and whether the column admits
/// `NULL`. The interpreter and the migration planner both decide
/// columns here. `Option<T>` is a nullable column of `T`'s kind, except
/// when `T` is itself an `Option`: `Some(None)` and `None` would both be
/// `NULL`, so a nested option is stored as JSON.
pub fn column_kind(ty: &Type) -> (ColumnKind, bool) {
    match ty {
        Type::Tainted(inner) => column_kind(inner),
        Type::Option(inner) => match untainted(inner) {
            Type::Option(_) => (ColumnKind::Json, true),
            inner => (column_kind(inner).0, true),
        },
        Type::Int => (ColumnKind::Integer, false),
        Type::Float => (ColumnKind::Real, false),
        Type::String => (ColumnKind::Text, false),
        Type::Bool => (ColumnKind::Boolean, false),
        _ => (ColumnKind::Json, false),
    }
}

/// `ty` without `Tainted` wrappers; taint is tracked statically.
pub fn untainted(mut ty: &Type) -> &Type {
    while let Type::Tainted(inner) = ty {
        ty = inner;
    }
    ty
}

/// A table's declared shape: its name, key and typed columns, with
/// `revision` last, and the history clauses the migration planner reads.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub key: String,
    pub columns: Vec<(String, Type)>,
    /// `(field, old name)` for each `renamed from`.
    pub renamed: Vec<(String, String)>,
    /// Stored columns declared `retired`.
    pub retired: Vec<String>,
}

/// The `CREATE TABLE` statement for a declared table.
pub fn create_table_sql(table: &TableSchema) -> String {
    create_table_named(table, &table.name)
}

/// [`create_table_sql`] under another name, for rebuilding a table.
fn create_table_named(table: &TableSchema, name: &str) -> String {
    let columns = table
        .columns
        .iter()
        .map(|(name, ty)| {
            let (kind, nullable) = column_kind(ty);
            let key = if *name == table.key { " PRIMARY KEY" } else { "" };
            let null = if nullable { "" } else { " NOT NULL" };
            format!("    {} {}{key}{null}", quote(name), kind.sql_type())
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("CREATE TABLE {} (\n{columns}\n);\n", quote(name))
}

/// One table as an operation sees it.
#[derive(Debug, Clone, Copy)]
pub struct TableRef<'a> {
    pub name: &'a str,
    pub key: &'a str,
    /// Column names in declaration order, with `revision` last.
    pub columns: &'a [String],
    pub location: &'a DatabaseLocation,
}

/// A row's values, in [`TableRef::columns`] order.
pub type Row = Vec<DbValue>;

/// The open database handle for each location, shared by a runtime's
/// table operations.
#[derive(Default, Clone)]
pub struct TableDatabases {
    handles: Arc<Mutex<HashMap<String, Arc<DbHandleInner>>>>,
}

/// Where a declared location points now.
pub fn resolve_location(location: &DatabaseLocation) -> Result<String, RuntimeError> {
    match location {
        DatabaseLocation::Path(path) => Ok(path.clone()),
        DatabaseLocation::Env(name) => std::env::var(name).map_err(|_| {
            RuntimeError::Other(format!(
                "the database location comes from the environment variable `{name}`, which is not set"
            ))
        }),
    }
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

impl Runtime {
    fn table_handle(&self, table: &TableRef<'_>) -> Result<Arc<DbHandleInner>, RuntimeError> {
        let path = resolve_location(table.location)?;
        let mut handles = self.table_databases.handles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(handle) = handles.get(&path) {
            return Ok(handle.clone());
        }
        let handle = self.db_registry().open(&path)?;
        handles.insert(path, handle.clone());
        Ok(handle)
    }

    /// `Shelf.get(key)`.
    async fn table_get_live(&self, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        let handle = self.table_handle(table)?;
        self.select_row(&handle, table, key)
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Err` with the row already stored under that key.
    async fn table_insert_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Row>, RuntimeError> {
        let handle = self.table_handle(table)?;
        let (columns, mut values) = split_revision(table, row)?;
        let names = columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let marks = (1..=values.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "INSERT INTO {} ({names}, {}) VALUES ({marks}, 1) ON CONFLICT DO NOTHING",
            quote(table.name),
            quote(corvid_ast::TableDecl::REVISION_FIELD)
        );
        let inserted = self.db_registry().execute_queued(&handle, &sql, &values).await.map_err(|e| named(table, e))?;
        let key = key_value(table, &values)?;
        if inserted.rows_affected == 1 {
            values.push(DbValue::Integer(1));
            return Ok(Ok(values));
        }
        let existing = self.select_row(&handle, table, key)?.ok_or_else(|| {
            RuntimeError::Other(format!("table `{}`: an insert conflicted with a row that is gone", table.name))
        })?;
        Ok(Err(existing))
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or `Err` with the row stored now (`None` if it was deleted).
    async fn table_save_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let handle = self.table_handle(table)?;
        let revision = revision_of(table, &row)?;
        let (columns, mut values) = split_revision(table, row)?;
        let key = key_value(table, &values)?;
        let assignments = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| *c != table.key)
            .map(|(i, c)| format!("{} = ?{}", quote(c), i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let key_mark = columns.iter().position(|c| *c == table.key).map(|i| i + 1).unwrap_or(1);
        let revision_column = quote(corvid_ast::TableDecl::REVISION_FIELD);
        let set = if assignments.is_empty() { String::new() } else { format!("{assignments}, ") };
        let sql = format!(
            "UPDATE {} SET {set}{revision_column} = {revision_column} + 1 WHERE {} = ?{key_mark} AND {revision_column} = ?{}",
            quote(table.name),
            quote(table.key),
            values.len() + 1
        );
        let mut params = values.clone();
        params.push(DbValue::Integer(revision));
        let updated = self.db_registry().execute_queued(&handle, &sql, &params).await.map_err(|e| named(table, e))?;
        if updated.rows_affected == 1 {
            values.push(DbValue::Integer(revision + 1));
            return Ok(Ok(values));
        }
        Ok(Err(self.select_row(&handle, table, key)?))
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or `Err` with
    /// the row stored now (`None` if it is already gone).
    async fn table_delete_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let handle = self.table_handle(table)?;
        let revision = revision_of(table, &row)?;
        let key = key_value(table, &row)?;
        let sql = format!(
            "DELETE FROM {} WHERE {} = ?1 AND {} = ?2",
            quote(table.name),
            quote(table.key),
            quote(corvid_ast::TableDecl::REVISION_FIELD)
        );
        let deleted = self
            .db_registry()
            .execute_queued(&handle, &sql, &[key.clone(), DbValue::Integer(revision)])
            .await
            .map_err(|e| named(table, e))?;
        if deleted.rows_affected == 1 {
            return Ok(Ok(row));
        }
        Ok(Err(self.select_row(&handle, table, key)?))
    }

    fn select_row(&self, handle: &DbHandleInner, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        let names = table.columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let sql = format!("SELECT {names} FROM {} WHERE {} = ?1", quote(table.name), quote(table.key));
        let rows = self.db_registry().query(handle.handle_id, &sql, &[key]).map_err(|e| named(table, e))?;
        let Some(cells) = rows.rows.into_iter().next() else { return Ok(None) };
        Ok(Some(ordered_row(table, cells)))
    }
}

/// A result row's cells in [`TableRef::columns`] order.
fn ordered_row(table: &TableRef<'_>, mut cells: std::collections::BTreeMap<String, crate::db::DbCell>) -> Row {
    table
        .columns
        .iter()
        .map(|c| cells.remove(c).map(|cell| cell.value).unwrap_or(DbValue::Null))
        .collect()
}

/// The recorded and replayable operations (P1b). Each is recorded as a
/// tool call named `table:<Table>.<op>`; a replay serves the recorded
/// outcome and never opens the database.
impl Runtime {
    /// `Shelf.get(key)`.
    pub async fn table_get(&self, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        let outcome = self
            .recorded_table_call(&call_name(table, "get"), vec![value_json(&key)], || async {
                Ok(match self.table_get_live(table, key).await? {
                    Some(row) => row_json(&row),
                    None => J::Null,
                })
            })
            .await?;
        if outcome.is_null() { Ok(None) } else { json_row(&outcome).map(Some) }
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Err` with the row already stored under that key.
    pub async fn table_insert(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Row>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "insert"), args, || async {
                Ok(match self.table_insert_live(table, row).await? {
                    Ok(stored) => serde_json::json!({ "ok": row_json(&stored) }),
                    Err(existing) => serde_json::json!({ "err": row_json(&existing) }),
                })
            })
            .await?;
        match (outcome.get("ok"), outcome.get("err")) {
            (Some(stored), _) => Ok(Ok(json_row(stored)?)),
            (_, Some(existing)) => Ok(Err(json_row(existing)?)),
            _ => Err(malformed(table, "insert")),
        }
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or `Err` with the row stored now (`None` if it was deleted).
    pub async fn table_save(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "save"), args, || async {
                Ok(checked_write_json(self.table_save_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table, "save", &outcome)
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or `Err` with the
    /// row stored now (`None` if it is already gone).
    pub async fn table_delete(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "delete"), args, || async {
                Ok(checked_write_json(self.table_delete_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table, "delete", &outcome)
    }
}

fn call_name(table: &TableRef<'_>, op: &str) -> String {
    format!("table:{}.{op}", table.name)
}

fn malformed(table: &TableRef<'_>, op: &str) -> RuntimeError {
    RuntimeError::Other(format!("the recorded outcome of `{}.{op}` is malformed", table.name))
}

fn checked_write_json(outcome: Result<Row, Option<Row>>) -> J {
    match outcome {
        Ok(row) => serde_json::json!({ "ok": row_json(&row) }),
        Err(current) => serde_json::json!({ "err": current.as_ref().map(|row| row_json(row)) }),
    }
}

fn checked_write_outcome(table: &TableRef<'_>, op: &str, outcome: &J) -> Result<Result<Row, Option<Row>>, RuntimeError> {
    match (outcome.get("ok"), outcome.get("err")) {
        (Some(row), _) => Ok(Ok(json_row(row)?)),
        (_, Some(J::Null)) => Ok(Err(None)),
        (_, Some(current)) => Ok(Err(Some(json_row(current)?))),
        _ => Err(malformed(table, op)),
    }
}

/// A column value as JSON for the trace. An integer stays an integer
/// and a float keeps its decimal point, so the two read back apart.
fn value_json(value: &DbValue) -> J {
    match value {
        DbValue::Null => J::Null,
        DbValue::Integer(n) => J::from(*n),
        DbValue::Float(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        DbValue::Text(text) => J::String(text.clone()),
        DbValue::Bool(b) => J::Bool(*b),
    }
}

fn json_value(json: &J) -> Result<DbValue, RuntimeError> {
    Ok(match json {
        J::Null => DbValue::Null,
        J::Bool(b) => DbValue::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => DbValue::Integer(i),
            None => DbValue::Float(n.as_f64().unwrap_or_default()),
        },
        J::String(text) => DbValue::Text(text.clone()),
        other => return Err(RuntimeError::Other(format!("a recorded column value `{other}` is not a scalar"))),
    })
}

fn row_json(row: &Row) -> J {
    J::Array(row.iter().map(value_json).collect())
}

fn json_row(json: &J) -> Result<Row, RuntimeError> {
    json.as_array()
        .ok_or_else(|| RuntimeError::Other(format!("a recorded row `{json}` is not an array")))?
        .iter()
        .map(json_value)
        .collect()
}

/// The row without its trailing revision, with the matching column names.
fn split_revision<'t>(table: &'t TableRef<'_>, mut row: Row) -> Result<(&'t [String], Row), RuntimeError> {
    if row.len() != table.columns.len() {
        return Err(RuntimeError::Other(format!(
            "table `{}`: a row has {} values for {} columns",
            table.name,
            row.len(),
            table.columns.len()
        )));
    }
    row.pop();
    Ok((&table.columns[..table.columns.len() - 1], row))
}

fn revision_of(table: &TableRef<'_>, row: &Row) -> Result<i64, RuntimeError> {
    match row.last() {
        Some(DbValue::Integer(revision)) => Ok(*revision),
        _ => Err(RuntimeError::Other(format!("table `{}`: a row has no revision", table.name))),
    }
}

fn key_value(table: &TableRef<'_>, row: &[DbValue]) -> Result<DbValue, RuntimeError> {
    let index = table
        .columns
        .iter()
        .position(|c| c == table.key)
        .ok_or_else(|| RuntimeError::Other(format!("table `{}` has no key column `{}`", table.name, table.key)))?;
    row.get(index)
        .cloned()
        .ok_or_else(|| RuntimeError::Other(format!("table `{}`: a row has no key value", table.name)))
}

/// Name the missing table and the commands that create it.
fn named(table: &TableRef<'_>, error: RuntimeError) -> RuntimeError {
    let text = error.to_string();
    if text.contains("no such table") {
        return RuntimeError::Other(format!(
            "table `{}` does not exist in its database yet; run `corvid migrate plan`, review the migration it writes, then `corvid migrate up`",
            table.name
        ));
    }
    error
}
