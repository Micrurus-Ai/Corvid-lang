//! The table columns a database cannot store yet.
//!
//! B09 P8a stores `Decimal`, the time types, `Currency` and `Locale` in
//! columns that SQLite compares exactly as Corvid does (see
//! [`crate::table_columns`]). Two cases still wait, and are compile
//! errors that name what they wait for:
//!
//! - `Bytes`, anywhere in a column, until B09 P8c gives it a binary
//!   column. The walk is `to_json`'s, so a record column holding one is
//!   caught too.
//! - Those exact columns on a Postgres database, until B09 P8b gives
//!   each a Postgres type that compares it exactly. Inside a JSON-stored
//!   column they are fine on either engine: JSON holds their exact text,
//!   and the database never compares inside it.

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
            for field in &table.record.fields {
                let Some(ty) = self.type_refs.get(&field.ty.span()).cloned() else { continue };
                let kind = column_kind(&ty).0;
                if on_postgres && kind.is_exact_text() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ExactColumnNotYetOnPostgres {
                            table: table_name.clone(),
                            column: field.name.name.clone(),
                            type_name: kind.type_name().to_string(),
                        },
                        field.ty.span(),
                    ));
                    continue;
                }
                if let Some(type_name) = self.unstored_column_type_reached(&ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::PendingJsonFormAtBoundary {
                            type_name: type_name.to_string(),
                            boundary: "a table column".to_string(),
                        },
                        field.ty.span(),
                    ));
                }
            }
        }
    }
}
