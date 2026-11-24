//! Table storage (B09): the SQL behind `Shelf.get`, `Shelf.insert`,
//! `Shelf.save` and `Shelf.delete`.
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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    pub fn sql_type(self) -> &'static str {
        match self {
            Self::Integer | Self::Boolean => "INTEGER",
            Self::Real => "REAL",
            Self::Text | Self::Json => "TEXT",
        }
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
/// `revision` last.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    pub key: String,
    pub columns: Vec<(String, Type)>,
}

/// The `CREATE TABLE` statement for a declared table.
pub fn create_table_sql(table: &TableSchema) -> String {
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
    format!("CREATE TABLE {} (\n{columns}\n);\n", quote(&table.name))
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
    pub async fn table_get(&self, table: &TableRef<'_>, key: DbValue) -> Result<Option<Row>, RuntimeError> {
        let handle = self.table_handle(table)?;
        self.select_row(&handle, table, key)
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Err` with the row already stored under that key.
    pub async fn table_insert(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Row>, RuntimeError> {
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
    pub async fn table_save(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
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
    pub async fn table_delete(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
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
        let mut cells = cells;
        Ok(Some(
            table
                .columns
                .iter()
                .map(|c| cells.remove(c).map(|cell| cell.value).unwrap_or(DbValue::Null))
                .collect(),
        ))
    }
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
