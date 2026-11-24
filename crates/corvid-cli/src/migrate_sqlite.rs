//! The SQLite migration-history store behind `corvid migrate`: opening
//! the database, reading and committing the applied-migration history
//! kept in `_corvid_migration_history`, exporting it to the state file,
//! and running migration and rollback SQL inside the runner's
//! transaction.

use crate::migrate_cmd::{
    load_migration_state, now_unix_seconds, save_migration_state, AppliedMigration, MigrationFile, MigrationState,
};
use anyhow::{Context, Result};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::path::Path;

pub(crate) fn open_database(database: &Path, writing: bool) -> Result<Option<Connection>> {
    if !writing && !database.try_exists()? {
        return Ok(None);
    }
    let conn = if writing {
        if let Some(parent) = database.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        Connection::open(database)
    } else {
        Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
    };
    Ok(Some(conn.with_context(|| {
        format!("cannot open migration database `{}`", database.display())
    })?))
}

pub(crate) fn read_history(
    conn: Option<&Connection>,
    path: &Path,
) -> Result<(MigrationState, Option<String>)> {
    if let Some(conn) = conn {
        let initialized: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = '_corvid_migration_history')",
            [], |row| row.get(0),
        )?;
        if initialized {
            let (json, pending): (String, Option<String>) = conn.query_row(
                "SELECT history, pending_export FROM _corvid_migration_history WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            return Ok((serde_json::from_str(&json)?, pending));
        }
    }
    Ok((load_migration_state(path)?, None))
}

pub(crate) fn commit_history(tx: Transaction<'_>, state: &MigrationState, action: &str) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS _corvid_migration_history (
        id INTEGER PRIMARY KEY CHECK(id = 1), history TEXT NOT NULL, pending_export TEXT
    )",
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO _corvid_migration_history VALUES (1, ?1, ?2)",
        (serde_json::to_string(state)?, action),
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn export_history(conn: &mut Connection, path: &Path) -> Result<()> {
    // Serialize writers and export the newest committed snapshot.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (state, _) = read_history(Some(&tx), path)?;
    save_migration_state(path, &state)?;
    tx.execute(
        "UPDATE _corvid_migration_history SET pending_export = NULL WHERE id = 1",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn apply_pending_sql_migrations(
    tx: &Transaction<'_>,
    migrations: &[MigrationFile],
    state: &mut MigrationState,
) -> Result<()> {
    for migration in migrations {
        let applied = state
            .migrations
            .iter()
            .any(|applied| applied.name == migration.name && applied.sha256 == migration.sha256);
        if applied {
            continue;
        }
        let sql = std::fs::read_to_string(&migration.path)
            .with_context(|| format!("cannot read migration SQL `{}`", migration.path.display()))?;
        execute_migration_sql(tx, &sql)
            .with_context(|| format!("cannot execute migration `{}`", migration.name))?;
        state.migrations.push(AppliedMigration {
            name: migration.name.clone(),
            sha256: migration.sha256.clone(),
            applied_at: now_unix_seconds(),
        });
    }
    Ok(())
}

fn execute_migration_sql(tx: &Transaction<'_>, sql: &str) -> rusqlite::Result<()> {
    // Only the runner may end the transaction that owns the history update.
    tx.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Transaction { .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }));
    let result = tx.execute_batch(sql);
    tx.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    result
}

pub(crate) fn execute_rollback_sql(tx: &Transaction<'_>, rollback: &Path, applied_name: &str) -> Result<()> {
    let sql = std::fs::read_to_string(rollback)
        .with_context(|| format!("cannot read rollback SQL `{}`", rollback.display()))?;
    execute_migration_sql(tx, &sql)
        .with_context(|| format!("cannot execute rollback for `{applied_name}`"))?;
    Ok(())
}
