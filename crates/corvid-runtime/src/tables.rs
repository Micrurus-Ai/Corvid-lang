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

mod conflict;
mod dialect;
mod live;
mod live_order;
pub mod exact_columns;
mod indexes;
mod migration;
mod pool;
mod postgres;
mod query;
mod recorded;
mod schema;
mod shared_commits;
mod transaction;
pub(crate) use shared_commits::new_instance_id;
pub use shared_commits::LISTENER_NAME;
pub use conflict::WriteConflict;
pub use live::{apply_live_changes, live_changes, LiveChange, LiveChangeKind, LiveMessage, LiveRow, TableCommit, WrittenRows};
pub use live_order::{merge_rechecked, OrderColumn, RowOrder, MOST_ROWS_RECHECKED};
pub use dialect::TableEngine;
pub use indexes::{derive_indexes, managed_prefix, plan_index_changes, unique_prefix, QueryShape, TableIndex};
pub use exact_columns::{parse_zoned_time_column_text, register_sqlite_collations, zoned_time_column_text};
pub use transaction::TableTransaction;
pub use migration::{plan_table_change, StoredColumn, TableChange};
pub use query::{aggregate_sql, query_sql, TableAggregate, TableFilter, TableQuery, MOST_LIST_VALUES};
pub use schema::{column_kind, create_table_sql, create_table_sql_on, untainted, ColumnKind, TableSchema};
pub use postgres::VerifiedTls;
use recorded::{call_name, json_row, row_json, value_json};
use schema::{create_table_named, quote};

/// One table as an operation sees it.
#[derive(Debug, Clone, Copy)]
pub struct TableRef<'a> {
    pub name: &'a str,
    /// The name of the database it lives in, which a commit announces.
    pub database: &'a str,
    /// The key's columns, in declaration order: one, or several that
    /// identify a row together.
    pub keys: &'a [String],
    /// Column names in declaration order, with `revision` last.
    pub columns: &'a [String],
    /// How each column is stored, in [`Self::columns`] order.
    pub kinds: &'a [ColumnKind],
    /// Each `unique` rule's columns (B09 P10).
    pub unique: &'a [Vec<String>],
    pub location: &'a DatabaseLocation,
    pub engine: TableEngine,
    /// The `atomic:` block this operation runs inside, if any.
    pub transaction: Option<&'a TableTransaction>,
    /// For a write, the row's key as live subscriptions identify it
    /// ([`LiveRow::key`]: the key fields' public JSON as compact array
    /// text), so the commit can name the row and the revision it
    /// reached. The interpreter computes it with the encoder the live
    /// snapshot uses; `None` leaves the row out of the commit's detail.
    pub row_key: Option<&'a str>,
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
    /// The pool each operation borrows a connection from.
    Postgres(Arc<pool::PostgresPool>),
    /// The connection an `atomic:` block holds until it ends.
    PostgresHeld(Arc<tokio::sync::Mutex<pool::BorrowedPostgres>>),
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
        if let Some(transaction) = table.transaction {
            if transaction.id != id {
                return Err(RuntimeError::Other(format!(
                    "table `{}` is not in the database its `atomic:` block writes to",
                    table.name
                )));
            }
            // A replay serves every operation before reaching here.
            return transaction.connection.clone().ok_or_else(|| {
                RuntimeError::Other("an `atomic:` block has no connection outside a replay".into())
            });
        }
        self.shared_connection(&location, table.engine)
    }

    /// The connection every operation on a database shares: SQLite's
    /// handle, or Postgres's pool. `atomic:` blocks borrow from the same
    /// pool, so the declared `connections:` bounds them too.
    fn shared_connection(&self, location: &str, engine: TableEngine) -> Result<TableConnection, RuntimeError> {
        let id = format!("{}:{location}", engine.name());
        if let Some(connection) = self.table_databases.connections.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
            return Ok(connection.clone());
        }
        let connection = match engine {
            TableEngine::Sqlite => TableConnection::Sqlite(self.db_registry().open(location)?),
            TableEngine::Postgres { tls, connections } => {
                TableConnection::Postgres(Arc::new(pool::PostgresPool::new(location.to_string(), tls, connections)))
            }
        };
        let mut connections = self.table_databases.connections.lock().unwrap_or_else(|e| e.into_inner());
        Ok(connections.entry(id).or_insert(connection).clone())
    }

    /// Run a write; the number of rows it changed.
    async fn table_execute(&self, table: &TableRef<'_>, sql: &str, params: &[DbValue]) -> Result<u64, RuntimeError> {
        let changed = match self.table_connection(table).await.map_err(|e| named(table, e))? {
            TableConnection::Sqlite(handle) => {
                self.db_registry().execute_queued(&handle, sql, params).await.map(|r| r.rows_affected)
            }
            TableConnection::Postgres(pool) => pool.borrow().await?.connection().execute(sql, params).await,
            TableConnection::PostgresHeld(held) => held.lock().await.connection().execute(sql, params).await,
        };
        changed.map_err(|e| named(table, e))
    }

    /// Run a read; its rows in [`TableRef::columns`] order.
    async fn table_select(&self, table: &TableRef<'_>, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, RuntimeError> {
        let rows = match self.table_connection(table).await.map_err(|e| named(table, e))? {
            TableConnection::Sqlite(handle) => self
                .db_registry()
                .query_table_rows(handle.handle_id, sql, params)
                .map(|rows| rows.rows.into_iter().map(|cells| ordered_row(table, cells)).collect()),
            TableConnection::Postgres(pool) => pool.borrow().await?.connection().select(sql, params, table.columns).await,
            TableConnection::PostgresHeld(held) => held.lock().await.connection().select(sql, params, table.columns).await,
        };
        rows.map_err(|e| named(table, e))
    }

    /// `Shelf.get(key)`, with one value per key column.
    async fn table_get_live(&self, table: &TableRef<'_>, key: Vec<DbValue>) -> Result<Option<Row>, RuntimeError> {
        self.select_row(table, key).await
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Taken` with the row that holds its key or a `unique` value.
    async fn table_insert_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
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
        let key = key_values(table, &values)?;
        if inserted == 1 {
            values.push(DbValue::Integer(1));
            return Ok(Ok(values));
        }
        if let Some(existing) = self.select_row(table, key.clone()).await? {
            return Ok(Err(WriteConflict::Taken(existing)));
        }
        // Not the key: another row holds a `unique` value.
        match self.unique_holder(table, &values).await? {
            Some(holder) => Ok(Err(WriteConflict::Taken(holder))),
            None => Err(RuntimeError::Other(format!("table `{}`: an insert conflicted with a row that is gone", table.name))),
        }
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or why not: `Changed` with the row stored now, `Deleted`, or
    /// `Taken` with the row that holds one of its `unique` values.
    async fn table_save_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
        let revision = revision_of(table, &row)?;
        let (columns, mut values) = split_revision(table, row)?;
        let key = key_values(table, &values)?;
        let mark = |i: usize| table.engine.mark(i + 1, table.kinds[i], &values[i]);
        let assignments = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| !table.keys.contains(c))
            .map(|(i, c)| format!("{} = {}", quote(c), mark(i)))
            .collect::<Vec<_>>()
            .join(", ");
        let matches_key = key_indices(table)?
            .into_iter()
            .map(|i| format!("{} = {}", quote(&table.columns[i]), mark(i)))
            .collect::<Vec<_>>()
            .join(" AND ");
        let revision_mark = table.engine.count_mark(values.len() + 1);
        let revision_column = quote(corvid_ast::TableDecl::REVISION_FIELD);
        let set = if assignments.is_empty() { String::new() } else { format!("{assignments}, ") };
        // A write that would give the row another row's `unique` values
        // changes nothing, so it can be reported as a value instead of a
        // database error that would end an `atomic:` block's transaction.
        let unique_free: String = table
            .unique
            .iter()
            .map(|rule| format!(" AND NOT EXISTS ({})", unique_holder_sql(table, rule, &mut |c| mark(column_index(table, c)))))
            .collect();
        let sql = format!(
            "UPDATE {} SET {set}{revision_column} = {revision_column} + 1 WHERE {matches_key} AND {revision_column} = {revision_mark}{unique_free}",
            quote(table.name),
        );
        let mut params = values.clone();
        params.push(DbValue::Integer(revision));
        let updated = self.table_execute(table, &sql, &params).await?;
        if updated == 1 {
            values.push(DbValue::Integer(revision + 1));
            return Ok(Ok(values));
        }
        let current = self.select_row(table, key.clone()).await?;
        // Still at the revision this write saw: what blocked it is the row
        // holding one of its `unique` values.
        if let Some(stored) = &current {
            if revision_of(table, stored)? == revision {
                if let Some(holder) = self.unique_holder(table, &values).await? {
                    return Ok(Err(WriteConflict::Taken(holder)));
                }
            }
        }
        Ok(Err(WriteConflict::stale(current)))
    }

    /// Another row (not the one with `key`) holding all of some `unique`
    /// rule's values in `values`, the row's columns without its revision.
    async fn unique_holder(&self, table: &TableRef<'_>, values: &[DbValue]) -> Result<Option<Row>, RuntimeError> {
        for rule in table.unique {
            let mut params = Vec::new();
            let mut mark_of = |column: &str| {
                let index = column_index(table, column);
                params.push(values[index].clone());
                table.engine.mark(params.len(), table.kinds[index], &values[index])
            };
            let sql = unique_holder_sql(table, rule, &mut mark_of).replacen("SELECT 1 ", &format!("SELECT {} ", table.engine.select_list(table)), 1);
            if let Some(holder) = self.table_select(table, &sql, &params).await?.into_iter().next() {
                return Ok(Some(holder));
            }
        }
        Ok(None)
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or `Changed` with
    /// the row stored now, or `Deleted` if it is already gone.
    async fn table_delete_live(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
        let revision = revision_of(table, &row)?;
        let key = key_values(table, &row)?;
        let sql = format!(
            "DELETE FROM {} WHERE {} AND {} = {}",
            quote(table.name),
            key_condition(table, &key, 1)?,
            quote(corvid_ast::TableDecl::REVISION_FIELD),
            table.engine.count_mark(key.len() + 1),
        );
        let mut params = key.clone();
        params.push(DbValue::Integer(revision));
        let deleted = self.table_execute(table, &sql, &params).await?;
        if deleted == 1 {
            return Ok(Ok(row));
        }
        Ok(Err(WriteConflict::stale(self.select_row(table, key).await?)))
    }

    async fn select_row(&self, table: &TableRef<'_>, key: Vec<DbValue>) -> Result<Option<Row>, RuntimeError> {
        let sql = format!(
            "SELECT {} FROM {} WHERE {}",
            table.engine.select_list(table),
            quote(table.name),
            key_condition(table, &key, 1)?,
        );
        Ok(self.table_select(table, &sql, &key).await?.into_iter().next())
    }
}

/// `SELECT 1 FROM Table AS holder WHERE` each of `rule`'s columns equals
/// the value `mark` binds, and the key is not the row's own: another row
/// holding the rule's values. A missing value never matches, so rows
/// missing one of a rule's values never conflict, as in a unique index.
fn unique_holder_sql(table: &TableRef<'_>, rule: &[String], mark: &mut dyn FnMut(&str) -> String) -> String {
    let equal = rule
        .iter()
        .map(|column| format!("\"holder\".{} = {}", quote(column), mark(column)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let own_key = table
        .keys
        .iter()
        .map(|column| format!("\"holder\".{} = {}", quote(column), mark(column)))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!("SELECT 1 FROM {} AS \"holder\" WHERE {equal} AND NOT ({own_key})", quote(table.name))
}

fn column_index(table: &TableRef<'_>, column: &str) -> usize {
    table.columns.iter().position(|c| c == column).expect("a unique or key column is one of the table's columns")
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

/// Where each key column sits in [`TableRef::columns`], in key order.
fn key_indices(table: &TableRef<'_>) -> Result<Vec<usize>, RuntimeError> {
    table
        .keys
        .iter()
        .map(|key| {
            table
                .columns
                .iter()
                .position(|c| c == key)
                .ok_or_else(|| RuntimeError::Other(format!("table `{}` has no key column `{key}`", table.name)))
        })
        .collect()
}

/// A row's key values, in key order.
fn key_values(table: &TableRef<'_>, row: &[DbValue]) -> Result<Vec<DbValue>, RuntimeError> {
    key_indices(table)?
        .into_iter()
        .map(|i| {
            row.get(i)
                .cloned()
                .ok_or_else(|| RuntimeError::Other(format!("table `{}`: a row has no key value", table.name)))
        })
        .collect()
}

/// `k1 = $first AND k2 = $first+1 ...`, matching a row by its key.
fn key_condition(table: &TableRef<'_>, key: &[DbValue], first: usize) -> Result<String, RuntimeError> {
    let indices = key_indices(table)?;
    if key.len() != indices.len() {
        return Err(RuntimeError::Other(format!(
            "table `{}`: a key has {} values for {} key columns",
            table.name,
            key.len(),
            indices.len()
        )));
    }
    Ok(indices
        .iter()
        .zip(key)
        .enumerate()
        .map(|(n, (i, value))| format!("{} = {}", quote(&table.columns[*i]), table.engine.mark(first + n, table.kinds[*i], value)))
        .collect::<Vec<_>>()
        .join(" AND "))
}

/// A failure of a database's connection or transaction, with the
/// database named: an outage `try` can retry or catch.
fn named_database(database: &str, error: RuntimeError) -> RuntimeError {
    match error {
        already @ RuntimeError::DatabaseFailed { .. } => already,
        other => RuntimeError::DatabaseFailed { database: database.to_string(), message: other.to_string() },
    }
}

/// A table operation's failure, with its database named, and a missing
/// table told the commands that create it.
fn named(table: &TableRef<'_>, error: RuntimeError) -> RuntimeError {
    if let RuntimeError::DatabaseFailed { .. } = error {
        return error;
    }
    let text = error.to_string();
    let message = if text.contains("no such table") {
        format!(
            "table `{}` does not exist in its database yet; run `corvid migrate plan`, review the migration it writes, then `corvid migrate up`",
            table.name
        )
    } else {
        text
    };
    RuntimeError::DatabaseFailed { database: table.database.to_string(), message }
}
