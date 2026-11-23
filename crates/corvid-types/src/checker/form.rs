use super::{import_call::imported_decl, Checker};
use crate::errors::{TypeError, TypeErrorKind};
use crate::Type;
use corvid_ast::{Decl, Expr, Ident, Literal, Span, TypeRef};
use std::path::Path;

impl Checker<'_> {
    fn form_error(&mut self, span: Span, message: String) {
        self.errors
            .push(TypeError::new(TypeErrorKind::InvalidForm { message }, span));
    }

    fn record_decl(&self, ty: &Type) -> Option<&corvid_ast::TypeDecl> {
        match ty {
            Type::Struct(id) => self.types_by_id.get(id).copied(),
            Type::ImportedStruct(t) => self
                .module_resolution
                .and_then(|modules| modules.lookup_by_path(Path::new(&t.module_path)))
                .and_then(|module| imported_decl(module, t.def_id))
                .and_then(|d| {
                    if let Decl::Type(record) = d {
                        Some(record)
                    } else {
                        None
                    }
                }),
            _ => None,
        }
    }

    pub(super) fn check_browser_fields(&mut self, name: &Ident, args: &[Expr]) -> Type {
        let result = Type::Map(
            Box::new(Type::String),
            Box::new(Type::List(Box::new(Type::Int))),
        );
        let [initial] = args else {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: "browser state".into(),
                    expected: 1,
                    got: args.len(),
                },
                name.span,
            ));
            return result;
        };
        let ty = self.check_expr(initial);
        let invalid = self.record_decl(&ty).is_none_or(|record| {
            record.fields.is_empty()
                || !record.variants.is_empty()
                || record
                    .fields
                    .iter()
                    .any(|field| field.browser_integer_bounds().is_none())
        });
        if invalid {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: "a record of Int fields with ordered, browser-safe between bounds"
                        .into(),
                    got: ty.display_name(),
                    context: "browser state".into(),
                },
                initial.span(),
            ));
        }
        result
    }

    pub(super) fn check_form_record(&mut self, ty: &Type, span: Span) {
        let record = self.record_decl(ty);
        let Some(record) = record.filter(|r| r.variants.is_empty() && r.alias.is_none()) else {
            self.form_error(
                span,
                format!("expected a record for the form, got {}", ty.display_name()),
            );
            return;
        };
        if let Some(field) = record.fields.iter().find(|f| {
            !matches!(&f.ty, TypeRef::Named { name, .. }
            if matches!(name.name.as_str(), "Int" | "Float" | "Bool" | "String"))
        }) {
            self.form_error(
                span,
                format!(
                    "field `{}` must be String, Int, Float or Bool",
                    field.name.name
                ),
            );
        }
    }

    pub(super) fn check_form_for(&mut self, name: &Ident, args: &[Expr]) -> Type {
        let [target, initial, label] = args else {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.name.clone(),
                    expected: 3,
                    got: args.len(),
                },
                name.span,
            ));
            return Type::String;
        };
        let label_ty = self.check_expr(label);
        if label_ty != Type::String {
            self.form_error(label.span(), "the submit label must be a String".into());
        }
        self.check_expr(target);
        let Expr::Literal {
            value: Literal::String(action),
            ..
        } = target
        else {
            self.form_error(
                target.span(),
                "use a route literal such as `GET /search`".into(),
            );
            return Type::String;
        };
        let route = self
            .file
            .decls
            .iter()
            .filter_map(|d| match d {
                Decl::Server(server) => Some(server),
                _ => None,
            })
            .flat_map(|s| &s.routes)
            .find(|r| action.split_once(' ') == Some((r.method.as_str(), r.path.as_str())));
        let Some(r) = route else {
            self.form_error(
                target.span(),
                format!("no declared route `{action}` in this module"),
            );
            return Type::String;
        };
        let record = match r.method.as_str() {
            "GET" if r.body_ty.is_none() => r.query_ty.as_ref(),
            "POST" if r.form_encoded && r.query_ty.is_none() => r.body_ty.as_ref(),
            _ => None,
        };
        let Some(record) = record.filter(|_| {
            r.path_params.is_empty()
                && r.path.starts_with('/')
                && !r.path.starts_with("//")
                && !r.path.contains(['\\', '?', '#'])
                && !r.path.chars().any(char::is_control)
        }) else {
            self.form_error(target.span(), format!("`{action}` requires a fixed application path and either GET query Record or POST form Record"));
            return Type::String;
        };
        let expected = self.type_ref_to_type(record);
        self.check_form_record(&expected, target.span());
        let got = self.check_expr_as(initial, Some(&expected));
        if got != expected {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: expected.display_name(),
                    got: got.display_name(),
                    context: format!("initial values for `{action}`"),
                },
                initial.span(),
            ));
        }
        Type::String
    }
}
