//! Request text in a route: the types a path segment or a query field
//! can have, checked with the shape the server reads them through
//! ([`crate::text_parameters`]).

use super::json_methods::imported_type_decl;
use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::text_parameters::query_field_shape;
use crate::types::Type;
use corvid_ast::{HttpRouteDecl, Span, TypeDecl};
use std::path::Path;

impl<'a> Checker<'a> {
    /// Whether `ty` is a declared sum type whose variants all have no
    /// fields, so request text can name one (`?status=Open`).
    pub(super) fn has_plain_variants(&self, ty: &Type) -> bool {
        self.declared_type(ty)
            .is_some_and(|(decl, _)| !decl.variants.is_empty() && decl.variants.iter().all(|v| v.fields.is_empty()))
    }

    /// A route's query record has fields the server can read from a
    /// query string: one value, an `Option` of one, or a `List` of one.
    pub(super) fn check_route_query_fields(&mut self, route: &HttpRouteDecl, query: &Type, span: Span) {
        let Some((decl, module)) = self.declared_type(query) else {
            if *query != Type::Unknown {
                self.errors.push(TypeError::new(
                    TypeErrorKind::RouteQueryFieldType {
                        path: route.path.clone(),
                        field: String::new(),
                        ty: query.display_name(),
                    },
                    span,
                ));
            }
            return;
        };
        let fields: Vec<(String, Type, Span)> = decl
            .fields
            .iter()
            .map(|field| {
                let ty = match module {
                    Some(module) => self.imported_type_ref_to_type(&field.ty, module),
                    None => self.type_ref_to_type(&field.ty),
                };
                (field.name.name.clone(), ty, field.span)
            })
            .collect();
        for (name, ty, field_span) in fields {
            let plain = |ty: &Type| self.has_plain_variants(ty);
            if ty != Type::Unknown && query_field_shape(&ty, &plain).is_none() {
                // An imported record's field lives in another file; the
                // route is where this program uses it as a query.
                let at = if module.is_some() { span } else { field_span };
                self.errors.push(TypeError::new(
                    TypeErrorKind::RouteQueryFieldType { path: route.path.clone(), field: name, ty: ty.display_name() },
                    at,
                ));
            }
        }
    }

    /// The declaration of a record or sum type, and its module when it
    /// was imported.
    fn declared_type(&self, ty: &Type) -> Option<(&'a TypeDecl, Option<&'a corvid_resolve::ResolvedModule>)> {
        match ty {
            Type::Struct(id) => self.types_by_id.get(id).copied().map(|decl| (decl, None)),
            Type::ImportedStruct(imported) => {
                let module = self.module_resolution?.lookup_by_path(Path::new(&imported.module_path))?;
                imported_type_decl(module, imported.def_id).map(|decl| (decl, Some(module)))
            }
            _ => None,
        }
    }
}
