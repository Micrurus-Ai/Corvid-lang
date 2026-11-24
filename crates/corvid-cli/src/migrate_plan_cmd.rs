//! `corvid migrate plan <source>` (B09): write the migration that
//! creates the tables a program declares.
//!
//! The migration files are the schema's history, so the planner applies
//! every existing migration to an empty in-memory database and compares
//! the declared tables with what that leaves. A table no migration
//! creates gets a `CREATE TABLE` in a new migration file for review. A
//! table whose columns differ from the declaration is reported; planning
//! changes to existing tables is slice P3, so the planner refuses rather
//! than guess.

use crate::migrate_cmd::{migration_version, scan_migration_files};
use anyhow::{bail, Context, Result};
use corvid_runtime::tables::{create_table_sql, TableSchema};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::path::Path;

pub(crate) fn cmd_migrate_plan(source: &Path, dir: &Path, dry_run: bool) -> Result<u8> {
    let text = std::fs::read_to_string(source).with_context(|| format!("cannot read `{}`", source.display()))?;
    let schemas = match corvid_driver::table_schemas(&text) {
        Ok(schemas) => schemas,
        Err(diagnostics) => {
            eprint!("{}", corvid_driver::render_all_pretty(&diagnostics, source, &text));
            return Ok(1);
        }
    };
    if schemas.is_empty() {
        println!("plan: `{}` declares no tables", source.display());
        return Ok(0);
    }

    let history = Connection::open_in_memory().context("open an in-memory database")?;
    let migrations = scan_migration_files(dir)?;
    for migration in &migrations {
        let sql = std::fs::read_to_string(&migration.path)
            .with_context(|| format!("cannot read migration `{}`", migration.path.display()))?;
        history
            .execute_batch(&sql)
            .with_context(|| format!("migration `{}` does not apply in order to an empty database", migration.name))?;
    }

    let mut new_tables = Vec::new();
    let mut changed = Vec::new();
    for schema in &schemas {
        let existing = existing_columns(&history, &schema.name)?;
        if existing.is_empty() {
            new_tables.push(schema);
            continue;
        }
        let declared: BTreeSet<String> = schema.columns.iter().map(|(name, _)| name.clone()).collect();
        if declared != existing {
            let added: Vec<_> = declared.difference(&existing).cloned().collect();
            let removed: Vec<_> = existing.difference(&declared).cloned().collect();
            changed.push(format!(
                "table `{}`: declared but not migrated {:?}; migrated but not declared {:?}",
                schema.name, added, removed
            ));
        }
    }
    if !changed.is_empty() {
        for change in &changed {
            eprintln!("{change}");
        }
        bail!(
            "these tables differ from what the migrations create; planning changes to existing tables arrives in B09 P3, so write that migration by hand"
        );
    }
    if new_tables.is_empty() {
        println!("plan: the migrations already create every declared table");
        return Ok(0);
    }

    let names: Vec<&str> = new_tables.iter().map(|t| t.name.as_str()).collect();
    let mut sql = format!(
        "-- Planned by `corvid migrate plan` from {}. Review before `corvid migrate up`.\n\n",
        source.display()
    );
    for table in &new_tables {
        sql.push_str(&create_table_sql(table));
        sql.push('\n');
    }
    let file_name = format!("{}_create_{}.sql", next_version(&migrations), names.join("_").to_lowercase());
    if dry_run {
        println!("plan: would write {}\n\n{sql}", dir.join(&file_name).display());
        return Ok(0);
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create `{}`", dir.display()))?;
    let path = dir.join(&file_name);
    if path.exists() {
        bail!("`{}` already exists", path.display());
    }
    std::fs::write(&path, sql).with_context(|| format!("cannot write `{}`", path.display()))?;
    println!("plan: wrote {} creating {}", path.display(), names.join(", "));
    Ok(0)
}

fn existing_columns(history: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut statement = history.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let columns = statement.query_map([table], |row| row.get::<_, String>(0))?;
    Ok(columns.collect::<rusqlite::Result<_>>()?)
}

/// The next numeric version, padded like the existing ones.
fn next_version(migrations: &[crate::migrate_cmd::MigrationFile]) -> String {
    let numbers: Vec<(u64, usize)> = migrations
        .iter()
        .filter_map(|m| {
            let version = migration_version(&m.name);
            version.parse::<u64>().ok().map(|n| (n, version.len()))
        })
        .collect();
    let next = numbers.iter().map(|(n, _)| n + 1).max().unwrap_or(1);
    let width = numbers.iter().map(|(_, w)| *w).max().unwrap_or(3);
    format!("{next:0width$}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> TableSchema {
        use corvid_types::Type;
        TableSchema {
            name: "Shelf".into(),
            key: "shelf".into(),
            columns: vec![
                ("shelf".into(), Type::Int),
                ("title".into(), Type::String),
                ("note".into(), Type::Option(Box::new(Type::String))),
                ("tags".into(), Type::List(Box::new(Type::String))),
                ("revision".into(), Type::Int),
            ],
        }
    }

    #[test]
    fn the_create_statement_follows_the_column_mapping() {
        let sql = create_table_sql(&schema());
        assert_eq!(
            sql,
            "CREATE TABLE \"Shelf\" (\n    \"shelf\" INTEGER PRIMARY KEY NOT NULL,\n    \"title\" TEXT NOT NULL,\n    \"note\" TEXT,\n    \"tags\" TEXT NOT NULL,\n    \"revision\" INTEGER NOT NULL\n);\n"
        );
        let history = Connection::open_in_memory().unwrap();
        history.execute_batch(&sql).expect("the statement runs");
        assert_eq!(existing_columns(&history, "Shelf").unwrap().len(), 5);
    }
}
