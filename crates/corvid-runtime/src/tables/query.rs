//! Table queries (B09 P2): the SQL behind `Shelf.where(...)` chains.
//!
//! The compiler has already reduced a `where` condition to comparisons,
//! string tests, `and`, `or` and `not` over columns, with every compared
//! value computed by the agent. This module turns that into one
//! `SELECT` whose values are all bound parameters, never SQL text.
//!
//! Results are always in a defined order: by the `order_by` column, then
//! by key, so equal sort values keep a stable order; without `order_by`,
//! by key alone. Equality matches missing values (`IS` on SQLite,
//! `IS NOT DISTINCT FROM` on Postgres), so `s.due == None` finds the rows
//! with no value, and a condition is never left undecided by `NULL`.

use super::{call_name, json_row, quote, row_json, value_json, ColumnKind, Row, TableRef};
use crate::db::DbValue;
use crate::errors::RuntimeError;
use crate::Runtime;
use corvid_types::table_ops::{QueryCompare, QueryTextTest};
use serde_json::Value as J;

/// A `where` condition with its values computed.
#[derive(Debug, Clone, PartialEq)]
pub enum TableFilter {
    Const(bool),
    /// A `Bool` column read on its own.
    Column(String),
    And(Box<TableFilter>, Box<TableFilter>),
    Or(Box<TableFilter>, Box<TableFilter>),
    Not(Box<TableFilter>),
    Compare { column: String, op: QueryCompare, value: DbValue },
    Text { column: String, test: QueryTextTest, value: DbValue },
    /// The column holds one of `values` (J3), each matched as `==`
    /// matches it, a missing value included.
    In { column: String, values: Vec<DbValue> },
}

/// The most values one `contains` list sends to the database. Each is a
/// bound parameter, and both engines cap a statement's parameters.
pub const MOST_LIST_VALUES: usize = 10_000;

/// A whole query chain, ready to run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableQuery {
    /// `None` for `all()`.
    pub filter: Option<TableFilter>,
    /// The column `order_by` reads.
    pub order: Option<String>,
    pub descending: bool,
    pub take: Option<i64>,
}

/// What an aggregate stage asks the database (aggregates J1).
#[derive(Debug, Clone, PartialEq)]
pub enum TableAggregate {
    /// `count()`.
    Count,
    /// `sum` of an `Int` column, added by the database, which refuses a
    /// total that leaves 64 bits on both engines.
    SumInteger(String),
    /// The column's stored values alone, for a sum the interpreter adds
    /// exactly with its own `+` (`Decimal` and `Duration`: SQLite stores
    /// both as text, so its `sum` would add floats).
    Values(String),
    /// `count_by` (J2): each value of the key column with its row count.
    CountBy(String),
    /// `sum_by` of an `Int` column: each key with its total, added by the
    /// database as [`Self::SumInteger`] adds one.
    SumIntegerBy { key: String, column: String },
    /// `sum_by` of a `Decimal` or `Duration` column: each row's key and
    /// value, for the interpreter to add exactly per key.
    ValuesBy { key: String, column: String },
}

impl TableAggregate {
    /// The grouping column, for the stages that group.
    fn key(&self) -> Option<&String> {
        match self {
            Self::CountBy(key) | Self::SumIntegerBy { key, .. } | Self::ValuesBy { key, .. } => Some(key),
            _ => None,
        }
    }

    /// The columns each result row holds, as the statement names them.
    fn result_columns(&self, table: &TableRef<'_>) -> (Vec<String>, Vec<ColumnKind>) {
        let mut columns = Vec::new();
        let mut kinds = Vec::new();
        if let Some(key) = self.key() {
            columns.push(key.clone());
            kinds.push(column_kind_of(table, key));
        }
        match self {
            Self::Values(column) | Self::ValuesBy { column, .. } => {
                columns.push(column.clone());
                kinds.push(column_kind_of(table, column));
            }
            _ => {
                columns.push(AGGREGATE_COLUMN.to_string());
                kinds.push(ColumnKind::Integer);
            }
        }
        (columns, kinds)
    }
}

/// The statement for an aggregate over the rows `filter` keeps and its
/// parameters. Grouped results come in the key's order, so the `Map`
/// they build is the same on every run and every engine.
pub fn aggregate_sql(
    table: &TableRef<'_>,
    filter: Option<&TableFilter>,
    aggregate: &TableAggregate,
) -> (String, Vec<DbValue>) {
    let mut params = Vec::new();
    let read = |column: &String| {
        let kind = column_kind_of(table, column);
        table.engine.select_list(&TableRef { columns: std::slice::from_ref(column), kinds: std::slice::from_ref(&kind), ..*table })
    };
    // `sum` of no rows is `NULL`; an empty total is 0. Postgres sums a
    // `bigint` as `numeric`, and the cast back refuses one past 64 bits.
    let integer_sum = |column: &String| match table.engine.is_postgres() {
        true => format!("coalesce(sum({}), 0)::bigint AS {}", quote(column), quote(AGGREGATE_COLUMN)),
        false => format!("coalesce(sum({}), 0) AS {}", quote(column), quote(AGGREGATE_COLUMN)),
    };
    let count = format!("count(*) AS {}", quote(AGGREGATE_COLUMN));
    let selected = match aggregate {
        TableAggregate::Count => count,
        TableAggregate::SumInteger(column) => integer_sum(column),
        TableAggregate::Values(column) => read(column),
        TableAggregate::CountBy(key) => format!("{}, {count}", read(key)),
        TableAggregate::SumIntegerBy { key, column } => format!("{}, {}", read(key), integer_sum(column)),
        TableAggregate::ValuesBy { key, column } => format!("{}, {}", read(key), read(column)),
    };
    let mut sql = format!("SELECT {selected} FROM {}", quote(table.name));
    if let Some(filter) = filter {
        sql.push_str(" WHERE ");
        sql.push_str(&filter_sql(table, filter, &mut params));
    }
    let table_name = quote(table.name);
    match aggregate {
        TableAggregate::CountBy(key) | TableAggregate::SumIntegerBy { key, .. } => {
            sql.push_str(&format!(" GROUP BY {table_name}.{key} ORDER BY {table_name}.{key} ASC", key = quote(key)));
        }
        TableAggregate::ValuesBy { key, .. } => {
            sql.push_str(&format!(" ORDER BY {table_name}.{} ASC", quote(key)));
        }
        _ => {}
    }
    (sql, params)
}

/// The name an aggregate's single result column is read back by.
const AGGREGATE_COLUMN: &str = "corvid_aggregate";

fn column_kind_of(table: &TableRef<'_>, column: &str) -> ColumnKind {
    table.columns.iter().position(|c| c == column).map(|i| table.kinds[i]).unwrap_or(ColumnKind::Text)
}

/// The `SELECT` statement for `query` and its parameters.
pub fn query_sql(table: &TableRef<'_>, query: &TableQuery) -> Result<(String, Vec<DbValue>), RuntimeError> {
    let mut params = Vec::new();
    let mut sql = format!("SELECT {} FROM {}", table.engine.select_list(table), quote(table.name));
    if let Some(filter) = &query.filter {
        sql.push_str(" WHERE ");
        sql.push_str(&filter_sql(table, filter, &mut params));
    }
    // The `order_by` column first, then the key's columns, so equal sort
    // values keep key order; without `order_by`, key order alone. Each is
    // named with its table: Postgres resolves a bare name in `ORDER BY` to
    // an output column first, and the select list reads money and time
    // columns as text, which would sort `9.50` after `10.0`.
    let table_name = quote(table.name);
    let mut order = Vec::new();
    if let Some(column) = &query.order {
        let direction = if query.descending { "DESC" } else { "ASC" };
        order.push(format!("{table_name}.{} {direction}", quote(column)));
    }
    for key in table.keys.iter().filter(|key| query.order.as_ref() != Some(*key)) {
        order.push(format!("{table_name}.{} ASC", quote(key)));
    }
    sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
    if let Some(count) = query.take {
        if count < 0 {
            return Err(RuntimeError::Other(format!(
                "`{}` query: `take({count})` asks for a negative number of rows; `take` needs 0 or more",
                table.name
            )));
        }
        params.push(DbValue::Integer(count));
        sql.push_str(&format!(" LIMIT {}", table.engine.count_mark(params.len())));
    }
    Ok((sql, params))
}

fn filter_sql(table: &TableRef<'_>, filter: &TableFilter, params: &mut Vec<DbValue>) -> String {
    let engine = table.engine;
    // A value's parameter, typed as the column it is compared with.
    let mut bind = |column: &str, value: &DbValue| {
        params.push(value.clone());
        let kind = table
            .columns
            .iter()
            .position(|c| c == column)
            .map(|i| table.kinds[i])
            .unwrap_or(ColumnKind::Text);
        engine.mark(params.len(), kind, value)
    };
    match filter {
        TableFilter::Const(value) => engine.literal_bool(*value).into(),
        TableFilter::Column(column) => engine.bool_column(&quote(column)),
        TableFilter::And(left, right) => {
            format!("({} AND {})", filter_sql(table, left, params), filter_sql(table, right, params))
        }
        TableFilter::Or(left, right) => {
            format!("({} OR {})", filter_sql(table, left, params), filter_sql(table, right, params))
        }
        TableFilter::Not(inner) => format!("(NOT {})", filter_sql(table, inner, params)),
        TableFilter::Compare { column, op, value } => {
            let mark = bind(column, value);
            let column = quote(column);
            match op {
                QueryCompare::Eq => engine.same(&column, &mark),
                QueryCompare::NotEq => engine.different(&column, &mark),
                QueryCompare::Lt => format!("{column} < {mark}"),
                QueryCompare::LtEq => format!("{column} <= {mark}"),
                QueryCompare::Gt => format!("{column} > {mark}"),
                QueryCompare::GtEq => format!("{column} >= {mark}"),
            }
        }
        TableFilter::Text { column, test, value } => {
            let mark = bind(column, value);
            let column = quote(column);
            match test {
                QueryTextTest::Contains => engine.contains(&column, &mark),
                QueryTextTest::StartsWith => engine.starts_with(&column, &mark),
                QueryTextTest::EndsWith => engine.ends_with(&column, &mark),
            }
        }
        TableFilter::In { column, values } => {
            // `IN` never matches `NULL`, while `==` matches a missing value
            // with `None`, so a `None` in the list is its own test.
            let present: Vec<&DbValue> = values.iter().filter(|v| **v != DbValue::Null).collect();
            let missing = values.len() != present.len();
            let marks: Vec<String> = present.iter().map(|value| bind(column, value)).collect();
            let quoted = quote(column);
            let mut tests = Vec::new();
            if !marks.is_empty() {
                tests.push(format!("{quoted} IN ({})", marks.join(", ")));
            }
            if missing {
                tests.push(format!("{quoted} IS NULL"));
            }
            match tests.len() {
                // An empty list holds nothing.
                0 => engine.literal_bool(false).into(),
                1 => tests.remove(0),
                _ => format!("({})", tests.join(" OR ")),
            }
        }
    }
}

impl Runtime {
    /// Run a query chain, recorded as `table:<Table>.query` with its
    /// SQL and parameters; a replay serves the recorded rows.
    pub async fn table_query(&self, table: &TableRef<'_>, query: &TableQuery) -> Result<Vec<Row>, RuntimeError> {
        let (sql, params) = query_sql(table, query)?;
        let args = vec![J::String(sql.clone()), J::Array(params.iter().map(value_json).collect())];
        let outcome = self
            .recorded_table_call(&call_name(table, "query"), args, || async {
                let rows = self.table_select(table, &sql, &params).await?;
                Ok(J::Array(rows.iter().map(row_json).collect()))
            })
            .await?;
        outcome
            .as_array()
            .ok_or_else(|| RuntimeError::Other(format!("the recorded outcome of `{}.query` is malformed", table.name)))?
            .iter()
            .map(json_row)
            .collect()
    }

    /// Run an aggregate (aggregates J1, J2), recorded as
    /// `table:<Table>.aggregate` with its SQL and parameters; a replay
    /// serves the recorded rows. Each row holds the grouping key first,
    /// for the grouped stages, then the count, the total or the column's
    /// value.
    pub async fn table_aggregate(
        &self,
        table: &TableRef<'_>,
        filter: Option<&TableFilter>,
        aggregate: &TableAggregate,
    ) -> Result<Vec<Row>, RuntimeError> {
        let (sql, params) = aggregate_sql(table, filter, aggregate);
        // The statement's columns, read back by their names.
        let (columns, kinds) = aggregate.result_columns(table);
        let selected = TableRef { columns: &columns, kinds: &kinds, ..*table };
        let args = vec![J::String(sql.clone()), J::Array(params.iter().map(value_json).collect())];
        let outcome = self
            .recorded_table_call(&call_name(table, "aggregate"), args, || async {
                let rows = self.table_select(&selected, &sql, &params).await?;
                Ok(J::Array(rows.iter().map(row_json).collect()))
            })
            .await?;
        let malformed = || RuntimeError::Other(format!("the recorded outcome of `{}.aggregate` is malformed", table.name));
        let rows = outcome.as_array().ok_or_else(malformed)?.iter().map(json_row).collect::<Result<Vec<_>, _>>()?;
        if rows.iter().any(|row| row.len() != columns.len()) {
            return Err(malformed());
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::DatabaseLocation;

    fn shelf() -> (Vec<String>, DatabaseLocation) {
        (vec!["id".into(), "title".into(), "count".into(), "revision".into()], DatabaseLocation::Path(":memory:".into()))
    }

    static ID: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| vec!["id".to_string()]);

    const KINDS: &[ColumnKind] = &[ColumnKind::Integer, ColumnKind::Text, ColumnKind::Integer, ColumnKind::Integer];

    fn table<'a>(columns: &'a [String], location: &'a DatabaseLocation, engine: super::super::TableEngine) -> TableRef<'a> {
        TableRef { name: "Shelf", database: "library", keys: ID.as_slice(), columns, kinds: KINDS, unique: &[], location, engine, transaction: None, row_key: None }
    }

    #[test]
    fn a_filter_binds_every_value_as_a_parameter() {
        let (columns, location) = shelf();
        let table = table(&columns, &location, super::super::TableEngine::Sqlite);
        let query = TableQuery {
            filter: Some(TableFilter::And(
                Box::new(TableFilter::Compare { column: "count".into(), op: QueryCompare::Gt, value: DbValue::Integer(2) }),
                Box::new(TableFilter::Text {
                    column: "title".into(),
                    test: QueryTextTest::StartsWith,
                    value: DbValue::Text("x'; DROP".into()),
                }),
            )),
            order: Some("title".into()),
            descending: true,
            take: Some(5),
        };
        let (sql, params) = query_sql(&table, &query).unwrap();
        assert_eq!(
            sql,
            "SELECT \"id\", \"title\", \"count\", \"revision\" FROM \"Shelf\" WHERE (\"count\" > ?1 AND substr(\"title\", 1, length(?2)) = ?2) ORDER BY \"Shelf\".\"title\" DESC, \"Shelf\".\"id\" ASC LIMIT ?3"
        );
        assert_eq!(params, vec![DbValue::Integer(2), DbValue::Text("x'; DROP".into()), DbValue::Integer(5)]);
    }

    #[test]
    fn equality_uses_is_so_none_matches_missing_values() {
        let (columns, location) = shelf();
        let table = table(&columns, &location, super::super::TableEngine::Sqlite);
        let query = TableQuery {
            filter: Some(TableFilter::Compare { column: "title".into(), op: QueryCompare::Eq, value: DbValue::Null }),
            ..TableQuery::default()
        };
        let (sql, _) = query_sql(&table, &query).unwrap();
        assert!(sql.ends_with("WHERE \"title\" IS ?1 ORDER BY \"Shelf\".\"id\" ASC"), "{sql}");
    }

    #[test]
    fn a_negative_take_is_refused() {
        let (columns, location) = shelf();
        let table = table(&columns, &location, super::super::TableEngine::Sqlite);
        let query = TableQuery { take: Some(-1), ..TableQuery::default() };
        let error = query_sql(&table, &query).unwrap_err().to_string();
        assert!(error.contains("take(-1)"), "{error}");
    }
}
