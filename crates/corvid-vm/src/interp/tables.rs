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
use corvid_ir::{IrTable, IrType};
use corvid_runtime::db::DbValue;
use corvid_runtime::tables::{column_kind, untainted, ColumnKind, Row, TableRef};
use corvid_types::{TableOp, Type};
use std::sync::Arc;

impl<'ir> Interpreter<'ir> {
    pub(super) async fn eval_table_op(
        &mut self,
        op: TableOp,
        table: &IrTable,
        arg: Value,
        span: Span,
    ) -> Result<Value, InterpError> {
        let fail = |message: String| InterpError::new(InterpErrorKind::Other(message), span);
        let record = *self
            .types_by_id
            .get(&table.def_id)
            .ok_or_else(|| fail(format!("table `{}` has no record type", table.name)))?;
        let target = TableRef {
            name: &table.name,
            key: &table.key,
            columns: &table.columns,
            location: &table.database.location,
        };
        let runtime_error = |e| InterpError::new(InterpErrorKind::Runtime(e), span);
        Ok(match op {
            TableOp::Get => {
                let key_field = record
                    .fields
                    .iter()
                    .find(|f| f.name == table.key)
                    .ok_or_else(|| fail(format!("table `{}` has no key field `{}`", table.name, table.key)))?;
                let key = self.to_column(&arg, &key_field.ty).map_err(fail)?;
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
            (_, column) => {
                return Err(format!("the database holds {column:?} where the declaration says {}", ty.display_name()))
            }
        };
        Ok(if nullable { Value::OptionSome(BoxedValue::new(scalar)) } else { scalar })
    }
}
