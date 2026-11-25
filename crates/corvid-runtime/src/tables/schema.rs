//! How declared tables are stored (B09): the column kind each field
//! type maps to, the SQL type each kind declares, and the
//! `CREATE TABLE` statement for a declared table.

use super::TableEngine;
use corvid_types::Type;

pub use corvid_types::table_columns::{column_kind, untainted, ColumnKind};

/// A table's declared shape: its name, key and typed columns, with
/// `revision` last, and the history clauses the migration planner reads.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub name: String,
    /// The key's columns, in declaration order.
    pub keys: Vec<String>,
    pub columns: Vec<(String, Type)>,
    /// `(field, old name)` for each `renamed from`.
    pub renamed: Vec<(String, String)>,
    /// Stored columns declared `retired`.
    pub retired: Vec<String>,
    /// The engine its database runs on.
    pub engine: TableEngine,
    /// The indexes its queries need (B09 P9), derived from the program.
    pub indexes: Vec<super::TableIndex>,
}

/// The `CREATE TABLE` statement for a declared table, on SQLite.
pub fn create_table_sql(table: &TableSchema) -> String {
    create_table_sql_on(table, TableEngine::Sqlite)
}

/// The `CREATE TABLE` statement for a declared table on `engine`.
pub fn create_table_sql_on(table: &TableSchema, engine: TableEngine) -> String {
    create_table_named_on(table, &table.name, engine)
}

/// [`create_table_sql`] under another name, for rebuilding a table.
pub(super) fn create_table_named(table: &TableSchema, name: &str) -> String {
    create_table_named_on(table, name, TableEngine::Sqlite)
}

fn create_table_named_on(table: &TableSchema, name: &str, engine: TableEngine) -> String {
    let columns = table
        .columns
        .iter()
        .map(|(name, ty)| {
            let (kind, nullable) = column_kind(ty);
            // A one-column key is declared on its column; a key of several
            // columns is a table constraint after the columns.
            let key = if table.keys.len() == 1 && table.keys[0] == *name { " PRIMARY KEY" } else { "" };
            let null = if nullable { "" } else { " NOT NULL" };
            format!("    {} {}{key}{null}", quote(name), engine.column_type(kind))
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let constraint = if table.keys.len() > 1 {
        let keys = table.keys.iter().map(|key| quote(key)).collect::<Vec<_>>().join(", ");
        format!(",\n    PRIMARY KEY ({keys})")
    } else {
        String::new()
    };
    // Postgres declares money and time columns with domains, created
    // first and only once each.
    let mut domains = Vec::new();
    for (_, ty) in &table.columns {
        if let Some(definition) = engine.domain_definition(column_kind(ty).0) {
            if !domains.contains(&definition) {
                domains.push(definition);
            }
        }
    }
    let domains: String = domains.into_iter().map(|d| d + "\n").collect();
    format!("{domains}CREATE TABLE {} (\n{columns}{constraint}\n);\n", quote(name))
}

/// `name` as a quoted SQL identifier.
pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
