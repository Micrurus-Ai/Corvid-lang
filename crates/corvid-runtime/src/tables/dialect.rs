//! Each engine's SQL for the same table operation (B09 P4).
//!
//! An operation means the same thing on every engine; only its spelling
//! differs. Parameters are `?1` on SQLite and `$1` on Postgres, where
//! each also carries the column's type so Postgres never has to guess
//! it. Equality that matches missing values is `IS` on SQLite and
//! `IS NOT DISTINCT FROM` on Postgres. JSON-stored columns are `JSONB` on
//! Postgres and are read back as text, so both engines hand the
//! interpreter the same values.

use super::{quote, ColumnKind, TableRef};
use crate::db::DbValue;

/// The engine a table's database runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableEngine {
    Sqlite,
    /// `tls` is whether the connection must be encrypted, as the
    /// `database` block's `tls:` setting declares.
    Postgres { tls: bool },
}

impl TableEngine {
    /// The engine a `database` block names, with its `tls:` setting.
    pub fn from_declaration(engine: &str, tls: Option<&str>) -> Option<Self> {
        match engine {
            "sqlite" => Some(Self::Sqlite),
            "postgres" => Some(Self::Postgres { tls: tls != Some("disabled") }),
            _ => None,
        }
    }

    /// The trace names the engine in each table call's arguments, so a
    /// replay is checked against the engine that recorded it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres { .. } => "postgres",
        }
    }

    fn is_postgres(self) -> bool {
        matches!(self, Self::Postgres { .. })
    }

    /// The `n`th parameter, bound to a column of `kind`. A `Float`
    /// compared with an `Int` column stays a float, so both engines
    /// compare the two as numbers.
    pub fn mark(self, n: usize, kind: ColumnKind, value: &DbValue) -> String {
        match self {
            Self::Sqlite => format!("?{n}"),
            // JSON, money and time travel as text and are cast in SQL to
            // the column's type, because the client's binary encodings of
            // `jsonb`, `numeric` and the domains are not text.
            Self::Postgres { .. } if kind == ColumnKind::Json || kind.is_exact_text() => {
                format!("(${n}::text)::{}", postgres_type(kind))
            }
            Self::Postgres { .. } => {
                let kind = match (kind, value) {
                    (ColumnKind::Integer, DbValue::Float(_)) => ColumnKind::Real,
                    (kind, _) => kind,
                };
                format!("${n}::{}", postgres_type(kind))
            }
        }
    }

    /// The `n`th parameter when it is a row count, as `LIMIT` takes.
    pub fn count_mark(self, n: usize) -> String {
        self.mark(n, ColumnKind::Integer, &DbValue::Integer(0))
    }

    /// `left` equals `right`, where either may be missing.
    pub fn same(self, left: &str, right: &str) -> String {
        let op = if self.is_postgres() { "IS NOT DISTINCT FROM" } else { "IS" };
        format!("{left} {op} {right}")
    }

    /// `left` differs from `right`, where either may be missing.
    pub fn different(self, left: &str, right: &str) -> String {
        let op = if self.is_postgres() { "IS DISTINCT FROM" } else { "IS NOT" };
        format!("{left} {op} {right}")
    }

    pub fn literal_bool(self, value: bool) -> &'static str {
        match (self.is_postgres(), value) {
            (true, true) => "TRUE",
            (true, false) => "FALSE",
            (false, true) => "1",
            (false, false) => "0",
        }
    }

    /// A `Bool` column read as a condition.
    pub fn bool_column(self, column: &str) -> String {
        if self.is_postgres() { column.to_string() } else { format!("{column} <> 0") }
    }

    pub fn contains(self, column: &str, needle: &str) -> String {
        if self.is_postgres() {
            format!("strpos({column}, {needle}) > 0")
        } else {
            format!("instr({column}, {needle}) > 0")
        }
    }

    pub fn starts_with(self, column: &str, prefix: &str) -> String {
        if self.is_postgres() {
            format!("left({column}, length({prefix})) = {prefix}")
        } else {
            format!("substr({column}, 1, length({prefix})) = {prefix}")
        }
    }

    pub fn ends_with(self, column: &str, suffix: &str) -> String {
        if self.is_postgres() {
            format!("right({column}, length({suffix})) = {suffix}")
        } else {
            format!("(length({suffix}) = 0 OR substr({column}, -length({suffix})) = {suffix})")
        }
    }

    /// The column list a `SELECT` reads, in [`TableRef::columns`]
    /// order. Postgres reads `JSONB` and the money and time domains as
    /// text, which is how the interpreter reads them on every engine.
    pub fn select_list(self, table: &TableRef<'_>) -> String {
        table
            .columns
            .iter()
            .zip(table.kinds)
            .map(|(column, kind)| match (self.is_postgres(), kind) {
                (true, kind) if *kind == ColumnKind::Json || kind.is_exact_text() => {
                    format!("{}::text AS {}", quote(column), quote(column))
                }
                _ => quote(column),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The SQL type a column of `kind` declares on this engine, with the
    /// collation it compares by where SQLite's text order is not the
    /// value's order.
    pub fn column_type(self, kind: ColumnKind) -> String {
        match (self, kind.sqlite_collation()) {
            (Self::Sqlite, Some(collation)) => format!("{} COLLATE {collation}", kind.sql_type()),
            (Self::Sqlite, None) => kind.sql_type().to_string(),
            (Self::Postgres { .. }, _) => postgres_type(kind).to_string(),
        }
    }

    /// The kind a stored column's type names on this engine, if Corvid
    /// created it: the inverse of [`Self::column_type`].
    pub fn kind_of(self, sql_type: &str) -> Option<ColumnKind> {
        match self {
            Self::Sqlite => ColumnKind::from_sql_type(sql_type),
            Self::Postgres { .. } => ColumnKind::ALL
                .into_iter()
                .find(|kind| postgres_type(*kind).eq_ignore_ascii_case(sql_type.trim())),
        }
    }

    /// The statement that creates the Postgres domain a column of `kind`
    /// is declared with (B09 P8b), or `None` on SQLite and for kinds
    /// Postgres stores in one of its own types. It does nothing when the
    /// domain already exists, so every table and migration that needs one
    /// can begin with it.
    pub fn domain_definition(self, kind: ColumnKind) -> Option<String> {
        if !self.is_postgres() {
            return None;
        }
        let base = postgres_domain_base(kind)?;
        Some(format!(
            "DO $$ BEGIN CREATE DOMAIN {} AS {base}; EXCEPTION WHEN duplicate_object THEN NULL; END $$;",
            postgres_type(kind)
        ))
    }

    /// Whether a column's type or nullability changes by rebuilding the
    /// table (SQLite, which cannot alter a column) or in place.
    pub fn rebuilds_to_alter(self) -> bool {
        !self.is_postgres()
    }
}

fn postgres_type(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Integer => "bigint",
        ColumnKind::Real => "double precision",
        ColumnKind::Text => "text",
        ColumnKind::Boolean => "boolean",
        ColumnKind::Json => "jsonb",
        ColumnKind::Bytes => "bytea",
        // B09 P8b — a domain each, so the stored schema says which Corvid
        // type a column holds; see `postgres_domain_base`.
        ColumnKind::Decimal => "corvid_decimal",
        ColumnKind::Duration => "corvid_duration",
        ColumnKind::Instant => "corvid_instant",
        ColumnKind::ZonedTime => "corvid_zoned_time",
        ColumnKind::TimeZone => "corvid_time_zone",
        ColumnKind::Currency => "corvid_currency",
        ColumnKind::Locale => "corvid_locale",
    }
}

/// What each money and time domain is on Postgres. A decimal is
/// `numeric`, exact at any scale and compared by value; a duration is its
/// count of nanoseconds, which needs 39 digits. The text forms compare in
/// byte order (`COLLATE "C"`), so a locale's rules never reorder an
/// instant's fixed-width text, and an instant must have that form.
fn postgres_domain_base(kind: ColumnKind) -> Option<&'static str> {
    Some(match kind {
        ColumnKind::Decimal => "numeric",
        ColumnKind::Duration => "numeric(39, 0)",
        ColumnKind::Instant => {
            r#"text COLLATE "C" CHECK (VALUE ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{9}Z$')"#
        }
        ColumnKind::ZonedTime | ColumnKind::TimeZone | ColumnKind::Currency | ColumnKind::Locale => r#"text COLLATE "C""#,
        _ => return None,
    })
}
