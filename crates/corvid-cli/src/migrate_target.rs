//! Which database a `corvid migrate` command runs against (B09 P4b).
//!
//! `--database <path>` names a SQLite file directly. `--program <source>`
//! reads the database from the program's own `database` block, so the
//! engine, the connection string (from the environment the block names)
//! and the TLS policy are the ones the program declares, never retyped on
//! the command line.

use crate::{migrate_cmd, migrate_postgres};
use anyhow::{bail, Context, Result};
use corvid_runtime::tables::{resolve_location, TableEngine};
use std::path::{Path, PathBuf};

/// The `status` / `up` / `down` action and its paths.
pub(crate) struct MigrateRequest<'a> {
    pub action: &'a str,
    pub dir: &'a Path,
    pub down_dir: Option<&'a Path>,
    pub state: &'a Path,
    pub dry_run: bool,
}

pub(crate) fn run(request: MigrateRequest<'_>, database: &Path, program: Option<&Path>) -> Result<u8> {
    let Some(program) = program else {
        return run_sqlite(&request, database);
    };
    let source = std::fs::read_to_string(program).with_context(|| format!("cannot read `{}`", program.display()))?;
    let databases = match corvid_driver::declared_databases(&source, program) {
        Ok(databases) => databases,
        Err(diagnostics) => {
            eprint!("{}", corvid_driver::render_all_pretty(&diagnostics, program, &source));
            return Ok(1);
        }
    };
    match databases.as_slice() {
        [] => bail!("`{}` declares no `database` block", program.display()),
        [one] => run_declared(&request, one),
        several => {
            // One history per database, each in `<dir>/<database>/`, with
            // its own state file beside the given one.
            let names: Vec<&str> = several.iter().map(|db| db.name.as_str()).collect();
            refuse_a_shared_history(request.dir, &names)?;
            let mut status = 0;
            for declared in several {
                println!("database `{}`", declared.name);
                let dir = request.dir.join(&declared.name);
                let down_dir = request.down_dir.map(|d| d.join(&declared.name));
                let state = per_database_state(request.state, &declared.name);
                let one = MigrateRequest { action: request.action, dir: &dir, down_dir: down_dir.as_deref(), state: &state, dry_run: request.dry_run };
                status = status.max(run_declared(&one, declared)?);
            }
            Ok(status)
        }
    }
}

/// A program with several databases keeps one history per database, in
/// `<dir>/<database>/`. Migration files directly in `<dir>` belonged to
/// the single database the program had before; running them against any
/// one of several would be a guess, so they must be moved first.
pub(crate) fn refuse_a_shared_history(dir: &Path, databases: &[&str]) -> Result<()> {
    let flat = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|x| x == "sql")))
        .unwrap_or(false);
    if flat {
        bail!(
            "this program uses {} databases ({}), so each keeps its migrations in `{}/<database>/`, but `{}` holds migrations directly; move them into the folder of the database they belong to",
            databases.len(),
            databases.iter().map(|d| format!("`{d}`")).collect::<Vec<_>>().join(", "),
            dir.display(),
            dir.display(),
        );
    }
    Ok(())
}

/// `target/state.json` becomes `target/state.<database>.json`.
fn per_database_state(state: &Path, database: &str) -> PathBuf {
    let stem = state.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "state".into());
    let extension = state.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    state.with_file_name(format!("{stem}.{database}{extension}"))
}

/// Run the request against one database the program declares.
fn run_declared(request: &MigrateRequest<'_>, declared: &corvid_driver::DeclaredDatabase) -> Result<u8> {
    let location = resolve_location(&declared.location).map_err(|e| anyhow::anyhow!("{e}"))?;
    match declared.engine {
        TableEngine::Sqlite => run_sqlite(request, &PathBuf::from(location)),
        TableEngine::Postgres { .. } => {
            let label = match &declared.location {
                corvid_ast::DatabaseLocation::Env(name) => format!("postgres database `{}` from `{name}`", declared.name),
                corvid_ast::DatabaseLocation::Path(_) => format!("postgres database `{}`", declared.name),
            };
            let mut client = migrate_postgres::connect(&location, declared.engine)?;
            match request.down_dir {
                Some(down_dir) => migrate_postgres::cmd_migrate_down(
                    request.dir,
                    down_dir,
                    request.state,
                    &label,
                    &mut client,
                    request.dry_run,
                ),
                None => migrate_postgres::cmd_migrate(request.action, request.dir, request.state, &label, &mut client, request.dry_run),
            }
        }
    }
}

fn run_sqlite(request: &MigrateRequest<'_>, database: &Path) -> Result<u8> {
    match request.down_dir {
        Some(down_dir) => migrate_cmd::cmd_migrate_down(request.dir, down_dir, request.state, database, request.dry_run),
        None => migrate_cmd::cmd_migrate(request.action, request.dir, request.state, database, request.dry_run),
    }
}
