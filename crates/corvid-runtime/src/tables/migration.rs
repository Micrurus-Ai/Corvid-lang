//! Planning a change to a stored table (B09 P3).
//!
//! The planner compares a declared table with the columns its stored
//! version has and writes the SQL that turns one into the other. It
//! never guesses about data:
//!
//! - A column is renamed only when the declaration says
//!   `renamed from`, because a rename looks the same as a drop and an
//!   add.
//! - A stored column is dropped only when the declaration says
//!   `retired`, because dropping loses its data. Any other stored
//!   column the declaration lacks is refused by name.
//! - A new column must be an `Option`, because Corvid has no value to
//!   give existing rows. The refusal says how to add a required column:
//!   add it as an `Option`, fill it, then make it required.
//! - A column's type may change only where every stored value converts
//!   exactly: to or from an `Option` of the same type, and from `Int`
//!   to `Float`. Making a column required fails while a row still has
//!   no value, and the migration then leaves the table as it was.
//!
//! Renames, new columns and retired columns use `ALTER TABLE`. On
//! Postgres a type or nullability change is `ALTER COLUMN` too. SQLite
//! cannot alter a column, so there the change rebuilds the table:
//! create it under a temporary name, copy every row, drop the old table
//! and rename the new one. `corvid migrate up` runs each migration in one transaction, so a
//! failed copy changes nothing.

use super::{create_table_named, quote, ColumnKind, TableSchema};
use crate::tables::column_kind;
use std::collections::BTreeSet;

/// One column of a stored table, as the engine's catalog reports it
/// (`pragma_table_info` on SQLite, `information_schema` on Postgres).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredColumn {
    pub name: String,
    pub sql_type: String,
    pub not_null: bool,
    pub primary_key: bool,
}

/// The statements that bring a stored table to its declaration, and a
/// line per change for the person reviewing them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableChange {
    pub statements: Vec<String>,
    pub summary: Vec<String>,
}

/// Where a declared column's values come from.
enum Source<'a> {
    /// A stored column, under this name.
    Stored(&'a StoredColumn),
    /// No stored column: the column is new.
    New,
}

/// Plan the change from `stored` to `table`. `Ok(None)` when the stored
/// table already matches; `Err` lists every change the planner refuses.
pub fn plan_table_change(table: &TableSchema, stored: &[StoredColumn]) -> Result<Option<TableChange>, Vec<String>> {
    let by_name = |name: &str| stored.iter().find(|c| c.name == name);
    let mut refusals = Vec::new();
    let mut claimed = BTreeSet::new();
    let mut columns = Vec::new();
    for (name, ty) in &table.columns {
        let renamed_from = table.renamed.iter().find(|(field, _)| field == name).map(|(_, old)| old.as_str());
        let source = match (by_name(name), renamed_from.and_then(by_name)) {
            (Some(column), _) => Source::Stored(column),
            (None, Some(column)) => Source::Stored(column),
            (None, None) => Source::New,
        };
        if let Source::Stored(column) = &source {
            claimed.insert(column.name.clone());
        }
        columns.push((name.as_str(), ty, source));
    }

    let mut retired = Vec::new();
    for column in stored {
        if claimed.contains(&column.name) {
            continue;
        }
        if table.retired.contains(&column.name) {
            retired.push(column.name.as_str());
        } else {
            refusals.push(format!(
                "table `{}` stores column `{}`, which it no longer declares; declare it again, name it in `renamed from` on the field it became, or write `retired {}` to drop it and its data",
                table.name, column.name, column.name
            ));
        }
    }

    let mut renames = Vec::new();
    let mut added = Vec::new();
    let mut altered: Vec<Alteration<'_>> = Vec::new();
    for (name, ty, source) in &columns {
        let (kind, nullable) = column_kind(ty);
        let is_key = *name == table.key;
        match source {
            Source::New => {
                if is_key {
                    refusals.push(format!("table `{}` gains the key `{name}`; changing a table's key is not planned, so create a new table", table.name));
                } else if !nullable {
                    refusals.push(format!(
                        "table `{}` gains the required column `{name}`, and existing rows have no value for it; declare it as an `Option` first, fill it, then make it required",
                        table.name
                    ));
                } else {
                    added.push((*name, kind));
                }
            }
            Source::Stored(column) => {
                if column.name != *name {
                    renames.push((column.name.as_str(), *name));
                }
                if column.primary_key != is_key {
                    refusals.push(format!(
                        "table `{}`: `{name}` {} the stored key; changing a table's key is not planned, so create a new table",
                        table.name,
                        if is_key { "is declared the key but is not" } else { "is no longer the key but is" }
                    ));
                    continue;
                }
                let Some(stored_kind) = table.engine.kind_of(&column.sql_type) else {
                    refusals.push(format!(
                        "table `{}`: column `{}` has the type `{}`, which Corvid does not create, so the planner cannot tell how its values are stored",
                        table.name, column.name, column.sql_type
                    ));
                    continue;
                };
                let kind_changes = stored_kind != kind;
                let null_changes = column.not_null == nullable;
                if kind_changes && !(stored_kind == ColumnKind::Integer && kind == ColumnKind::Real) {
                    refusals.push(format!(
                        "table `{}`: column `{name}` changes from {} to {}, which does not convert every stored value exactly; add a new column, fill it from this one, then retire this one",
                        table.name,
                        kind_label(stored_kind),
                        kind_label(kind)
                    ));
                } else if kind_changes || null_changes {
                    altered.push(Alteration {
                        name,
                        label: change_label(name, stored_kind, column.not_null, kind, nullable),
                        new_type: kind_changes.then_some(kind),
                        nullable: null_changes.then_some(nullable),
                    });
                }
            }
        }
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    if renames.is_empty() && added.is_empty() && retired.is_empty() && altered.is_empty() {
        return Ok(None);
    }

    let mut summary = Vec::new();
    summary.extend(renames.iter().map(|(old, new)| format!("rename `{old}` to `{new}`")));
    summary.extend(added.iter().map(|(name, _)| format!("add `{name}`")));
    summary.extend(retired.iter().map(|name| format!("drop retired `{name}` and its data")));
    summary.extend(altered.iter().map(|a| a.label.clone()));

    let table_name = quote(&table.name);
    let statements = if altered.is_empty() || !table.engine.rebuilds_to_alter() {
        let mut statements = Vec::new();
        for (old, new) in &renames {
            statements.push(format!("ALTER TABLE {table_name} RENAME COLUMN {} TO {};", quote(old), quote(new)));
        }
        for (name, kind) in &added {
            statements.push(format!("ALTER TABLE {table_name} ADD COLUMN {} {};", quote(name), table.engine.column_type(*kind)));
        }
        for name in &retired {
            statements.push(format!("ALTER TABLE {table_name} DROP COLUMN {};", quote(name)));
        }
        // Postgres alters a column in place; renames ran first, so the
        // column has its declared name.
        for alteration in &altered {
            let column = quote(alteration.name);
            if let Some(kind) = alteration.new_type {
                statements.push(format!("ALTER TABLE {table_name} ALTER COLUMN {column} TYPE {};", table.engine.column_type(kind)));
            }
            match alteration.nullable {
                Some(true) => statements.push(format!("ALTER TABLE {table_name} ALTER COLUMN {column} DROP NOT NULL;")),
                Some(false) => statements.push(format!("ALTER TABLE {table_name} ALTER COLUMN {column} SET NOT NULL;")),
                None => {}
            }
        }
        statements
    } else {
        rebuild_statements(table, &columns)
    };
    Ok(Some(TableChange { statements, summary }))
}

/// A stored column whose type or nullability changes.
struct Alteration<'a> {
    name: &'a str,
    label: String,
    /// The new storage kind, when it changes.
    new_type: Option<ColumnKind>,
    /// Whether the column now admits no value, when that changes.
    nullable: Option<bool>,
}

/// Create the declared table under a temporary name, copy every row
/// into it converting where the type changed, then replace the old one.
fn rebuild_statements(table: &TableSchema, columns: &[(&str, &corvid_types::Type, Source<'_>)]) -> Vec<String> {
    let temporary = format!("{}__corvid_rebuild", table.name);
    let mut targets = Vec::new();
    let mut values = Vec::new();
    for (name, ty, source) in columns {
        let Source::Stored(column) = source else { continue };
        targets.push(quote(name));
        let converts_to_real = column_kind(ty).0 == ColumnKind::Real
            && table.engine.kind_of(&column.sql_type) == Some(ColumnKind::Integer);
        values.push(if converts_to_real {
            format!("CAST({} AS REAL)", quote(&column.name))
        } else {
            quote(&column.name)
        });
    }
    vec![
        create_table_named(table, &temporary).trim_end().to_string(),
        format!(
            "INSERT INTO {} ({}) SELECT {} FROM {};",
            quote(&temporary),
            targets.join(", "),
            values.join(", "),
            quote(&table.name)
        ),
        format!("DROP TABLE {};", quote(&table.name)),
        format!("ALTER TABLE {} RENAME TO {};", quote(&temporary), quote(&table.name)),
    ]
}

fn kind_label(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Integer => "`Int`",
        ColumnKind::Real => "`Float`",
        ColumnKind::Text => "`String`",
        ColumnKind::Boolean => "`Bool`",
        ColumnKind::Json => "a JSON-stored type",
    }
}

fn change_label(name: &str, from: ColumnKind, from_not_null: bool, to: ColumnKind, to_nullable: bool) -> String {
    let describe = |kind, optional: bool| {
        if optional { format!("an optional {}", kind_label(kind)) } else { kind_label(kind).to_string() }
    };
    let mut label = format!("change `{name}` from {} to {}", describe(from, !from_not_null), describe(to, to_nullable));
    if from_not_null == to_nullable && !to_nullable {
        label.push_str("; the migration fails, changing nothing, while any row has no value");
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_types::Type;

    fn column(name: &str, sql_type: &str, not_null: bool, primary_key: bool) -> StoredColumn {
        StoredColumn { name: name.into(), sql_type: sql_type.into(), not_null, primary_key }
    }

    fn stored() -> Vec<StoredColumn> {
        vec![
            column("id", "INTEGER", true, true),
            column("name", "TEXT", true, false),
            column("count", "INTEGER", true, false),
            column("legacy", "TEXT", false, false),
            column("revision", "INTEGER", true, false),
        ]
    }

    fn table(columns: Vec<(&str, Type)>, renamed: &[(&str, &str)], retired: &[&str]) -> TableSchema {
        TableSchema {
            name: "Shelf".into(),
            key: "id".into(),
            columns: columns.into_iter().map(|(n, t)| (n.to_string(), t)).collect(),
            renamed: renamed.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            retired: retired.iter().map(|s| s.to_string()).collect(),
            engine: crate::tables::TableEngine::Sqlite,
        }
    }

    fn optional(ty: Type) -> Type {
        Type::Option(Box::new(ty))
    }

    #[test]
    fn renames_additions_and_retirements_alter_the_table() {
        let declared = table(
            vec![
                ("id", Type::Int),
                ("title", Type::String),
                ("count", Type::Int),
                ("note", optional(Type::String)),
                ("revision", Type::Int),
            ],
            &[("title", "name")],
            &["legacy"],
        );
        let change = plan_table_change(&declared, &stored()).unwrap().expect("a change");
        assert_eq!(
            change.statements,
            vec![
                "ALTER TABLE \"Shelf\" RENAME COLUMN \"name\" TO \"title\";",
                "ALTER TABLE \"Shelf\" ADD COLUMN \"note\" TEXT;",
                "ALTER TABLE \"Shelf\" DROP COLUMN \"legacy\";",
            ]
        );
    }

    #[test]
    fn an_undeclared_drop_and_a_required_addition_are_refused() {
        let declared = table(
            vec![("id", Type::Int), ("name", Type::String), ("count", Type::Int), ("shelf", Type::String), ("revision", Type::Int)],
            &[],
            &[],
        );
        let refusals = plan_table_change(&declared, &stored()).unwrap_err();
        assert!(refusals.iter().any(|r| r.contains("stores column `legacy`") && r.contains("retired legacy")), "{refusals:#?}");
        assert!(refusals.iter().any(|r| r.contains("required column `shelf`")), "{refusals:#?}");
    }

    #[test]
    fn an_exact_type_change_rebuilds_and_a_lossy_one_is_refused() {
        let widened = table(
            vec![("id", Type::Int), ("name", optional(Type::String)), ("count", Type::Float), ("revision", Type::Int)],
            &[],
            &["legacy"],
        );
        let change = plan_table_change(&widened, &stored()).unwrap().expect("a change");
        assert!(change.statements[0].starts_with("CREATE TABLE \"Shelf__corvid_rebuild\""), "{:#?}", change.statements);
        assert_eq!(
            change.statements[1],
            "INSERT INTO \"Shelf__corvid_rebuild\" (\"id\", \"name\", \"count\", \"revision\") SELECT \"id\", \"name\", CAST(\"count\" AS REAL), \"revision\" FROM \"Shelf\";"
        );

        let lossy = table(
            vec![("id", Type::Int), ("name", Type::Int), ("count", Type::Int), ("revision", Type::Int)],
            &[],
            &["legacy"],
        );
        let refusals = plan_table_change(&lossy, &stored()).unwrap_err();
        assert!(refusals.iter().any(|r| r.contains("`name` changes from `String` to `Int`")), "{refusals:#?}");
    }

    #[test]
    fn a_matching_table_needs_no_change() {
        let same = table(
            vec![("id", Type::Int), ("name", Type::String), ("count", Type::Int), ("revision", Type::Int)],
            &[("name", "old_name")],
            &["legacy", "long_gone"],
        );
        let mut columns = stored();
        columns.retain(|c| c.name != "legacy");
        assert_eq!(plan_table_change(&same, &columns).unwrap(), None);
    }

    #[test]
    fn a_planned_rebuild_runs_on_sqlite() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE \"Shelf\" (\"id\" INTEGER PRIMARY KEY NOT NULL, \"name\" TEXT NOT NULL, \"count\" INTEGER NOT NULL, \"legacy\" TEXT, \"revision\" INTEGER NOT NULL);
                 INSERT INTO \"Shelf\" VALUES (1, 'Reading', 3, 'x', 2);",
            )
            .unwrap();
        let declared = table(
            vec![("id", Type::Int), ("title", optional(Type::String)), ("count", Type::Float), ("revision", Type::Int)],
            &[("title", "name")],
            &["legacy"],
        );
        let change = plan_table_change(&declared, &stored()).unwrap().expect("a change");
        connection.execute_batch(&change.statements.join("\n")).expect("the plan runs");
        let (title, count): (String, f64) = connection
            .query_row("SELECT \"title\", \"count\" FROM \"Shelf\" WHERE \"id\" = 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!((title.as_str(), count), ("Reading", 3.0));
    }
}
