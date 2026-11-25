//! The table columns a database cannot store yet.
//!
//! B09 P8a stores `Decimal`, the time types, `Currency` and `Locale` in
//! columns that SQLite compares exactly as Corvid does, and P8c stores
//! `Bytes` as binary on both engines (see [`crate::table_columns`]). One
//! case still waits and is a compile error that names what it waits for:
//! the money and time columns on a Postgres database, until B09 P8b gives
//! each a Postgres type that compares it exactly. Inside a JSON-stored
//! column they are fine on either engine: JSON holds their exact text,
//! and the database never compares inside it.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::table_columns::column_kind;
use corvid_ast::{Decl, File};

impl<'a> Checker<'a> {
    /// Runs after `record_signature_types`, so every type is resolved.
    pub(super) fn check_json_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            let Decl::Table(table) = decl else { continue };
            let table_name = &table.name().name;
            let on_postgres = file.decls.iter().any(|decl| match decl {
                Decl::Database(db) => {
                    db.tables.iter().any(|t| t.name == *table_name)
                        && db.engine.as_ref().is_some_and(|engine| engine.name == "postgres")
                }
                _ => false,
            });
            if !on_postgres {
                continue;
            }
            for field in &table.record.fields {
                let Some(ty) = self.type_refs.get(&field.ty.span()).cloned() else { continue };
                let kind = column_kind(&ty).0;
                if kind.is_exact_text() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ExactColumnNotYetOnPostgres {
                            table: table_name.clone(),
                            column: field.name.name.clone(),
                            type_name: kind.type_name().to_string(),
                        },
                        field.ty.span(),
                    ));
                }
            }
        }
    }
}
