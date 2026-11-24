//! B06 — refusing the B06 types as table columns until B09 maps them.
//!
//! `Decimal`, the time types, `Currency` and `Locale` cross JSON as
//! their exact text everywhere else (routes, `to_json`, tools, prompts,
//! stores). A table column is different: how it is stored decides how
//! `where` and `order_by` compare it (a decimal stored as text sorts
//! wrong; Postgres timestamps hold microseconds, not nanoseconds), so
//! the mapping belongs with B09's query and migration planner. Until
//! it lands, a table that would store one is a compile error. The walk
//! is `to_json`'s, so a record column holding one is caught too.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{Decl, File};

impl<'a> Checker<'a> {
    /// Runs after `record_signature_types`, so every type is resolved.
    pub(super) fn check_json_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            let Decl::Table(table) = decl else { continue };
            for field in &table.record.fields {
                let Some(ty) = self.type_refs.get(&field.ty.span()).cloned() else { continue };
                if let Some(type_name) = self.b06_type_reached(&ty) {
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
