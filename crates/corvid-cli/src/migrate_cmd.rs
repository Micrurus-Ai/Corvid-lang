//! `corvid migrate` CLI dispatch — slice 27 / 38 migrations
//! surface, decomposed in Phase 20j-A1.
//!
//! Two entry points:
//!
//! - [`cmd_migrate`] handles the `up` / `inspect` subcommands —
//!   scans the migrations directory, classifies pending vs
//!   applied vs drift, and (for `up`) applies pending SQL
//!   transactions atomically while updating the recorded state.
//! - [`cmd_migrate_down`] handles the `down` subcommand —
//!   identifies the latest applied migration, locates its
//!   `<name>.down.sql` rollback file, and executes the
//!   rollback as a single transaction.
//!
//! All migration discovery / state I/O / SQL apply / SQL rollback
//! helpers are private to this module; main.rs's dispatch arm
//! only needs the two `cmd_migrate*` entry points.

use anyhow::{Context, Result};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(crate) fn cmd_migrate(
    action: &str,
    dir: &Path,
    state: &Path,
    database: &Path,
    dry_run: bool,
) -> Result<u8> {
    let migrations = scan_migration_files(dir)?;
    let writing = action == "up" && !dry_run;
    let mut connection = open_database(database, writing)?;
    let tx = connection
        .as_mut()
        .map(|conn| {
            conn.transaction_with_behavior(if writing {
                TransactionBehavior::Immediate
            } else {
                TransactionBehavior::Deferred
            })
        })
        .transpose()?;
    let (mut migration_state, _pending_export) = read_history(tx.as_deref(), state)?;

    let drift = detect_migration_drift(&migrations, &migration_state);
    println!("corvid migrate {action}");
    println!("migrations: {}", dir.display());
    println!("state: {}", state.display());
    println!("database: {}", database.display());
    println!("dry_run: {dry_run}");
    let applied_count = migrations
        .iter()
        .filter(|migration| {
            migration_state
                .migrations
                .iter()
                .any(|applied| applied.name == migration.name && applied.sha256 == migration.sha256)
        })
        .count();
    let pending_count = migrations.len().saturating_sub(applied_count);
    println!("applied_count: {applied_count}");
    println!("pending_count: {pending_count}");
    println!("drift_count: {}", drift.len());
    if migrations.is_empty() {
        println!("migrations_found: 0");
    } else {
        println!("migrations_found: {}", migrations.len());
        for migration in &migrations {
            let applied = migration_state.migrations.iter().any(|applied| {
                applied.name == migration.name && applied.sha256 == migration.sha256
            });
            println!(
                "migration: {} sha256:{} status:{}",
                migration.name,
                migration.sha256,
                if applied { "applied" } else { "pending" }
            );
        }
    }
    for item in &drift {
        println!("drift: {} {}", item.kind, item.message);
    }
    if !drift.is_empty() {
        println!("drift_found: {}", drift.len());
        println!("state_updated: false");
        return Ok(1);
    }
    if action == "up" && !dry_run {
        let tx = tx.expect("writing opens the database");
        apply_pending_sql_migrations(&tx, &migrations, &mut migration_state)?;
        commit_history(tx, &migration_state, "up")?;
        export_history(connection.as_mut().unwrap(), state)?;
        println!("state_updated: true");
    } else {
        println!("state_updated: false");
    }
    println!(
        "mutation_intent: {}",
        if action == "up" && !dry_run && pending_count > 0 {
            "apply_pending"
        } else if action == "down" && !dry_run {
            "rollback_latest"
        } else {
            "none"
        }
    );
    Ok(0)
}

pub(crate) fn cmd_migrate_down(
    dir: &Path,
    down_dir: &Path,
    state: &Path,
    database: &Path,
    dry_run: bool,
) -> Result<u8> {
    let migrations = scan_migration_files(dir)?;
    let writing = !dry_run;
    let mut connection = open_database(database, writing)?;
    let tx = connection
        .as_mut()
        .map(|conn| {
            conn.transaction_with_behavior(if writing {
                TransactionBehavior::Immediate
            } else {
                TransactionBehavior::Deferred
            })
        })
        .transpose()?;
    let (mut migration_state, pending_export) = read_history(tx.as_deref(), state)?;

    let drift = detect_migration_drift(&migrations, &migration_state);
    println!("corvid migrate down");
    println!("migrations: {}", dir.display());
    println!("down_migrations: {}", down_dir.display());
    println!("state: {}", state.display());
    println!("database: {}", database.display());
    println!("dry_run: {dry_run}");
    println!("applied_count: {}", migration_state.migrations.len());
    println!("drift_count: {}", drift.len());
    for item in &drift {
        println!("drift: {} {}", item.kind, item.message);
    }
    if !drift.is_empty() {
        println!("drift_found: {}", drift.len());
        println!("state_updated: false");
        return Ok(1);
    }
    if writing && pending_export.as_deref() == Some("down") {
        drop(tx);
        export_history(connection.as_mut().unwrap(), state)?;
        println!("state_updated: true");
        println!("mutation_intent: recovered_rollback");
        return Ok(0);
    }
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
        return Err(anyhow::anyhow!(
            "missing rollback SQL `{}` for `{}`",
            rollback.display(),
            latest.name
        ));
    }
    if dry_run {
        println!("state_updated: false");
        println!("mutation_intent: rollback_latest");
        return Ok(0);
    }
    let tx = tx.expect("writing opens the database");
    execute_rollback_sql(&tx, &rollback, &latest.name)?;
    migration_state.migrations.pop();
    commit_history(tx, &migration_state, "down")?;
    export_history(connection.as_mut().unwrap(), state)?;
    println!("state_updated: true");
    println!("mutation_intent: rollback_latest");
    Ok(0)
}

struct MigrationFile {
    name: String,
    sha256: String,
    path: PathBuf,
}

struct MigrationDrift {
    kind: &'static str,
    message: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct MigrationState {
    migrations: Vec<AppliedMigration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppliedMigration {
    name: String,
    sha256: String,
    applied_at: u64,
}

fn scan_migration_files(dir: &Path) -> Result<Vec<MigrationFile>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("cannot read migrations directory `{}`", dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("cannot read entry under `{}`", dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("cannot read migration `{}`", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<invalid>")
            .to_string();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        files.push(MigrationFile { name, sha256, path });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

fn detect_migration_drift(
    migrations: &[MigrationFile],
    state: &MigrationState,
) -> Vec<MigrationDrift> {
    let mut drift = Vec::new();
    let mut seen_versions = std::collections::HashMap::<String, String>::new();
    for migration in migrations {
        let version = migration_version(&migration.name);
        if let Some(previous) = seen_versions.insert(version.clone(), migration.name.clone()) {
            drift.push(MigrationDrift {
                kind: "duplicate",
                message: format!(
                    "version `{version}` appears in `{previous}` and `{}`",
                    migration.name
                ),
            });
        }
    }

    for applied in &state.migrations {
        match migrations
            .iter()
            .find(|migration| migration.name == applied.name)
        {
            Some(current) if current.sha256 != applied.sha256 => drift.push(MigrationDrift {
                kind: "changed",
                message: format!(
                    "`{}` expected sha256:{}, actual sha256:{}",
                    applied.name, applied.sha256, current.sha256
                ),
            }),
            Some(_) => {}
            None => drift.push(MigrationDrift {
                kind: "missing",
                message: format!("applied migration `{}` is missing from disk", applied.name),
            }),
        }
    }

    let file_order = migrations
        .iter()
        .map(|migration| migration.name.as_str())
        .collect::<Vec<_>>();
    let mut last_index = None;
    for applied in &state.migrations {
        if let Some(index) = file_order.iter().position(|name| *name == applied.name) {
            if last_index.is_some_and(|last| index < last) {
                drift.push(MigrationDrift {
                    kind: "out_of_order",
                    message: format!(
                        "applied migration `{}` is earlier than a previously applied migration",
                        applied.name
                    ),
                });
            }
            last_index = Some(index);
        }
    }

    drift
}

fn migration_version(name: &str) -> String {
    name.split_once('_')
        .map(|(version, _)| version)
        .or_else(|| name.split_once('.').map(|(version, _)| version))
        .unwrap_or(name)
        .to_string()
}

fn load_migration_state(path: &Path) -> Result<MigrationState> {
    if !path.exists() {
        return Ok(MigrationState::default());
    }
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read migration state `{}`", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("cannot parse migration state `{}`", path.display()))
}

fn save_migration_state(path: &Path, state: &MigrationState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create migration state dir `{}`", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(state).context("cannot serialize migration state")?;
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&json)?;
    temp.persist(path)
        .map(|_| ())
        .with_context(|| format!("cannot write migration state `{}`", path.display()))
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn open_database(database: &Path, writing: bool) -> Result<Option<Connection>> {
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

fn read_history(
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

fn commit_history(tx: Transaction<'_>, state: &MigrationState, action: &str) -> Result<()> {
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

fn export_history(conn: &mut Connection, path: &Path) -> Result<()> {
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

fn apply_pending_sql_migrations(
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

fn rollback_migration_path(down_dir: &Path, applied_name: &str) -> PathBuf {
    down_dir.join(format!("{applied_name}.down.sql"))
}

fn execute_rollback_sql(tx: &Transaction<'_>, rollback: &Path, applied_name: &str) -> Result<()> {
    let sql = std::fs::read_to_string(rollback)
        .with_context(|| format!("cannot read rollback SQL `{}`", rollback.display()))?;
    execute_migration_sql(tx, &sql)
        .with_context(|| format!("cannot execute rollback for `{applied_name}`"))?;
    Ok(())
}
