//! Recorded table operations (B09 P1b): each runs live and is recorded
//! as a tool call named `table:<Table>.<op>`, or is served from the
//! replay trace without opening the database. The trace holds rows
//! and outcomes as JSON; the helpers here encode and decode them.

use super::{Row, TableRef};
use crate::db::DbValue;
use crate::errors::RuntimeError;
use crate::Runtime;
use serde_json::Value as J;

/// The recorded and replayable operations (P1b). Each is recorded as a
/// tool call named `table:<Table>.<op>`; a replay serves the recorded
/// outcome and never opens the database.
impl Runtime {
    /// `Shelf.get(key)`.
    pub async fn table_get(&self, table: &TableRef<'_>, key: Vec<DbValue>) -> Result<Option<Row>, RuntimeError> {
        let outcome = self
            .recorded_table_call(&call_name(table, "get"), key.iter().map(value_json).collect(), || async {
                Ok(match self.table_get_live(table, key).await? {
                    Some(row) => row_json(&row),
                    None => J::Null,
                })
            })
            .await?;
        if outcome.is_null() { Ok(None) } else { json_row(&outcome).map(Some) }
    }

    /// `Shelf.insert(row)`: `Ok` with the stored row (revision 1), or
    /// `Err` with the row already stored under that key.
    pub async fn table_insert(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Row>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "insert"), args, || async {
                Ok(match self.table_insert_live(table, row).await? {
                    Ok(stored) => serde_json::json!({ "ok": row_json(&stored) }),
                    Err(existing) => serde_json::json!({ "err": row_json(&existing) }),
                })
            })
            .await?;
        match (outcome.get("ok"), outcome.get("err")) {
            (Some(stored), _) => Ok(Ok(json_row(stored)?)),
            (_, Some(existing)) => Ok(Err(json_row(existing)?)),
            _ => Err(malformed(table, "insert")),
        }
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or `Err` with the row stored now (`None` if it was deleted).
    pub async fn table_save(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "save"), args, || async {
                Ok(checked_write_json(self.table_save_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table, "save", &outcome)
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or `Err` with the
    /// row stored now (`None` if it is already gone).
    pub async fn table_delete(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, Option<Row>>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "delete"), args, || async {
                Ok(checked_write_json(self.table_delete_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table, "delete", &outcome)
    }
}

pub(super) fn call_name(table: &TableRef<'_>, op: &str) -> String {
    format!("table:{}.{op}", table.name)
}

pub(super) fn malformed(table: &TableRef<'_>, op: &str) -> RuntimeError {
    RuntimeError::Other(format!("the recorded outcome of `{}.{op}` is malformed", table.name))
}

fn checked_write_json(outcome: Result<Row, Option<Row>>) -> J {
    match outcome {
        Ok(row) => serde_json::json!({ "ok": row_json(&row) }),
        Err(current) => serde_json::json!({ "err": current.as_ref().map(|row| row_json(row)) }),
    }
}

fn checked_write_outcome(table: &TableRef<'_>, op: &str, outcome: &J) -> Result<Result<Row, Option<Row>>, RuntimeError> {
    match (outcome.get("ok"), outcome.get("err")) {
        (Some(row), _) => Ok(Ok(json_row(row)?)),
        (_, Some(J::Null)) => Ok(Err(None)),
        (_, Some(current)) => Ok(Err(Some(json_row(current)?))),
        _ => Err(malformed(table, op)),
    }
}

/// A column value as JSON for the trace. An integer stays an integer
/// and a float keeps its decimal point, so the two read back apart.
pub(super) fn value_json(value: &DbValue) -> J {
    match value {
        DbValue::Null => J::Null,
        DbValue::Integer(n) => J::from(*n),
        DbValue::Float(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        DbValue::Text(text) => J::String(text.clone()),
        DbValue::Bool(b) => J::Bool(*b),
        DbValue::Bytes(bytes) => serde_json::json!({ RECORDED_BYTES: corvid_crypto::Bytes::new(bytes).to_base64_url() }),
    }
}

/// The key a recorded `Bytes` column value is held under.
const RECORDED_BYTES: &str = "bytes_base64url";

fn json_value(json: &J) -> Result<DbValue, RuntimeError> {
    Ok(match json {
        J::Null => DbValue::Null,
        J::Bool(b) => DbValue::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => DbValue::Integer(i),
            None => DbValue::Float(n.as_f64().unwrap_or_default()),
        },
        J::String(text) => DbValue::Text(text.clone()),
        J::Object(fields) if fields.len() == 1 && fields.contains_key(RECORDED_BYTES) => {
            let text = fields[RECORDED_BYTES].as_str().unwrap_or_default();
            let bytes = corvid_crypto::Bytes::from_base64_url(text)
                .map_err(|e| RuntimeError::Other(format!("a recorded `Bytes` column value is not base64url: {e}")))?;
            DbValue::Bytes(bytes.as_slice().to_vec())
        }
        other => return Err(RuntimeError::Other(format!("a recorded column value `{other}` is not a scalar"))),
    })
}

pub(super) fn row_json(row: &Row) -> J {
    J::Array(row.iter().map(value_json).collect())
}

pub(super) fn json_row(json: &J) -> Result<Row, RuntimeError> {
    json.as_array()
        .ok_or_else(|| RuntimeError::Other(format!("a recorded row `{json}` is not an array")))?
        .iter()
        .map(json_value)
        .collect()
}
