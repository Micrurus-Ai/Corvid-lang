//! Where a connector `Response` may appear (B05).
//!
//! A `Response<Body>` holds a provider's status and headers next to the
//! body. It is a value for the program that made the call: an agent or
//! function can take one, return one and keep one in a local. It has no
//! JSON form and is not part of any public surface, so it never appears
//! in a record field, a route, a tool, a prompt, a store, a table, a
//! connector operation's parameters, or the signature of `main` or an
//! exported agent. A connector operation returns one only as its whole
//! result or as its `Result`'s success type; `check_connector` checks
//! that shape.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{Decl, ExtendMethodKind, Field, File, Param, TypeRef};

impl<'a> Checker<'a> {
    /// Runs after `record_signature_types`, so every type is resolved.
    pub(super) fn check_response_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            match decl {
                Decl::Type(ty) => {
                    let record = format!("a field of `{}`", ty.name.name);
                    self.refuse_response_in_fields(&ty.fields, &record);
                    for variant in &ty.variants {
                        let place = format!("a field of the `{}` variant", variant.name.name);
                        self.refuse_response_in_fields(&variant.fields, &place);
                    }
                }
                Decl::Store(store) => self.refuse_response_in_fields(&store.fields, "a store field"),
                Decl::Table(table) => self.refuse_response_in_fields(&table.record.fields, "a table column"),
                Decl::Tool(tool) => self.refuse_response_in_signature(&tool.params, &tool.return_ty, "a tool's signature"),
                Decl::Prompt(prompt) => {
                    self.refuse_response_in_signature(&prompt.params, &prompt.return_ty, "a prompt's signature")
                }
                Decl::Agent(agent) if agent.name.name == "main" => {
                    self.refuse_response_in_signature(&agent.params, &agent.return_ty, "the `main` agent's signature")
                }
                Decl::Agent(agent) if agent.extern_abi.is_some() => {
                    self.refuse_response_in_signature(&agent.params, &agent.return_ty, "an exported agent's signature")
                }
                Decl::Server(server) => {
                    for route in &server.routes {
                        let inputs = route
                            .path_params
                            .iter()
                            .map(|param| &param.ty)
                            .chain(route.query_ty.iter())
                            .chain(route.body_ty.iter())
                            .chain(std::iter::once(&route.response.ty));
                        for ty in inputs {
                            self.refuse_response_at(ty, "a route's inputs or response");
                        }
                    }
                }
                Decl::Connector(connector) => {
                    for op in &connector.operations {
                        for param in &op.params {
                            self.refuse_response_at(&param.ty, "a connector operation's parameters");
                        }
                    }
                }
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        if let ExtendMethodKind::Prompt(prompt) = &method.kind {
                            self.refuse_response_in_signature(&prompt.params, &prompt.return_ty, "a prompt's signature");
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn refuse_response_in_fields(&mut self, fields: &[Field], place: &str) {
        for field in fields {
            self.refuse_response_at(&field.ty, place);
        }
    }

    fn refuse_response_in_signature(&mut self, params: &[Param], return_ty: &TypeRef, place: &str) {
        for param in params {
            self.refuse_response_at(&param.ty, place);
        }
        self.refuse_response_at(return_ty, place);
    }

    fn refuse_response_at(&mut self, ty: &TypeRef, place: &str) {
        let Some(resolved) = self.type_refs.get(&ty.span()) else { return };
        if resolved.mentions_response() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ResponseOutsideCaller { place: place.to_string() },
                ty.span(),
            ));
        }
    }
}
