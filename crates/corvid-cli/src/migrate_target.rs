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
    let databases = match corvid_driver::declared_databases(&source) {
        Ok(databases) => databases,
        Err(diagnostics) => {
            eprint!("{}", corvid_driver::render_all_pretty(&diagnostics, program, &source));
            return Ok(1);
        }
    };
    let declared = match databases.as_slice() {
        [one] => one,
        [] => bail!("`{}` declares no `database` block", program.display()),
        _ => bail!(
            "`{}` declares more than one database; migrations per database are planned work, so migrate each with its own program for now",
            program.display()
        ),
    };
    let location = resolve_location(&declared.location).map_err(|e| anyhow::anyhow!("{e}"))?;
    match declared.engine {
        TableEngine::Sqlite => run_sqlite(&request, &PathBuf::from(location)),
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
