//! Table storage (B09): the SQL behind `Shelf.get`, `Shelf.insert`,
//! `Shelf.save` and `Shelf.delete`; queries live in [`query`].
//!
//! On SQLite, operations run through the same
//! [`crate::db::DbHandleRegistry`] the `std.db` tools use, so they share
//! its per-file writer lock and parameter-only binding. On Postgres they
//! run on an asynchronous connection per location ([`postgres`]). Either
//! way values never become SQL text, and [`dialect`] spells each
//! statement for its engine. Rows travel as [`DbValue`]s in column
//! order, with `revision` last; the interpreter converts its own values
//! to and from them.
//!
//! Every write is a compare-and-set on `revision`: `save` and `delete`
//! succeed only when the stored row still has the revision the caller
//! read, and each successful write advances it.

use crate::db::{DbHandleInner, DbValue};
use crate::errors::RuntimeError;
use crate::Runtime;
use corvid_ast::DatabaseLocation;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

mod dialect;
mod migration;
mod postgres;
mod query;
mod recorded;
mod schema;
pub use dialect::TableEngine;
pub use migration::{plan_table_change, StoredColumn, TableChange};
pub use query::{query_sql, TableFilter, TableQuery};
pub use schema::{column_kind, create_table_sql, create_table_sql_on, untainted, ColumnKind, TableSchema};
use postgres::PostgresTables;
use recorded::{call_name, json_row, row_json, value_json};
use schema::{create_table_named, quote};

/// One table as an operation sees it.
#[derive(Debug, Clone, Copy)]
pub struct TableRef<'a> {
    pub name: &'a str,
    pub key: &'a str,
    /// Column names in declaration order, with `revision` last.
    pub columns: &'a [String],
    /// How each column is stored, in [`Self::columns`] order.
    pub kinds: &'a [ColumnKind],
    pub location: &'a DatabaseLocation,
    pub engine: TableEngine,
}

/// A row's values, in [`TableRef::columns`] order.
pub type Row = Vec<DbValue>;

/// The open connection for each database, shared by a runtime's table
/// operations.
#[derive(Default, Clone)]
pub struct TableDatabases {
    connections: Arc<Mutex<HashMap<String, TableConnection>>>,
}

#[derive(Clone)]
enum TableConnection {
    Sqlite(Arc<DbHandleInner>),
    Postgres(Arc<PostgresTables>),
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

impl Runtime {
    async fn table_connection(&self, table: &TableRef<'_>) -> Result<TableConnection, RuntimeError> {
        let location = resolve_location(table.location)?;
        let id = format!("{}:{location}", table.engine.name());
        if let Some(connection) = self.table_databases.connections.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            return Ok(connection.clone());
        }
        let connection = match table.engine {
            TableEngine::Sqlite => TableConnection::Sqlite(self.db_registry().open(&location)?),
            TableEngine::Postgres { tls } => TableConnection::Postgres(Arc::new(PostgresTables::connect(&location, tls).await?)),
        };
        let mut connections = self.table_databases.connections.lock().unwrap_or_else(|e| e.into_inner());
        Ok(connections.entry(id).or_insert(connection).clone())
    }

    /// Run a write; the number of rows it changed.
    async fn table_execute(&self, table: &TableRef<'_>, sql: &str, params: &[DbValue]) -> Result<u64, RuntimeError> {
        let changed = match self.table_connection(table).await? {
            TableConnection::Sqlite(handle) => {
                self.db_registry().execute_queued(&handle, sql, params).await.map(|r| r.rows_affected)
            }
            TableConnection::Postgres(connection) => connection.execute(sql, params).await,
        };
        changed.map_err(|e| named(table, e))
    }

    /// Run a read; its rows in [`TableRef::columns`] order.
    async fn table_select(&self, table: &TableRef<'_>, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, RuntimeError> {
        let rows = match self.table_connection(table).await? {
            TableConnection::Sqlite(handle) => self
                .db_registry()
                .query(handle.handle_id, sql, params)
                .map(|rows| rows.rows.into_iter().map(|cells| ordered_row(table, cells)).collect()),
            TableConnection::Postgres(connection) => connection.select(sql, params, table.columns).await,
        };
        rows.map_err(|e| named(table, e))
    }

    /// `Shelf.get(key)`.
    async fn table_get_live(&self, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        self.select_row(table, key).await
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Err` with the row already stored under that key.
    async fn table_insert_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Row>, RuntimeError> {
        let (columns, mut values) = split_revision(table, row)?;
        let names = columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let marks = values
            .iter()
            .enumerate()
            .map(|(i, value)| table.engine.mark(i + 1, table.kinds[i], value))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "INSERT INTO {} ({names}, {}) VALUES ({marks}, 1) ON CONFLICT DO NOTHING",
            quote(table.name),
            quote(corvid_ast::TableDecl::REVISION_FIELD)
        );
        let inserted = self.table_execute(table, &sql, &values).await?;
        let key = key_value(table, &values)?;
        if inserted == 1 {
            values.push(DbValue::Integer(1));
            return Ok(Ok(values));
        }
        let existing = self.select_row(table, key).await?.ok_or_else(|| {
            RuntimeError::Other(format!("table `{}`: an insert conflicted with a row that is gone", table.name))
        })?;
        Ok(Err(existing))
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or `Err` with the row stored now (`None` if it was deleted).
    async fn table_save_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let revision = revision_of(table, &row)?;
        let (columns, mut values) = split_revision(table, row)?;
        let key = key_value(table, &values)?;
        let mark = |i: usize| table.engine.mark(i + 1, table.kinds[i], &values[i]);
        let assignments = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| *c != table.key)
            .map(|(i, c)| format!("{} = {}", quote(c), mark(i)))
            .collect::<Vec<_>>()
            .join(", ");
        let key_mark = mark(key_index(table)?);
        let revision_mark = table.engine.count_mark(values.len() + 1);
        let revision_column = quote(corvid_ast::TableDecl::REVISION_FIELD);
        let set = if assignments.is_empty() { String::new() } else { format!("{assignments}, ") };
        let sql = format!(
            "UPDATE {} SET {set}{revision_column} = {revision_column} + 1 WHERE {} = {key_mark} AND {revision_column} = {revision_mark}",
            quote(table.name),
            quote(table.key),
        );
        let mut params = values.clone();
        params.push(DbValue::Integer(revision));
        let updated = self.table_execute(table, &sql, &params).await?;
        if updated == 1 {
            values.push(DbValue::Integer(revision + 1));
            return Ok(Ok(values));
        }
        Ok(Err(self.select_row(table, key).await?))
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or `Err` with
    /// the row stored now (`None` if it is already gone).
    async fn table_delete_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let revision = revision_of(table, &row)?;
        let key = key_value(table, &row)?;
        let sql = format!(
            "DELETE FROM {} WHERE {} = {} AND {} = {}",
            quote(table.name),
            quote(table.key),
            table.engine.mark(1, table.kinds[key_index(table)?], &key),
            quote(corvid_ast::TableDecl::REVISION_FIELD),
            table.engine.count_mark(2),
        );
        let deleted = self.table_execute(table, &sql, &[key.clone(), DbValue::Integer(revision)]).await?;
        if deleted == 1 {
            return Ok(Ok(row));
        }
        Ok(Err(self.select_row(table, key).await?))
    }

    async fn select_row(&self, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {}",
            table.engine.select_list(table),
            quote(table.name),
            quote(table.key),
            table.engine.mark(1, table.kinds[key_index(table)?], &key),
        );
        Ok(self.table_select(table, &sql, &[key]).await?.into_iter().next())
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

fn key_index(table: &TableRef<'_>) -> Result<usize, RuntimeError> {
    table
        .columns
        .iter()
        .position(|c| c == table.key)
        .ok_or_else(|| RuntimeError::Other(format!("table `{}` has no key column `{}`", table.name, table.key)))
}

fn key_value(table: &TableRef<'_>, row: &[DbValue]) -> Result<DbValue, RuntimeError> {
    row.get(key_index(table)?)
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
