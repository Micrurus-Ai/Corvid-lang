//! Table queries (B09 P2): the SQL behind `Shelf.where(...)` chains.
//!
//! The compiler has already reduced a `where` condition to comparisons,
//! string tests, `and`, `or` and `not` over columns, with every compared
//! value computed by the agent. This module turns that into one
//! `SELECT` whose values are all bound parameters, never SQL text.
//!
//! Results are always in a defined order: by the `order_by` column, then
//! by key, so equal sort values keep a stable order; without `order_by`,
//! by key alone. Equality uses `IS`, so `s.due == None` finds the rows
//! with no value, and a condition is never left undecided by `NULL`.

use super::{call_name, json_row, named, ordered_row, quote, row_json, value_json, Row, TableRef};
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
}

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

/// The `SELECT` statement for `query` and its parameters.
pub fn query_sql(table: &TableRef<'_>, query: &TableQuery) -> Result<(String, Vec<DbValue>), RuntimeError> {
    let mut params = Vec::new();
    let names = table.columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
    let mut sql = format!("SELECT {names} FROM {}", quote(table.name));
    if let Some(filter) = &query.filter {
        sql.push_str(" WHERE ");
        sql.push_str(&filter_sql(filter, &mut params));
    }
    let key = quote(table.key);
    match &query.order {
        Some(column) if column != table.key => {
            let direction = if query.descending { "DESC" } else { "ASC" };
            sql.push_str(&format!(" ORDER BY {} {direction}, {key} ASC", quote(column)));
        }
        Some(_) if query.descending => sql.push_str(&format!(" ORDER BY {key} DESC")),
        _ => sql.push_str(&format!(" ORDER BY {key} ASC")),
    }
    if let Some(count) = query.take {
        if count < 0 {
            return Err(RuntimeError::Other(format!(
                "`{}` query: `take({count})` asks for a negative number of rows; `take` needs 0 or more",
                table.name
            )));
        }
        params.push(DbValue::Integer(count));
        sql.push_str(&format!(" LIMIT ?{}", params.len()));
    }
    Ok((sql, params))
}

fn filter_sql(filter: &TableFilter, params: &mut Vec<DbValue>) -> String {
    match filter {
        TableFilter::Const(true) => "1".into(),
        TableFilter::Const(false) => "0".into(),
        TableFilter::Column(column) => format!("{} <> 0", quote(column)),
        TableFilter::And(left, right) => format!("({} AND {})", filter_sql(left, params), filter_sql(right, params)),
        TableFilter::Or(left, right) => format!("({} OR {})", filter_sql(left, params), filter_sql(right, params)),
        TableFilter::Not(inner) => format!("(NOT {})", filter_sql(inner, params)),
        TableFilter::Compare { column, op, value } => {
            params.push(value.clone());
            let mark = format!("?{}", params.len());
            let op = match op {
                QueryCompare::Eq => "IS",
                QueryCompare::NotEq => "IS NOT",
                QueryCompare::Lt => "<",
                QueryCompare::LtEq => "<=",
                QueryCompare::Gt => ">",
                QueryCompare::GtEq => ">=",
            };
            format!("{} {op} {mark}", quote(column))
        }
        TableFilter::Text { column, test, value } => {
            params.push(value.clone());
            let mark = format!("?{}", params.len());
            let column = quote(column);
            match test {
                QueryTextTest::Contains => format!("instr({column}, {mark}) > 0"),
                QueryTextTest::StartsWith => format!("substr({column}, 1, length({mark})) = {mark}"),
                QueryTextTest::EndsWith => {
                    format!("(length({mark}) = 0 OR substr({column}, -length({mark})) = {mark})")
                }
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
                let handle = self.table_handle(table)?;
                let rows = self.db_registry().query(handle.handle_id, &sql, &params).map_err(|e| named(table, e))?;
                Ok(J::Array(rows.rows.into_iter().map(|cells| row_json(&ordered_row(table, cells))).collect()))
            })
            .await?;
        outcome
            .as_array()
            .ok_or_else(|| RuntimeError::Other(format!("the recorded outcome of `{}.query` is malformed", table.name)))?
            .iter()
            .map(json_row)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::DatabaseLocation;

    fn shelf() -> (Vec<String>, DatabaseLocation) {
        (vec!["id".into(), "title".into(), "count".into(), "revision".into()], DatabaseLocation::Path(":memory:".into()))
    }

    #[test]
    fn a_filter_binds_every_value_as_a_parameter() {
        let (columns, location) = shelf();
        let table = TableRef { name: "Shelf", key: "id", columns: &columns, location: &location };
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
            "SELECT \"id\", \"title\", \"count\", \"revision\" FROM \"Shelf\" WHERE (\"count\" > ?1 AND substr(\"title\", 1, length(?2)) = ?2) ORDER BY \"title\" DESC, \"id\" ASC LIMIT ?3"
        );
        assert_eq!(params, vec![DbValue::Integer(2), DbValue::Text("x'; DROP".into()), DbValue::Integer(5)]);
    }

    #[test]
    fn equality_uses_is_so_none_matches_missing_values() {
        let (columns, location) = shelf();
        let table = TableRef { name: "Shelf", key: "id", columns: &columns, location: &location };
        let query = TableQuery {
            filter: Some(TableFilter::Compare { column: "title".into(), op: QueryCompare::Eq, value: DbValue::Null }),
            ..TableQuery::default()
        };
        let (sql, _) = query_sql(&table, &query).unwrap();
        assert!(sql.ends_with("WHERE \"title\" IS ?1 ORDER BY \"id\" ASC"), "{sql}");
    }

    #[test]
    fn a_negative_take_is_refused() {
        let (columns, location) = shelf();
        let table = TableRef { name: "Shelf", key: "id", columns: &columns, location: &location };
        let query = TableQuery { take: Some(-1), ..TableQuery::default() };
        let error = query_sql(&table, &query).unwrap_err().to_string();
        assert!(error.contains("take(-1)"), "{error}");
    }
}
