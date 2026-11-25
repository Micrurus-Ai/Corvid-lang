//! Recorded table operations (B09 P1b): each runs live and is recorded
//! as a tool call named `table:<Table>.<op>`, or is served from the
//! replay trace without opening the database. The trace holds rows
//! and outcomes as JSON; the helpers here encode and decode them.

use super::{Row, TableRef, WriteConflict};
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
    /// `Taken` with the row that holds its key or a `unique` value.
    pub async fn table_insert(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "insert"), args, || async {
                Ok(checked_write_json(self.table_insert_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table.name, "insert", &outcome)
    }

    /// `Shelf.save(row)`: `Ok` with the stored row and its new revision,
    /// or the conflict that stopped it.
    pub async fn table_save(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "save"), args, || async {
                Ok(checked_write_json(self.table_save_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table.name, "save", &outcome)
    }

    /// `Shelf.delete(row)`: `Ok` with the deleted row, or the conflict
    /// that stopped it.
    pub async fn table_delete(&self, table: &TableRef<'_>, row: Row) -> Result<Result<Row, WriteConflict>, RuntimeError> {
        let args = vec![row_json(&row)];
        let outcome = self
            .recorded_table_call(&call_name(table, "delete"), args, || async {
                Ok(checked_write_json(self.table_delete_live(table, row).await?))
            })
            .await?;
        checked_write_outcome(table.name, "delete", &outcome)
    }
}

pub(super) fn call_name(table: &TableRef<'_>, op: &str) -> String {
    format!("table:{}.{op}", table.name)
}

pub(super) fn malformed(table: &TableRef<'_>, op: &str) -> RuntimeError {
    malformed_outcome(table.name, op)
}

fn malformed_outcome(table: &str, op: &str) -> RuntimeError {
    RuntimeError::Other(format!("the recorded outcome of `{table}.{op}` is malformed"))
}

/// A write's outcome in the trace: `{"ok": row}`, or `{"conflict":
/// "Changed" | "Deleted" | "Taken", "row": row}` naming the case, with
/// `row` absent for `Deleted`.
fn checked_write_json(outcome: Result<Row, WriteConflict>) -> J {
    match outcome {
        Ok(row) => serde_json::json!({ "ok": row_json(&row) }),
        Err(conflict) => {
            let mut recorded = serde_json::json!({ "conflict": conflict.case().name() });
            if let Some(row) = conflict.row() {
                recorded["row"] = row_json(row);
            }
            recorded
        }
    }
}

fn checked_write_outcome(table: &str, op: &str, outcome: &J) -> Result<Result<Row, WriteConflict>, RuntimeError> {
    if let Some(row) = outcome.get("ok") {
        return Ok(Ok(json_row(row)?));
    }
    if outcome.get("err").is_some() {
        // Recorded before writes named their conflict; which case it was
        // cannot be told from it, so it is not guessed.
        return Err(RuntimeError::Other(format!(
            "the recorded outcome of `{table}.{op}` predates named conflicts; record the trace again"
        )));
    }
    let case = outcome
        .get("conflict")
        .and_then(J::as_str)
        .and_then(corvid_types::ConflictCase::from_name)
        .ok_or_else(|| malformed_outcome(table, op))?;
    let row = outcome.get("row").map(json_row).transpose()?;
    WriteConflict::new(case, row).map(Err).ok_or_else(|| malformed_outcome(table, op))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each case is recorded by name and read back as itself.
    #[test]
    fn every_write_outcome_round_trips_through_the_trace() {
        let row = vec![DbValue::Integer(1), DbValue::Text("Novels".into()), DbValue::Integer(2)];
        for outcome in [
            Ok(row.clone()),
            Err(WriteConflict::Changed(row.clone())),
            Err(WriteConflict::Deleted),
            Err(WriteConflict::Taken(row.clone())),
        ] {
            let recorded = checked_write_json(outcome.clone());
            let read = checked_write_outcome("Shelf", "save", &recorded).expect("reads back");
            assert_eq!(read, outcome, "{recorded}");
        }
        assert_eq!(checked_write_json(Err(WriteConflict::Deleted)), serde_json::json!({ "conflict": "Deleted" }));
    }

    /// An outcome recorded before conflicts were named could be either
    /// case, so a replay refuses it instead of guessing.
    #[test]
    fn an_outcome_recorded_before_named_conflicts_is_refused() {
        let error = checked_write_outcome("Shelf", "save", &serde_json::json!({ "err": [1, "Novels", 2] })).unwrap_err();
        assert!(error.to_string().contains("predates named conflicts; record the trace again"), "{error}");
        let wrong = checked_write_outcome("Shelf", "save", &serde_json::json!({ "conflict": "Deleted", "row": [1] }));
        assert!(wrong.is_err(), "a Deleted conflict carries no row");
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
