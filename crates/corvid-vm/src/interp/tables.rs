//! Table operations in the interpreter (B09).
//!
//! The runtime runs the SQL; this module converts interpreter values to
//! and from the column values it stores. Scalars map to SQLite columns
//! directly, `Option` maps to a nullable column, and every other field
//! type is stored as its public JSON text, so a record or list field
//! reads back exactly as the public mapping decodes it.

use super::Interpreter;
use crate::errors::{InterpError, InterpErrorKind};
use crate::public_json::{decode_public_json_text, encode_public_json_text, UnknownFields};
use crate::value::{BoxedValue, StructValue, Value};
use corvid_ast::Span;
use corvid_ir::{IrTable, IrTableFilter, IrType};
use corvid_runtime::db::DbValue;
use corvid_runtime::tables::{
    column_kind, parse_zoned_time_column_text, untainted, zoned_time_column_text, ColumnKind, Row, TableEngine, TableFilter,
    TableQuery, TableRef,
};
use corvid_types::{TableOp, Type};
use std::sync::Arc;

impl<'ir> Interpreter<'ir> {
    pub(super) async fn eval_table_op(
        &mut self,
        op: TableOp,
        table: &IrTable,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, InterpError> {
        let fail = |message: String| InterpError::new(InterpErrorKind::Other(message), span);
        // `get` takes the key's values; the other operations take one row.
        let arg = args.first().cloned().unwrap_or(Value::Nothing);
        let record = *self
            .types_by_id
            .get(&table.def_id)
            .ok_or_else(|| fail(format!("table `{}` has no record type", table.name)))?;
        let kinds = column_kinds(record);
        let target = table_target(table, &kinds, self.table_transaction.as_deref()).map_err(fail)?;
        let runtime_error = |e| InterpError::new(InterpErrorKind::Runtime(e), span);
        Ok(match op {
            TableOp::Get => {
                let key = table
                    .keys
                    .iter()
                    .zip(&args)
                    .map(|(key, value)| {
                        let field = record
                            .fields
                            .iter()
                            .find(|f| f.name == *key)
                            .ok_or_else(|| format!("table `{}` has no key field `{key}`", table.name))?;
                        self.to_column(value, &field.ty)
                    })
                    .collect::<Result<Vec<_>, String>>()
                    .map_err(fail)?;
                match self.runtime.table_get(&target, key).await.map_err(runtime_error)? {
                    Some(row) => Value::OptionSome(BoxedValue::new(self.row_value(record, row).map_err(fail)?)),
                    None => Value::OptionNone,
                }
            }
            TableOp::Insert => {
                let row = self.value_row(record, &arg).map_err(fail)?;
                match self.runtime.table_insert(&target, row).await.map_err(runtime_error)? {
                    Ok(stored) => Value::ResultOk(BoxedValue::new(self.row_value(record, stored).map_err(fail)?)),
                    Err(existing) => Value::ResultErr(BoxedValue::new(self.row_value(record, existing).map_err(fail)?)),
                }
            }
            TableOp::Save | TableOp::Delete => {
                let row = self.value_row(record, &arg).map_err(fail)?;
                let outcome = if op == TableOp::Save {
                    self.runtime.table_save(&target, row).await
                } else {
                    self.runtime.table_delete(&target, row).await
                };
                match outcome.map_err(runtime_error)? {
                    Ok(stored) => Value::ResultOk(BoxedValue::new(self.row_value(record, stored).map_err(fail)?)),
                    Err(Some(current)) => Value::ResultErr(BoxedValue::new(Value::OptionSome(BoxedValue::new(
                        self.row_value(record, current).map_err(fail)?,
                    )))),
                    Err(None) => Value::ResultErr(BoxedValue::new(Value::OptionNone)),
                }
            }
        })
    }

    /// A query chain (B09 P2). `operands` are the evaluated `where`
    /// values, then the `take` count if there is one, in the order
    /// `corvid_ir::table_query_operands` gives them.
    pub(super) async fn eval_table_query(
        &mut self,
        table: &IrTable,
        filter: Option<&IrTableFilter>,
        order: Option<&str>,
        descending: bool,
        operands: Vec<Value>,
        span: Span,
    ) -> Result<Value, InterpError> {
        let fail = |message: String| InterpError::new(InterpErrorKind::Other(message), span);
        let record = *self
            .types_by_id
            .get(&table.def_id)
            .ok_or_else(|| fail(format!("table `{}` has no record type", table.name)))?;
        let mut operands = operands.into_iter();
        let filter = match filter {
            Some(filter) => Some(self.table_filter(record, filter, &mut operands).map_err(fail)?),
            None => None,
        };
        let take = match operands.next() {
            Some(Value::Int(n)) => Some(n),
            Some(other) => return Err(fail(format!("`take` needs an Int, got {}", other.type_name()))),
            None => None,
        };
        let query = TableQuery { filter, order: order.map(str::to_string), descending, take };
        let kinds = column_kinds(record);
        let target = table_target(table, &kinds, self.table_transaction.as_deref()).map_err(fail)?;
        let rows = self
            .runtime
            .table_query(&target, &query)
            .await
            .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
        let items = rows.into_iter().map(|row| self.row_value(record, row)).collect::<Result<Vec<_>, _>>().map_err(fail)?;
        Ok(Value::List(crate::value::ListValue::new(items)))
    }

    /// A lowered condition with its values converted to column values.
    fn table_filter(
        &self,
        record: &IrType,
        filter: &IrTableFilter,
        operands: &mut std::vec::IntoIter<Value>,
    ) -> Result<TableFilter, String> {
        let column_type = |column: &str| {
            record
                .fields
                .iter()
                .find(|f| f.name == column)
                .map(|f| &f.ty)
                .ok_or_else(|| format!("table `{}` has no column `{column}`", record.name))
        };
        Ok(match filter {
            IrTableFilter::Const(b) => TableFilter::Const(*b),
            IrTableFilter::Column(column) => TableFilter::Column(column.clone()),
            IrTableFilter::And(left, right) => {
                let left = self.table_filter(record, left, operands)?;
                TableFilter::And(Box::new(left), Box::new(self.table_filter(record, right, operands)?))
            }
            IrTableFilter::Or(left, right) => {
                let left = self.table_filter(record, left, operands)?;
                TableFilter::Or(Box::new(left), Box::new(self.table_filter(record, right, operands)?))
            }
            IrTableFilter::Not(inner) => TableFilter::Not(Box::new(self.table_filter(record, inner, operands)?)),
            IrTableFilter::Compare { column, op, .. } => {
                let value = self.query_value(&next_operand(operands)?, column_type(column)?)?;
                TableFilter::Compare { column: column.clone(), op: *op, value }
            }
            IrTableFilter::Text { column, test, .. } => {
                let value = self.query_value(&next_operand(operands)?, column_type(column)?)?;
                TableFilter::Text { column: column.clone(), test: *test, value }
            }
        })
    }

    /// A value compared with a column, stored the way the column is. A
    /// `Float` compared with an `Int` column stays a float, so SQLite
    /// compares the two as numbers, as the agent would.
    fn query_value(&self, value: &Value, column_ty: &Type) -> Result<DbValue, String> {
        let (kind, _) = column_kind(untainted(column_ty));
        match (kind, value) {
            (ColumnKind::Integer, Value::Float(f)) => Ok(DbValue::Float(*f)),
            _ => self.to_column(value, column_ty),
        }
    }

    /// `atomic:` (B09 P7a). The body's table writes commit when it
    /// reaches its end. Leaving it through an error undoes them all:
    /// `?` propagating out of it (the checker refuses a plain `return`,
    /// `break` or `continue` there) or an interpreter error.
    pub(super) async fn eval_atomic(
        &mut self,
        database: Option<&corvid_ir::IrDatabase>,
        body: &'ir corvid_ir::IrBlock,
        span: Span,
    ) -> Result<super::Flow, InterpError> {
        let fail = |message: String| InterpError::new(InterpErrorKind::Other(message), span);
        let runtime_error = |e| InterpError::new(InterpErrorKind::Runtime(e), span);
        // A body that touches no table has nothing to commit.
        let Some(database) = database else { return self.eval_block(body).await };
        if self.table_transaction.is_some() {
            return Err(fail("an `atomic:` block runs inside another one".into()));
        }
        let engine = TableEngine::from_declaration(&database.engine, database.tls.as_deref(), database.connections)
            .ok_or_else(|| fail(format!("database `{}` names engine `{}`, which the runtime does not run", database.name, database.engine)))?;
        let transaction = self
            .runtime
            .table_begin(&database.name, &database.location, engine)
            .await
            .map_err(runtime_error)?;
        self.table_transaction = Some(Arc::new(transaction));
        let outcome = self.eval_block(body).await;
        let transaction = self
            .table_transaction
            .take()
            .and_then(|shared| Arc::try_unwrap(shared).ok())
            .ok_or_else(|| fail("an `atomic:` block's transaction is still in use when it ends".into()))?;
        match outcome {
            Ok(super::Flow::Normal) => {
                self.runtime.table_commit(transaction).await.map_err(runtime_error)?;
                Ok(super::Flow::Normal)
            }
            Ok(left_early) => {
                self.runtime.table_rollback(transaction).await.map_err(runtime_error)?;
                Ok(left_early)
            }
            Err(error) => {
                // The block's own error is the one to report; a failed
                // rollback still leaves the database unchanged, because the
                // transaction never committed.
                let _ = self.runtime.table_rollback(transaction).await;
                Err(error)
            }
        }
    }

    /// A row value's columns, in declaration order.
    fn value_row(&self, record: &IrType, value: &Value) -> Result<Row, String> {
        let Value::Struct(fields) = value else {
            return Err(format!("a `{}` row is {}", record.name, value.type_name()));
        };
        record
            .fields
            .iter()
            .map(|field| {
                let field_value = fields
                    .get_field(&field.name)
                    .ok_or_else(|| format!("a `{}` row has no `{}`", record.name, field.name))?;
                self.to_column(&field_value, &field.ty)
            })
            .collect()
    }

    /// A stored row as a record value.
    fn row_value(&self, record: &IrType, row: Row) -> Result<Value, String> {
        let fields = record
            .fields
            .iter()
            .zip(row)
            .map(|(field, column)| {
                self.from_column(column, &field.ty)
                    .map(|value| (field.name.clone(), value))
                    .map_err(|reason| format!("column `{}` of table `{}`: {reason}", field.name, record.name))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Struct(StructValue::new(record.id, record.name.clone(), fields)))
    }

    /// A field value as its column value, stored as `column_kind` says.
    fn to_column(&self, value: &Value, ty: &Type) -> Result<DbValue, String> {
        let ty = untainted(ty);
        let (kind, nullable) = column_kind(ty);
        if kind == ColumnKind::Json {
            return Ok(DbValue::Text(encode_public_json_text(value, ty, &self.types_by_id)?));
        }
        let (value, ty) = match (nullable, ty, value) {
            (true, _, Value::OptionNone | Value::Nothing) => return Ok(DbValue::Null),
            (true, Type::Option(inner), Value::OptionSome(payload)) => (payload.get(), untainted(inner)),
            (true, Type::Option(inner), other) => (other.clone(), untainted(inner)),
            (_, ty, other) => (other.clone(), ty),
        };
        Ok(match (kind, value) {
            (ColumnKind::Integer, Value::Int(n)) => DbValue::Integer(n),
            (ColumnKind::Real, Value::Float(f)) => DbValue::Float(f),
            (ColumnKind::Real, Value::Int(n)) => DbValue::Float(n as f64),
            (ColumnKind::Text, Value::String(s)) => DbValue::Text(s.to_string()),
            (ColumnKind::Boolean, Value::Bool(b)) => DbValue::Bool(b),
            // B09 P8a — each as the text its column compares exactly.
            (ColumnKind::Decimal, Value::Decimal(d)) => DbValue::Text(d.to_string()),
            (ColumnKind::Duration, Value::Duration(d)) => DbValue::Text(d.nanoseconds().to_string()),
            (ColumnKind::Instant, Value::Instant(i)) => DbValue::Text(i.to_sortable_text()),
            (ColumnKind::ZonedTime, Value::ZonedTime(z)) => DbValue::Text(zoned_time_column_text(&z)),
            (ColumnKind::TimeZone, Value::TimeZone(z)) => DbValue::Text(z.name().to_string()),
            (ColumnKind::Currency, Value::Currency(c)) => DbValue::Text(c.to_string()),
            (ColumnKind::Locale, Value::Locale(l)) => DbValue::Text(l.to_string()),
            // B09 P8c — the bytes themselves.
            (ColumnKind::Bytes, Value::Bytes(b)) => DbValue::Bytes(b.as_slice().to_vec()),
            (_, other) => return Err(format!("{} where the column holds {}", other.type_name(), ty.display_name())),
        })
    }

    /// A column value as the field value `column_kind` stored.
    fn from_column(&self, column: DbValue, ty: &Type) -> Result<Value, String> {
        let ty = untainted(ty);
        let (kind, nullable) = column_kind(ty);
        if nullable && column == DbValue::Null {
            return Ok(Value::OptionNone);
        }
        if kind == ColumnKind::Json {
            let DbValue::Text(json) = column else {
                return Err(format!("the database holds {column:?} where JSON text for {} belongs", ty.display_name()));
            };
            return decode_public_json_text(&json, ty, &self.types_by_id, UnknownFields::Ignore).map_err(|e| e.message());
        }
        let scalar = match (kind, column) {
            (ColumnKind::Integer, DbValue::Integer(n)) => Value::Int(n),
            (ColumnKind::Real, DbValue::Float(f)) => Value::Float(f),
            (ColumnKind::Real, DbValue::Integer(n)) => Value::Float(n as f64),
            (ColumnKind::Text, DbValue::Text(s)) => Value::String(Arc::from(s.as_str())),
            (ColumnKind::Boolean, DbValue::Bool(b)) => Value::Bool(b),
            // SQLite stores a Bool as an integer.
            (ColumnKind::Boolean, DbValue::Integer(n)) => Value::Bool(n != 0),
            (kind, DbValue::Text(text)) if kind.is_exact_text() => exact_value(kind, &text)?,
            (ColumnKind::Bytes, DbValue::Bytes(bytes)) => Value::Bytes(corvid_crypto::Bytes::from(bytes)),
            (_, column) => {
                return Err(format!("the database holds {column:?} where the declaration says {}", ty.display_name()))
            }
        };
        Ok(if nullable { Value::OptionSome(BoxedValue::new(scalar)) } else { scalar })
    }
}

/// A money or time value from the text its column stores (B09 P8a),
/// read with each type's own parser.
fn exact_value(kind: ColumnKind, text: &str) -> Result<Value, String> {
    let refused = |problem: String| format!("the database holds `{text}` where {} belongs: {problem}", kind.type_name());
    Ok(match kind {
        ColumnKind::Decimal => Value::Decimal(corvid_decimal::Decimal::parse(text).map_err(|e| refused(e.to_string()))?),
        ColumnKind::Duration => {
            let nanoseconds = text.parse::<i128>().map_err(|e| refused(e.to_string()))?;
            Value::Duration(corvid_time::Duration::from_nanoseconds(nanoseconds).map_err(|e| refused(e.to_string()))?)
        }
        ColumnKind::Instant => Value::Instant(corvid_time::Instant::parse(text).map_err(|e| refused(e.to_string()))?),
        ColumnKind::ZonedTime => Value::ZonedTime(parse_zoned_time_column_text(text).map_err(refused)?),
        ColumnKind::TimeZone => Value::TimeZone(corvid_time::TimeZone::parse(text).map_err(|e| refused(e.to_string()))?),
        ColumnKind::Currency => Value::Currency(corvid_currency::Currency::parse(text).map_err(|e| refused(e.to_string()))?),
        ColumnKind::Locale => Value::Locale(Arc::new(corvid_currency::Locale::parse(text).map_err(|e| refused(e.to_string()))?)),
        other => return Err(format!("{other:?} is not a money or time column")),
    })
}

fn next_operand(operands: &mut std::vec::IntoIter<Value>) -> Result<Value, String> {
    operands.next().ok_or_else(|| "a query condition is missing a value".to_string())
}

/// How each of a table's columns is stored, in declaration order.
fn column_kinds(record: &IrType) -> Vec<ColumnKind> {
    record.fields.iter().map(|field| column_kind(&field.ty).0).collect()
}

/// The table as the runtime sees it, on the engine its database names.
fn table_target<'t>(
    table: &'t IrTable,
    kinds: &'t [ColumnKind],
    transaction: Option<&'t corvid_runtime::tables::TableTransaction>,
) -> Result<TableRef<'t>, String> {
    let engine = TableEngine::from_declaration(&table.database.engine, table.database.tls.as_deref(), table.database.connections)
        .ok_or_else(|| format!("database `{}` names engine `{}`, which the runtime does not run", table.database.name, table.database.engine))?;
    Ok(TableRef {
        name: &table.name,
        keys: &table.keys,
        columns: &table.columns,
        kinds,
        unique: &table.unique,
        location: &table.database.location,
        engine,
        transaction,
    })
}
