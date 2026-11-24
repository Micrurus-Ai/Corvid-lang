//! B06 — refusing the B06 types where a value would leave the program
//! as JSON before B06's JSON slice gives them a published form.
//!
//! Without this a `Decimal` or an `Instant` would cross a route, store
//! or table as whatever that boundary's fallback produced, which is
//! not a decision anyone made. Tools and prompts are the exception for
//! `Duration` and `Instant`: on the interpreter they carry them as the
//! standard text `corvid_time` reads and writes (RFC 3339, ISO 8601).
//! The walk is `to_json`'s, so a record with such a field is caught.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{Decl, ExtendMethodKind, File, Param, TypeRef};

impl<'a> Checker<'a> {
    /// Refuses the B06 types wherever a value leaves the program as JSON
    /// before B06's JSON slice gives them a published form: `Decimal`
    /// at every boundary, and `Duration` and `Instant` everywhere but
    /// tools and prompts, which carry them as standard text.
    /// Without this a decimal would cross as whatever the boundary's
    /// fallback produced, which is not a decision anyone made. Runs
    /// after `record_signature_types`, so every type is resolved.
    pub(super) fn check_json_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            match decl {
                Decl::Tool(t) => self.check_signature_boundary(&t.params, &t.return_ty, "a tool signature", true),
                Decl::Prompt(p) => self.check_signature_boundary(&p.params, &p.return_ty, "a prompt signature", true),
                Decl::Connector(c) => {
                    for op in &c.operations {
                        self.check_signature_boundary(&op.params, &op.return_ty, "a connector operation", false);
                    }
                }
                Decl::Server(s) => {
                    for route in &s.routes {
                        for param in &route.path_params {
                            self.check_boundary(&param.ty, "a route", false);
                        }
                        for ty in route.query_ty.iter().chain(route.body_ty.iter()) {
                            self.check_boundary(ty, "a route", false);
                        }
                        self.check_boundary(&route.response.ty, "a route", false);
                    }
                }
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        match &method.kind {
                            ExtendMethodKind::Tool(t) => {
                                self.check_signature_boundary(&t.params, &t.return_ty, "a tool signature", true)
                            }
                            ExtendMethodKind::Prompt(p) => {
                                self.check_signature_boundary(&p.params, &p.return_ty, "a prompt signature", true)
                            }
                            ExtendMethodKind::Agent(_) => {}
                        }
                    }
                }
                Decl::Store(store) => {
                    for field in &store.fields {
                        self.check_boundary(&field.ty, "a store field", false);
                    }
                }
                Decl::Table(table) => {
                    for field in &table.record.fields {
                        self.check_boundary(&field.ty, "a table column", false);
                    }
                }
                _ => {}
            }
        }
    }

    fn check_signature_boundary(&mut self, params: &[Param], return_ty: &TypeRef, boundary: &str, text_boundary: bool) {
        for param in params {
            self.check_boundary(&param.ty, boundary, text_boundary);
        }
        self.check_boundary(return_ty, boundary, text_boundary);
    }

    /// `text_boundary` marks a tool or prompt, where durations and
    /// instants cross as their standard text on the interpreter.
    fn check_boundary(&mut self, ty_ref: &TypeRef, boundary: &str, text_boundary: bool) {
        let Some(ty) = self.type_refs.get(&ty_ref.span()).cloned() else { return };
        if let Some(type_name) = self.pending_json_form(&ty, text_boundary) {
            self.errors.push(TypeError::new(
                TypeErrorKind::PendingJsonFormAtBoundary { type_name: type_name.to_string(), boundary: boundary.to_string() },
                ty_ref.span(),
            ));
        }
    }

}
