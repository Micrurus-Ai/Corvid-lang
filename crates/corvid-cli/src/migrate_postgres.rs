//! `corvid migrate` on Postgres (B09 P4b): the applied-migration history,
//! applying and rolling back migrations, and reading a table's stored
//! columns for the planner.
//!
//! The history lives in the database, in `_corvid_migration_history`, as
//! on SQLite, and every command runs in one transaction under an
//! advisory lock, so two runners never apply the same migration and a
//! failed migration changes nothing. A migration may not end that
//! transaction itself: one that contains `BEGIN`, `COMMIT`, `ROLLBACK`
//! or another transaction-control statement is refused before it runs.
//!
//! The connection string comes from the program's `database` block,
//! never the command line, and a `tls: required` database is verified
//! with the same connector the runtime uses.

use crate::migrate_cmd::{
    load_migration_state, now_unix_seconds, print_mutation_intent, report_migrations, rollback_migration_path,
    save_migration_state, scan_migration_files, AppliedMigration, MigrationFile, MigrationState,
};
use anyhow::{bail, Context, Result};
use corvid_runtime::tables::{StoredColumn, TableEngine, VerifiedTls};
use postgres::{Client, Transaction};
use std::path::Path;

/// Serializes `corvid migrate` runners on one database.
const MIGRATION_LOCK: i64 = 0x636f_7276_6964; // "corvid"

/// Connect to a Postgres database with the declared TLS policy.
pub(crate) fn connect(url: &str, engine: TableEngine) -> Result<Client> {
    let TableEngine::Postgres { tls, .. } = engine else { bail!("not a Postgres database") };
    let client = if tls {
        Client::connect(url, VerifiedTls::new().map_err(|e| anyhow::anyhow!("{e}"))?)
    } else {
        Client::connect(url, postgres::NoTls)
    };
    client.map_err(|e| {
        let hint = if tls { "; the `database` block says `tls: required`, so the server's certificate must verify" } else { "" };
        anyhow::anyhow!("cannot connect to the Postgres database: {e}{hint}")
    })
}

/// `corvid migrate status` and `up` against a Postgres database.
pub(crate) fn cmd_migrate(action: &str, dir: &Path, state: &Path, label: &str, client: &mut Client, dry_run: bool) -> Result<u8> {
    let migrations = scan_migration_files(dir)?;
    let mut tx = client.transaction()?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])?;
    let (mut migration_state, _) = read_history(&mut tx, state)?;
    let Some(pending_count) = report_migrations(action, dir, state, label, dry_run, &migrations, &migration_state) else {
        return Ok(1);
    };
    if action == "up" && !dry_run {
        apply_pending(&mut tx, &migrations, &mut migration_state)?;
        commit_history(tx, &migration_state, state)?;
        println!("state_updated: true");
    } else {
        println!("state_updated: false");
    }
    print_mutation_intent(action, dry_run, pending_count);
    Ok(0)
}

/// `corvid migrate down` against a Postgres database.
pub(crate) fn cmd_migrate_down(
    dir: &Path,
    down_dir: &Path,
    state: &Path,
    label: &str,
    client: &mut Client,
    dry_run: bool,
) -> Result<u8> {
    let migrations = scan_migration_files(dir)?;
    let mut tx = client.transaction()?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])?;
    let (mut migration_state, _) = read_history(&mut tx, state)?;
    if report_migrations("down", dir, state, label, dry_run, &migrations, &migration_state).is_none() {
        return Ok(1);
    }
    println!("down_migrations: {}", down_dir.display());
    let Some(latest) = migration_state.migrations.last().cloned() else {
        println!("rollback: none");
        println!("state_updated: false");
        println!("mutation_intent: none");
        return Ok(0);
    };
    let rollback = rollback_migration_path(down_dir, &latest.name);
    println!("rollback: {}", latest.name);
    println!("rollback_sql: {}", rollback.display());
    if !rollback.exists() {
        println!("state_updated: false");
        bail!("missing rollback SQL `{}` for `{}`", rollback.display(), latest.name);
    }
    if dry_run {
        println!("state_updated: false");
        println!("mutation_intent: rollback_latest");
        return Ok(0);
    }
    let sql = std::fs::read_to_string(&rollback).with_context(|| format!("cannot read rollback SQL `{}`", rollback.display()))?;
    run_migration_sql(&mut tx, &sql).with_context(|| format!("cannot execute rollback for `{}`", latest.name))?;
    migration_state.migrations.pop();
    commit_history(tx, &migration_state, state)?;
    println!("state_updated: true");
    println!("mutation_intent: rollback_latest");
    Ok(0)
}

fn read_history(tx: &mut Transaction<'_>, state: &Path) -> Result<(MigrationState, Option<String>)> {
    let initialized: bool = tx
        .query_one("SELECT to_regclass('_corvid_migration_history') IS NOT NULL", &[])?
        .get(0);
    if initialized {
        let row = tx.query_one("SELECT history, pending_export FROM _corvid_migration_history WHERE id = 1", &[])?;
        let history: String = row.get(0);
        return Ok((serde_json::from_str(&history)?, row.get(1)));
    }
    Ok((load_migration_state(state)?, None))
}

/// Record the history in the database, commit, then mirror it to the
/// state file. The file is a copy; the database is the record.
fn commit_history(mut tx: Transaction<'_>, migration_state: &MigrationState, state: &Path) -> Result<()> {
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS _corvid_migration_history (
            id integer PRIMARY KEY CHECK (id = 1), history text NOT NULL, pending_export text
        )",
    )?;
    tx.execute(
        "INSERT INTO _corvid_migration_history VALUES (1, $1, NULL)
         ON CONFLICT (id) DO UPDATE SET history = EXCLUDED.history, pending_export = NULL",
        &[&serde_json::to_string(migration_state)?],
    )?;
    tx.commit()?;
    save_migration_state(state, migration_state)
}

fn apply_pending(tx: &mut Transaction<'_>, migrations: &[MigrationFile], state: &mut MigrationState) -> Result<()> {
    for migration in migrations {
        let applied = state.migrations.iter().any(|a| a.name == migration.name && a.sha256 == migration.sha256);
        if applied {
            continue;
        }
        let sql = std::fs::read_to_string(&migration.path)
            .with_context(|| format!("cannot read migration SQL `{}`", migration.path.display()))?;
        run_migration_sql(tx, &sql).with_context(|| format!("cannot execute migration `{}`", migration.name))?;
        state.migrations.push(AppliedMigration {
            name: migration.name.clone(),
            sha256: migration.sha256.clone(),
            applied_at: now_unix_seconds(),
        });
    }
    Ok(())
}

/// Run migration SQL inside the runner's transaction, refusing any
/// statement that would end it.
pub(crate) fn run_migration_sql(tx: &mut Transaction<'_>, sql: &str) -> Result<()> {
    if let Some(statement) = transaction_control(sql) {
        bail!(
            "the migration contains `{statement}`; `corvid migrate` runs each migration in its own transaction, so a migration may not begin or end one"
        );
    }
    tx.batch_execute(sql).map_err(|e| {
        let detail = e.as_db_error().map(|db| db.message().to_string()).unwrap_or_else(|| e.to_string());
        anyhow::anyhow!("{detail}")
    })
}

/// A table's columns as stored in the current schema; empty if the table
/// does not exist.
pub(crate) fn stored_columns(tx: &mut Transaction<'_>, table: &str) -> Result<Vec<StoredColumn>> {
    let rows = tx.query(
        "SELECT c.column_name::text, coalesce(c.domain_name, c.data_type)::text, c.is_nullable = 'NO',
                EXISTS (
                    SELECT 1 FROM information_schema.table_constraints tc
                    JOIN information_schema.key_column_usage k
                      ON k.constraint_name = tc.constraint_name AND k.table_schema = tc.table_schema
                    WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema
                      AND tc.table_name = c.table_name AND k.column_name = c.column_name
                )
         FROM information_schema.columns c
         WHERE c.table_schema = current_schema() AND c.table_name = $1
         ORDER BY c.ordinal_position",
        &[&table],
    )?;
    Ok(rows
        .iter()
        .map(|row| StoredColumn { name: row.get(0), sql_type: row.get(1), not_null: row.get(2), primary_key: row.get(3) })
        .collect())
}

/// The names of a table's indexes in the current schema.
pub(crate) fn stored_indexes(tx: &mut Transaction<'_>, table: &str) -> Result<Vec<String>> {
    let rows = tx.query(
        "SELECT indexname::text FROM pg_indexes WHERE schemaname = current_schema() AND tablename = $1 ORDER BY indexname",
        &[&table],
    )?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// The first transaction-control statement in `sql`, if any. Comments,
/// quoted strings, quoted identifiers and dollar-quoted bodies are
/// skipped, and a `BEGIN ATOMIC ... END` function body is one statement.
pub(crate) fn transaction_control(sql: &str) -> Option<String> {
    const CONTROL: &[&str] = &["BEGIN", "START", "COMMIT", "END", "ROLLBACK", "ABORT", "PREPARE", "SAVEPOINT", "RELEASE"];
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut first_word: Option<String> = None;
    let mut previous_word = String::new();
    let mut atomic_depth = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < bytes.len() {
                if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if c == b'\'' || c == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == c && bytes.get(i + 1) == Some(&c) {
                    i += 2;
                } else if bytes[i] == c {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else if c == b'$' {
            // `$tag$ ... $tag$`; a bare `$1` parameter is not a quote.
            let tag_end = sql[i + 1..].find('$').map(|n| i + 1 + n);
            match tag_end {
                Some(end) if sql[i + 1..end].chars().all(|ch| ch.is_alphanumeric() || ch == '_')
                    && !sql[i + 1..end].starts_with(|ch: char| ch.is_ascii_digit()) =>
                {
                    let tag = &sql[i..=end];
                    i = sql[end + 1..].find(tag).map(|n| end + 1 + n + tag.len()).unwrap_or(bytes.len());
                }
                _ => i += 1,
            }
        } else if c == b';' {
            if atomic_depth == 0 {
                first_word = None;
            }
            i += 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = sql[start..i].to_ascii_uppercase();
            if word == "ATOMIC" && previous_word == "BEGIN" && first_word.as_deref() != Some("BEGIN") {
                atomic_depth += 1;
            } else if word == "END" && atomic_depth > 0 {
                atomic_depth -= 1;
            } else if first_word.is_none() {
                if CONTROL.contains(&word.as_str()) {
                    return Some(word);
                }
                first_word = Some(word.clone());
            }
            previous_word = word;
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::transaction_control;

    #[test]
    fn transaction_control_is_found_outside_quotes_and_comments() {
        assert_eq!(transaction_control("CREATE TABLE t (a int);\nCOMMIT;"), Some("COMMIT".into()));
        assert_eq!(transaction_control("begin; create table t (a int);"), Some("BEGIN".into()));
        assert_eq!(transaction_control("INSERT INTO t VALUES ('COMMIT'); -- COMMIT\n/* ROLLBACK */"), None);
        assert_eq!(transaction_control("CREATE FUNCTION f() RETURNS int AS $body$ BEGIN; COMMIT; $body$ LANGUAGE sql;"), None);
        assert_eq!(
            transaction_control("CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END; CREATE TABLE t (a int);"),
            None
        );
        assert_eq!(transaction_control("UPDATE t SET a = $1;"), None);
    }
}
