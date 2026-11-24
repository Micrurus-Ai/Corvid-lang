//! `corvid migrate plan <source>` (B09): write the migration that
//! brings the stored tables to the program's declarations.
//!
//! The migration files are the schema's history, so the planner applies
//! every existing migration to an empty in-memory database and compares
//! the declared tables with what that leaves. A table no migration
//! creates gets a `CREATE TABLE`; a stored table that differs gets the
//! change [`plan_table_change`] plans, or its refusals. The new file is
//! checked by running it on the history before it is written.

use crate::migrate_cmd::{migration_version, scan_migration_files};
use anyhow::{bail, Context, Result};
use corvid_runtime::tables::{create_table_sql, plan_table_change, StoredColumn};
use rusqlite::Connection;
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

    let mut created = Vec::new();
    let mut changed = Vec::new();
    let mut refusals = Vec::new();
    for schema in &schemas {
        let stored = stored_columns(&history, &schema.name)?;
        if stored.is_empty() {
            created.push(schema);
            continue;
        }
        match plan_table_change(schema, &stored) {
            Ok(None) => {}
            Ok(Some(change)) => changed.push((schema, change)),
            Err(reasons) => refusals.extend(reasons),
        }
    }
    if !refusals.is_empty() {
        for refusal in &refusals {
            eprintln!("refused: {refusal}");
        }
        eprintln!("plan: no migration written; change the declarations as each refusal says, then plan again");
        return Ok(1);
    }
    if created.is_empty() && changed.is_empty() {
        println!("plan: the migrations already match every declared table");
        return Ok(0);
    }

    let mut sql = format!(
        "-- Planned by `corvid migrate plan` from {}. Review before `corvid migrate up`.\n\n",
        source.display()
    );
    for table in &created {
        sql.push_str(&format!("-- Create `{}`.\n", table.name));
        sql.push_str(&create_table_sql(table));
        sql.push('\n');
    }
    for (table, change) in &changed {
        for line in &change.summary {
            sql.push_str(&format!("-- `{}`: {line}.\n", table.name));
        }
        for statement in &change.statements {
            sql.push_str(statement);
            sql.push('\n');
        }
        sql.push('\n');
    }

    // The plan must bring the history to the declarations exactly;
    // check that before handing it to a person.
    history.execute_batch(&sql).context("the planned migration does not run on the migrated schema")?;
    for schema in &schemas {
        let stored = stored_columns(&history, &schema.name)?;
        if !matches!(plan_table_change(schema, &stored), Ok(None)) {
            bail!("the planned migration leaves table `{}` different from its declaration; this is a planner bug", schema.name);
        }
    }

    let verb = if changed.is_empty() { "create" } else { "change" };
    let names: Vec<&str> = created.iter().copied().chain(changed.iter().map(|(t, _)| *t)).map(|t| t.name.as_str()).collect();
    let file_name = format!("{}_{verb}_{}.sql", next_version(&migrations), names.join("_").to_lowercase());
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
    println!("plan: wrote {} for {}", path.display(), names.join(", "));
    Ok(0)
}

/// A table's columns as the migrations left them; empty if no migration
/// creates it.
fn stored_columns(history: &Connection, table: &str) -> Result<Vec<StoredColumn>> {
    let mut statement = history.prepare("SELECT name, type, \"notnull\", pk FROM pragma_table_info(?1)")?;
    let columns = statement.query_map([table], |row| {
        Ok(StoredColumn {
            name: row.get(0)?,
            sql_type: row.get(1)?,
            not_null: row.get::<_, i64>(2)? != 0,
            primary_key: row.get::<_, i64>(3)? != 0,
        })
    })?;
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
    use corvid_runtime::tables::TableSchema;

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
            renamed: Vec::new(),
            retired: Vec::new(),
        }
    }

    #[test]
    fn the_create_statement_follows_the_column_mapping() {
        let sql = create_table_sql(&schema());
        assert_eq!(
            sql,
            "CREATE TABLE \"Shelf\" (\n    \"shelf\" INTEGER PRIMARY KEY NOT NULL,\n    \"title\" TEXT NOT NULL,\n    \"note\" TEXT,\n    \"tags\" JSON TEXT NOT NULL,\n    \"revision\" INTEGER NOT NULL\n);\n"
        );
        let history = Connection::open_in_memory().unwrap();
        history.execute_batch(&sql).expect("the statement runs");
        let stored = stored_columns(&history, "Shelf").unwrap();
        assert_eq!(stored.len(), 5);
        assert_eq!(plan_table_change(&schema(), &stored).unwrap(), None, "a created table matches its declaration");
    }
}
