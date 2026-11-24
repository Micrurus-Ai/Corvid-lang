//! How declared tables are stored (B09): the column kind each field
//! type maps to, the SQL type each kind declares, and the
//! `CREATE TABLE` statement for a declared table.

use super::TableEngine;
use corvid_types::Type;

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
        // B06 — a decision, not a fallthrough: `Decimal` stays JSON-stored
        // until B06 and the main line agree its column mapping (the
        // "Decimal, Instant, Duration and ZonedTime as B09 table columns"
        // roadmap item, after B06's JSON slice). The checker refuses
        // `Decimal` table columns until then.
        Type::Decimal => (ColumnKind::Json, false),
        // B06 — the same deferred decision for the time types.
        Type::Duration | Type::Instant | Type::TimeZone | Type::ZonedTime | Type::Currency | Type::Locale => (ColumnKind::Json, false),
        // The same deferred decision for `Bytes`, which may want a BLOB
        // column; the checker refuses `Bytes` table columns until then.
        Type::Bytes => (ColumnKind::Json, false),
        // A `Secret` is never stored: the checker refuses a `Secret`
        // column (hash it, or wait for an encrypted column). Named here
        // so no default arm decides its storage.
        Type::Secret => (ColumnKind::Json, false),
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
    /// The key's columns, in declaration order.
    pub keys: Vec<String>,
    pub columns: Vec<(String, Type)>,
    /// `(field, old name)` for each `renamed from`.
    pub renamed: Vec<(String, String)>,
    /// Stored columns declared `retired`.
    pub retired: Vec<String>,
    /// The engine its database runs on.
    pub engine: TableEngine,
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
    format!("CREATE TABLE {} (\n{columns}{constraint}\n);\n", quote(name))
}

/// `name` as a quoted SQL identifier.
pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
