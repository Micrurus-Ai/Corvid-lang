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

    pub(super) fn track_browser_model(&mut self, local: corvid_resolve::LocalId, value: &Expr) {
        // A browser-state declaration expands to browser_record(__browser_fields(initial)).
        let model = match value {
            Expr::Call { callee, args, .. } if callee.span().expansion.is_some() => {
                match args.as_slice() {
                    [Expr::Call { callee, args, .. }]
                        if matches!(
                            self.bindings.get(&callee.span()),
                            Some(corvid_resolve::Binding::BuiltIn(
                                corvid_resolve::BuiltIn::BrowserFields
                            ))
                        ) =>
                    {
                        args.first()
                            .and_then(|initial| self.types.get(&initial.span()))
                            .cloned()
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if self.local_types.contains_key(&local) {
            if let Some(previous) = self.browser_models.get_mut(&local) {
                if *previous != model {
                    *previous = None;
                }
            }
        } else if model.is_some() {
            self.browser_models.insert(local, model);
        }
    }

    pub(super) fn check_browser_field_key(&mut self, name: &Ident, args: &[Expr]) -> Type {
        let [owner, Expr::Literal {
            value: Literal::String(key),
            span,
        }, condition @ ..] = args
        else {
            self.form_error(name.span, "expected bind state.field".into());
            return Type::String;
        };
        self.check_expr(owner);
        let model = match self.bindings.get(&owner.span()) {
            Some(corvid_resolve::Binding::Local(id)) => {
                self.browser_models.get(id).cloned().flatten()
            }
            _ => None,
        };
        let Some(model) = model else {
            self.form_error(
                owner.span(),
                "bind needs a named browser state with a known record type".into(),
            );
            return Type::String;
        };
        let mut ty = model;
        for part in key.split('.') {
            let field = self
                .record_decl(&ty)
                .and_then(|record| record.fields.iter().find(|f| f.name.name == part))
                .map(|field| field.ty.clone());
            let Some(field) = field else {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnknownField {
                        struct_name: self
                            .record_decl(&ty)
                            .map_or_else(|| ty.display_name(), |r| r.name.name.clone()),
                        field: part.into(),
                    },
                    *span,
                ));
                return Type::String;
            };
            ty = self.browser_member_type(&ty, &field);
        }
        if self.record_decl(&ty).is_some_and(|r| r.variants.is_empty()) {
            self.form_error(*span, "bind needs a field inside this record".into());
        }
        match condition {
            [] => {}
            [Expr::Literal { value: Literal::Bool(true), span }] => {
                if ty != Type::Bool {
                    self.form_error(*span, "a condition without a comparison needs a Bool field".into());
                }
            }
            [Expr::Literal { value: Literal::String(variant), span }] => {
                let valid = self.record_decl(&ty).is_some_and(|r|
                    r.variants.iter().any(|v| v.name.name == *variant && v.fields.is_empty()));
                if !valid {
                    self.form_error(*span, format!("expected a choice field with a variant named `{variant}`"));
                }
            }
            _ => self.form_error(name.span, "expected a named choice in the view condition".into()),
        }
        self.browser_field_types.insert(name.span, ty);
        Type::String
    }

    pub(super) fn check_bound_control(&mut self, control: &str, args: &[Expr]) {
        let (expected, indices): (&str, &[usize]) = match control {
            "local_slider" => ("Int", &[1]),
            "record_swatch" => ("Int", &[0, 1, 2]),
            "local_checkbox" | "checkbox" => ("Bool", &[1]),
            "segmented_choice" => ("choice", &[1]),
            _ => return,
        };
        for &index in indices {
            let Some(arg) = args.get(index) else { continue };
            let Some(ty) = self.browser_field_types.get(&arg.span()) else {
                continue;
            };
            let valid = match expected {
                "Int" => *ty == Type::Int,
                "Bool" => *ty == Type::Bool,
                _ => self.record_decl(ty).is_some_and(|r| !r.variants.is_empty()),
            };
            if !valid {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: expected.into(),
                        got: ty.display_name(),
                        context: format!("{control} field binding"),
                    },
                    arg.span(),
                ));
            }
        }
    }

    pub(super) fn check_browser_fields(&mut self, name: &Ident, args: &[Expr]) -> Type {
        let result = Type::Map(
            Box::new(Type::String),
            Box::new(Type::List(Box::new(Type::String))),
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
        let invalid = !self.valid_browser_record(&ty, &mut Vec::new());
        if invalid {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected:
                        "a record of bounded integers, text, booleans or choices without payloads"
                            .into(),
                    got: ty.display_name(),
                    context: "browser state".into(),
                },
                initial.span(),
            ));
        }
        result
    }

    fn browser_member_type(&mut self, owner: &Type, field: &TypeRef) -> Type {
        if let Type::ImportedStruct(t) = owner {
            if let Some(module) = self
                .module_resolution
                .and_then(|m| m.lookup_by_path(Path::new(&t.module_path)))
            {
                return self.imported_type_ref_to_type(field, module);
            }
        }
        self.type_ref_to_type(field)
    }

    fn valid_browser_record(&mut self, ty: &Type, parents: &mut Vec<Type>) -> bool {
        if parents.contains(ty) {
            return false;
        }
        parents.push(ty.clone());
        let fields = self
            .record_decl(ty)
            .filter(|r| r.variants.is_empty())
            .map(|r| r.fields.clone());
        let invalid = fields.is_none_or(|fields| {
            fields.is_empty()
                || fields.iter().any(|field| {
                    if field.browser_integer_bounds().is_some() {
                        return false;
                    }
                    let field_ty = self.browser_member_type(ty, &field.ty);
                    match field_ty {
                        Type::Bool => field.refinement.is_some(),
                        Type::String => match field.refinement {
                            None => false,
                            Some(corvid_ast::Refinement::LenBetween { min, max }) => {
                                min > max || max > (1 << 53) - 1
                            }
                            _ => true,
                        },
                        _ if self
                            .record_decl(&field_ty)
                            .is_some_and(|r| !r.variants.is_empty()) =>
                        {
                            field.refinement.is_some()
                                || self
                                    .record_decl(&field_ty)
                                    .unwrap()
                                    .variants
                                    .iter()
                                    .any(|v| !v.fields.is_empty())
                        }
                        _ => {
                            field.refinement.is_some()
                                || !self.valid_browser_record(&field_ty, parents)
                        }
                    }
                })
        });
        parents.pop();
        !invalid
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
