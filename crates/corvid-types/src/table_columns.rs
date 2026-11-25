//! How a table field is stored (B09): the column kind each field type
//! maps to and the SQL type each kind declares on SQLite. The checker
//! decides which columns a query may compare from this, and the runtime
//! and the migration planner store and plan columns from it, so the
//! three never disagree.

use crate::Type;

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
    /// B09 P8a — a `Decimal`, as its exact text (`19.90`), compared by
    /// value through the `corvid_decimal` collation.
    Decimal,
    /// A `Duration`, as its ISO 8601 text (`PT1H30M`), compared by value
    /// through the `corvid_duration` collation. Corvid durations hold
    /// 128-bit nanoseconds, more than an SQLite integer.
    Duration,
    /// An `Instant`, as UTC RFC 3339 text with all nine fractional
    /// digits, which sorts in time order.
    Instant,
    /// A `ZonedTime`, as its instant's sortable text and its zone,
    /// `2026-03-29T01:30:00.000000000Z[Europe/Berlin]`.
    ZonedTime,
    /// A `TimeZone`, as its IANA name.
    TimeZone,
    /// A `Currency`, as its ISO 4217 code.
    Currency,
    /// A `Locale`, as its canonical BCP 47 tag.
    Locale,
    /// B09 P8c — `Bytes`, as the bytes themselves: `BLOB` on SQLite,
    /// `bytea` on Postgres. Compared bytewise, with no order.
    Bytes,
}

impl ColumnKind {
    /// The declared SQL type. Each kind has its own, so the stored
    /// schema records how a column is stored and the migration planner
    /// can see a change from `Int` to `Bool`, from `String` to a
    /// JSON-stored type, or from `String` to `Decimal`, rather than
    /// reading stored values as another type. `BOOLEAN` has SQLite's
    /// numeric affinity, which keeps 0 and 1 as integers. Every name
    /// ending in `TEXT` has text affinity (none contains `INT`), so text
    /// such as `5` is never turned into a number.
    pub fn sql_type(self) -> &'static str {
        match self {
            Self::Integer => "INTEGER",
            Self::Real => "REAL",
            Self::Text => "TEXT",
            Self::Boolean => "BOOLEAN",
            Self::Json => "JSON TEXT",
            Self::Decimal => "DECIMAL TEXT",
            Self::Duration => "DURATION TEXT",
            Self::Instant => "INSTANT TEXT",
            Self::ZonedTime => "ZONED TIME TEXT",
            Self::TimeZone => "TIME ZONE TEXT",
            Self::Currency => "CURRENCY TEXT",
            Self::Locale => "LOCALE TEXT",
            Self::Bytes => "BLOB",
        }
    }

    /// Every kind, for reading a stored column's declared type back.
    pub const ALL: [ColumnKind; 13] = [
        Self::Integer,
        Self::Real,
        Self::Text,
        Self::Boolean,
        Self::Json,
        Self::Decimal,
        Self::Duration,
        Self::Instant,
        Self::ZonedTime,
        Self::TimeZone,
        Self::Currency,
        Self::Locale,
        Self::Bytes,
    ];

    /// The kind a stored column's declared type names, if Corvid
    /// created it.
    pub fn from_sql_type(sql_type: &str) -> Option<Self> {
        let sql_type = sql_type.trim();
        Self::ALL.into_iter().find(|kind| kind.sql_type().eq_ignore_ascii_case(sql_type))
    }

    /// The SQLite collation a column of this kind compares with, where
    /// text order is not value order. Corvid registers each on every
    /// SQLite connection it opens.
    pub fn sqlite_collation(self) -> Option<&'static str> {
        match self {
            Self::Decimal => Some(DECIMAL_COLLATION),
            Self::Duration => Some(DURATION_COLLATION),
            _ => None,
        }
    }

    /// Whether the kind stores one of the money and time types in a
    /// column that compares it exactly (B09 P8a).
    pub fn is_exact_text(self) -> bool {
        matches!(
            self,
            Self::Decimal | Self::Duration | Self::Instant | Self::ZonedTime | Self::TimeZone | Self::Currency | Self::Locale
        )
    }

    /// Whether the type stored has an order in Corvid, so `order_by` may
    /// sort by it. A zoned time compares only by equality, since two in
    /// different zones can be the same instant, and `Bytes` only by
    /// equality, as in Corvid.
    pub fn is_ordered(self) -> bool {
        !matches!(self, Self::ZonedTime | Self::TimeZone | Self::Currency | Self::Locale | Self::Bytes | Self::Json)
    }

    /// The Corvid type a column of this kind holds, for messages.
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Integer => "Int",
            Self::Real => "Float",
            Self::Text => "String",
            Self::Boolean => "Bool",
            Self::Json => "a JSON-stored type",
            Self::Decimal => "Decimal",
            Self::Duration => "Duration",
            Self::Instant => "Instant",
            Self::ZonedTime => "ZonedTime",
            Self::TimeZone => "TimeZone",
            Self::Currency => "Currency",
            Self::Locale => "Locale",
            Self::Bytes => "Bytes",
        }
    }
}

/// The SQLite collation that compares decimal text by value.
pub const DECIMAL_COLLATION: &str = "corvid_decimal";
/// The SQLite collation that compares ISO 8601 duration text by value.
pub const DURATION_COLLATION: &str = "corvid_duration";

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
        Type::Decimal => (ColumnKind::Decimal, false),
        Type::Duration => (ColumnKind::Duration, false),
        Type::Instant => (ColumnKind::Instant, false),
        Type::ZonedTime => (ColumnKind::ZonedTime, false),
        Type::TimeZone => (ColumnKind::TimeZone, false),
        Type::Currency => (ColumnKind::Currency, false),
        Type::Locale => (ColumnKind::Locale, false),
        Type::Bytes => (ColumnKind::Bytes, false),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_reads_back_from_its_declared_type() {
        for kind in ColumnKind::ALL {
            assert_eq!(ColumnKind::from_sql_type(kind.sql_type()), Some(kind), "{kind:?}");
            assert_eq!(ColumnKind::from_sql_type(&kind.sql_type().to_ascii_lowercase()), Some(kind), "{kind:?}");
        }
    }

    #[test]
    fn text_kinds_keep_sqlite_text_affinity() {
        // SQLite gives integer affinity to any declared type containing
        // `INT`, ahead of the `TEXT` rule.
        for kind in ColumnKind::ALL.into_iter().filter(|k| k.sql_type().ends_with("TEXT")) {
            assert!(!kind.sql_type().contains("INT"), "{kind:?}");
        }
    }

    #[test]
    fn money_and_time_get_their_own_columns() {
        assert_eq!(column_kind(&Type::Decimal), (ColumnKind::Decimal, false));
        assert_eq!(column_kind(&Type::Option(Box::new(Type::Instant))), (ColumnKind::Instant, true));
        assert_eq!(column_kind(&Type::Bytes), (ColumnKind::Bytes, false));
        assert!(!ColumnKind::ZonedTime.is_ordered() && !ColumnKind::Bytes.is_ordered() && ColumnKind::Duration.is_ordered());
    }
}
