use super::{import_call::imported_decl, Checker};
use crate::action_input::{is_caller, whole_input_type};
use crate::text_parameters::{QueryFieldShape, TextShape};
use crate::errors::{TypeError, TypeErrorKind};
use crate::Type;
use corvid_ast::{AgentDecl, Decl, Expr, Ident, Literal, Span, TypeRef};
use std::path::Path;

impl<'a> Checker<'a> {
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
        let model = value.browser_state_initializer()
            .and_then(|initial| self.types.get(&initial.span())).cloned();
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

    pub(super) fn check_browser_field_key(&mut self, name: &Ident, args: &[Expr], row: bool) -> Type {
        let [owner, Expr::Literal {
            value: Literal::String(key),
            span,
        }, condition @ ..] = args
        else {
            self.form_error(name.span, "expected bind state.field".into());
            return Type::String;
        };
        let owner_type = self.check_expr(owner);
        let model = if row {
            Some(owner_type)
        } else if let Some(corvid_resolve::Binding::Local(id)) = self.bindings.get(&owner.span()) {
            self.browser_models.get(id).cloned().flatten()
        } else {
            None
        };
        let Some(model) = model else {
            self.form_error(
                owner.span(),
                "bind needs a named browser state with a known record type".into(),
            );
            return Type::String;
        };
        self.check_field_path(name, model, key, *span, condition)
    }

    pub(super) fn check_field_path(
        &mut self, name: &Ident, model: Type, key: &str, field_span: Span, condition: &[Expr],
    ) -> Type {
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
                    field_span,
                ));
                return Type::String;
            };
            ty = self.browser_member_type(&ty, &field);
        }
        if self.record_decl(&ty).is_some_and(|r| r.variants.is_empty()) {
            self.form_error(field_span, "bind needs a field inside this record".into());
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
        let record = match &ty {
            Type::List(item) if name.span.expansion.is_some() => item.as_ref(),
            _ => &ty,
        };
        let invalid = !self.valid_browser_record(record, &mut Vec::new());
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
        // A field is a form control exactly when it is one piece of request
        // text by the rule the server decodes forms with
        // (`text_parameters::query_field_shape`), so a form never offers a
        // field the server would refuse, or refuses one it would read.
        let declared: Vec<(String, TypeRef)> =
            record.fields.iter().map(|field| (field.name.name.clone(), field.ty.clone())).collect();
        // An imported record's field types are resolved in its own module.
        let module = match ty {
            Type::ImportedStruct(t) => self.module_resolution.and_then(|modules| modules.lookup_by_path(Path::new(&t.module_path))),
            _ => None,
        };
        let fields: Vec<(String, Type)> = declared
            .into_iter()
            .map(|(name, field_ty)| {
                let ty = match module {
                    Some(module) => self.imported_type_ref_to_type(&field_ty, module),
                    None => self.type_ref_to_type(&field_ty),
                };
                (name, ty)
            })
            .collect();
        for (name, ty) in fields {
            let plain = |ty: &Type| self.has_plain_variants(ty);
            let shape = crate::text_parameters::query_field_shape(&ty, &plain);
            match shape {
                Some(QueryFieldShape::One(TextShape::Int | TextShape::Float | TextShape::Bool | TextShape::String | TextShape::PlainVariant)) => {}
                // The server reads these, but a form does not draw a
                // control for them yet; refused by name, never degraded.
                Some(_) => self.form_error(
                    span,
                    format!(
                        "field `{name}` is a {}, which a form cannot show yet; form fields are String, Int, Float, Bool or a type of plain variants",
                        ty.display_name()
                    ),
                ),
                None => self.form_error(
                    span,
                    format!("field `{name}` must be String, Int, Float, Bool or a type of plain variants"),
                ),
            }
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
        // `form_for(move_card, ...)`: an action, named, in the route's place.
        if let Expr::Ident { name: action, .. } = target {
            if let Some(record) = self.action_form_input(action, "form_for") {
                self.check_form_record(&record, action.span);
                self.check_initial(initial, &record, &action.name);
            }
            return Type::String;
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
        self.check_initial(initial, &expected, action);
        Type::String
    }

    /// `initial` must be exactly the input the form's target reads.
    fn check_initial(&mut self, initial: &Expr, expected: &Type, target: &str) {
        let got = self.check_expr_as(initial, Some(expected));
        if got != *expected {
            // Name records by their declared names: `MoveCard`, not `struct`.
            let types = &self.types_by_id;
            let named = |ty: &Type| ty.display_name_with(&|id| types.get(&id).map(|decl| decl.name.name.clone()));
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: named(expected),
                    got: named(&got),
                    context: format!("initial values for `{target}`"),
                },
                initial.span(),
            ));
        }
    }

    /// The action `name` refers to, declared in this module.
    fn action_decl(&self, name: &Ident) -> Option<&'a AgentDecl> {
        self.file.decls.iter().find_map(|decl| match decl {
            Decl::Agent(agent) if agent.action.is_some() && agent.name.name == name.name => Some(agent),
            _ => None,
        })
    }

    /// An action's input parameters, without its caller: their names and
    /// types, in declaration order.
    fn action_inputs(&self, agent: &AgentDecl) -> Vec<(String, Type)> {
        agent
            .params
            .iter()
            .filter_map(|param| Some((param.name.name.clone(), self.type_refs.get(&param.ty.span())?.clone())))
            .filter(|(_, ty)| !is_caller(ty))
            .collect()
    }

    /// The record a form for `action` edits: the action's whole input,
    /// which must be one record. Reports why when it is not.
    fn action_form_input(&mut self, action: &Ident, builtin: &str) -> Option<Type> {
        let Some(agent) = self.action_decl(action) else {
            self.form_error(
                action.span,
                format!("`{}` is not an action declared in this module; `{builtin}` takes an action, or a route literal such as `POST /save`", action.name),
            );
            return None;
        };
        let inputs = self.action_inputs(agent);
        match whole_input_type(inputs.iter().map(|(_, ty)| ty)) {
            Some(record) => Some(record.clone()),
            None => {
                self.form_error(
                    action.span,
                    format!(
                        "`form_for` needs an action whose input is one record; `{}` takes {}. Use `button_for({}, value, label)` to call it with one value",
                        action.name,
                        describe_inputs(&inputs),
                        action.name
                    ),
                );
                None
            }
        }
    }

    /// `button_for(action, value, label)` (B10-2b-5): one button that calls
    /// `action` with `value` as its whole input, either the action's one
    /// input record or its one input of a form type.
    pub(super) fn check_button_for(&mut self, name: &Ident, args: &[Expr]) -> Type {
        let [target, value, label] = args else {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch { callee: name.name.clone(), expected: 3, got: args.len() },
                name.span,
            ));
            return Type::String;
        };
        if self.check_expr(label) != Type::String {
            self.form_error(label.span(), "the button label must be a String".into());
        }
        let Expr::Ident { name: action, .. } = target else {
            self.check_expr(target);
            self.form_error(target.span(), "`button_for` takes an action by name, such as `button_for(archive_card, card.id, \"Archive\")`".into());
            return Type::String;
        };
        let Some(agent) = self.action_decl(action) else {
            self.form_error(action.span, format!("`{}` is not an action declared in this module", action.name));
            return Type::String;
        };
        let inputs = self.action_inputs(agent);
        let expected = match (whole_input_type(inputs.iter().map(|(_, ty)| ty)), inputs.as_slice()) {
            (Some(record), _) => {
                let record = record.clone();
                self.check_form_record(&record, action.span);
                record
            }
            (None, [(_, ty)]) if is_form_scalar(ty) => ty.clone(),
            _ => {
                self.form_error(
                    action.span,
                    format!(
                        "`button_for` calls an action with one value; `{}` takes {}. Use `form_for` for an action whose input is a record of several fields",
                        action.name,
                        describe_inputs(&inputs)
                    ),
                );
                return Type::String;
            }
        };
        self.check_initial(value, &expected, &action.name);
        Type::String
    }
}

/// A type a form field can hold.
fn is_form_scalar(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float | Type::String | Type::Bool)
}

fn describe_inputs(inputs: &[(String, Type)]) -> String {
    match inputs {
        [] => "no input".to_string(),
        [(name, ty)] => format!("one input, `{name}: {}`", ty.display_name()),
        many => format!(
            "{} inputs ({})",
            many.len(),
            many.iter().map(|(name, ty)| format!("`{name}: {}`", ty.display_name())).collect::<Vec<_>>().join(", ")
        ),
    }
}
