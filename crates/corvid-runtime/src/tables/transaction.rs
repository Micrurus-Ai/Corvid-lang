//! `atomic:` blocks (B09 P7a): table writes that commit together or not
//! at all.
//!
//! A transaction runs on its own connection, so no other agent's
//! statements land inside it. On SQLite it also holds the file's writer
//! turn from `BEGIN IMMEDIATE` to its end: other writers wait in the
//! registry's queue instead of on SQLite's lock, where they could
//! deadlock against the open transaction. On Postgres the connection is
//! opened for the block and closed after it.
//!
//! The begin, commit and rollback are recorded in the trace as table
//! calls named `table:<database>.begin`, `.commit` and `.rollback`, so a
//! replay shows where a transaction started and how it ended; a replay
//! serves them from the trace and never opens the database.

use super::{named_database, resolve_location, TableConnection, TableEngine};
use crate::db::DbValue;
use crate::errors::RuntimeError;
use crate::Runtime;
use corvid_ast::DatabaseLocation;
use serde_json::Value as J;
use std::sync::Arc;

/// An open `atomic:` block's transaction.
pub struct TableTransaction {
    /// The database's connection identity, as table operations name it.
    pub(super) id: String,
    database: String,
    /// `None` during a replay, which never opens the database.
    pub(super) connection: Option<TableConnection>,
    _writer_turn: Option<tokio::sync::OwnedMutexGuard<()>>,
    /// The tables written inside the block, announced at commit (B14).
    pub(super) written: super::live::WrittenTables,
}

impl std::fmt::Debug for TableTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableTransaction").field("database", &self.database).finish_non_exhaustive()
    }
}

impl Runtime {
    /// Begin the transaction for an `atomic:` block on `database`.
    pub async fn table_begin(
        &self,
        database: &str,
        location: &DatabaseLocation,
        engine: TableEngine,
    ) -> Result<TableTransaction, RuntimeError> {
        let resolved = resolve_location(location)?;
        let id = format!("{}:{resolved}", engine.name());
        let mut opened = None;
        self.recorded_table_call(&format!("table:{database}.begin"), Vec::new(), || async {
            opened = Some(self.open_transaction(&resolved, engine).await?);
            Ok(J::Null)
        })
        .await?;
        let (connection, writer_turn) = match opened {
            Some((connection, turn)) => (Some(connection), turn),
            None => (None, None),
        };
        Ok(TableTransaction {
            id,
            database: database.to_string(),
            connection,
            _writer_turn: writer_turn,
            written: Default::default(),
        })
    }

    /// Commit an `atomic:` block's writes.
    pub async fn table_commit(&self, transaction: TableTransaction) -> Result<(), RuntimeError> {
        let database = transaction.database.clone();
        let written = transaction.written.take();
        // Other processes on Postgres hear of the block through a
        // notification sent inside it, which Postgres delivers only if
        // the block commits (RT-6).
        let notify = (!written.tables.is_empty())
            .then(|| super::shared_commits::notify_statement(&self.instance_id, &database, &written.tables, &written.rows));
        self.end_transaction(transaction, "commit", "COMMIT", notify).await?;
        // Only now are the block's writes visible, so only now announced.
        self.announce_commit(&database, written);
        Ok(())
    }

    /// Undo an `atomic:` block's writes.
    pub async fn table_rollback(&self, transaction: TableTransaction) -> Result<(), RuntimeError> {
        self.end_transaction(transaction, "rollback", "ROLLBACK", None).await
    }

    async fn open_transaction(
        &self,
        location: &str,
        engine: TableEngine,
    ) -> Result<(TableConnection, Option<tokio::sync::OwnedMutexGuard<()>>), RuntimeError> {
        match engine {
            TableEngine::Sqlite => {
                // A connection of its own, opened before taking the turn.
                let handle = self.db_registry().open(location)?;
                let turn = handle.hold_writer_turn().await;
                self.db_registry().execute(handle.handle_id, "BEGIN IMMEDIATE", &[])?;
                Ok((TableConnection::Sqlite(handle), turn))
            }
            TableEngine::Postgres { .. } => {
                // A connection from the database's pool, held until the
                // block ends; a block waits its turn when all are in use.
                let TableConnection::Postgres(pool) = self.shared_connection(location, engine)? else {
                    return Err(RuntimeError::Other("a Postgres database has no connection pool".into()));
                };
                let mut borrowed = pool.borrow().await?;
                if let Err(error) = borrowed.connection().execute("BEGIN", &[]).await {
                    borrowed.discard();
                    return Err(error);
                }
                Ok((TableConnection::PostgresHeld(Arc::new(tokio::sync::Mutex::new(borrowed))), None))
            }
        }
    }

    async fn end_transaction(
        &self,
        transaction: TableTransaction,
        op: &str,
        sql: &str,
        notify: Option<(&'static str, [DbValue; 5])>,
    ) -> Result<(), RuntimeError> {
        let TableTransaction { database, connection, _writer_turn, .. } = transaction;
        self.recorded_table_call(&format!("table:{database}.{op}"), Vec::new(), || async {
            match &connection {
                Some(TableConnection::Sqlite(handle)) => {
                    self.db_registry().execute(handle.handle_id, sql, &[]).map_err(|e| named_database(&database, e))?;
                }
                Some(TableConnection::PostgresHeld(held)) => {
                    let mut borrowed = held.lock().await;
                    if let Some((notify, params)) = &notify {
                        if let Err(error) = borrowed.connection().execute(notify, params).await {
                            // Nothing is committed: the block fails whole.
                            let _ = borrowed.connection().execute("ROLLBACK", &[]).await;
                            borrowed.discard();
                            return Err(named_database(&database, error));
                        }
                    }
                    if let Err(error) = borrowed.connection().execute(sql, &[]).await {
                        // The transaction may still be open on it.
                        borrowed.discard();
                        return Err(named_database(&database, error));
                    }
                }
                Some(TableConnection::Postgres(_)) => {
                    return Err(RuntimeError::Other(format!("the `atomic:` block on `{database}` holds no connection")));
                }
                None => {}
            }
            Ok(J::Null)
        })
        .await?;
        // The writer turn is released only now, after the commit or
        // rollback has finished.
        drop(_writer_turn);
        Ok(())
    }
}
