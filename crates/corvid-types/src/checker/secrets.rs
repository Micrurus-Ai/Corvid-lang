//! Compile-time rules that keep a `Secret`'s text inside the program.
//!
//! A `Secret` can be compared with another secret and passed to a tool.
//! Every other way its text could leave the program is a compile error
//! that says why and what to write instead:
//!
//! - **Operators.** `==` and `!=` between two secrets, which are
//!   constant-time at run time, are the only ones. Ordering would stop
//!   at the first differing character; `+` and any mix with `String`
//!   would turn the text into ordinary text.
//! - **Methods.** None returns the text: no `to_string`, no `to_json`.
//!   The one method, `decode_base64_after`, returns another secret.
//! - **Boundaries.** A route's inputs and response, a store field, a
//!   table column, a prompt's parameters and result, a connector
//!   operation, `ask` and `choose`, and the `main` agent's signature
//!   cannot hold a secret, directly or in a field of a record they
//!   hold. Each would hand the text to a client, a database, a model, a
//!   person or the terminal.
//!
//! `to_json` and `from_json` refuse a secret through the JSON-form walk
//! in `json_methods.rs`. The application contract leaves out every
//! position this file's walk finds a secret in (see
//! [`crate::Checked::secret_type_refs`]).

use super::json_methods::imported_type_decl;
use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{BinaryOp, Decl, Expr, ExtendMethodKind, Field, File, Ident, Param, Span, TypeDecl, TypeRef};
use corvid_resolve::DefId;
use std::path::Path;

/// A place a secret's text would leave the program, with what to do instead.
#[derive(Clone, Copy)]
enum Boundary {
    RouteInput,
    RouteResponse,
    StoreField,
    PromptParameter,
    PromptResult,
    ConnectorOperation,
    MainAgent,
}

impl Boundary {
    fn describe(self) -> &'static str {
        match self {
            Boundary::RouteInput => "a route's path, query or body",
            Boundary::RouteResponse => "a route's response",
            Boundary::StoreField => "a store field",
            Boundary::PromptParameter => "a prompt parameter",
            Boundary::PromptResult => "a prompt's result",
            Boundary::ConnectorOperation => "a connector operation's signature",
            Boundary::MainAgent => "the `main` agent's signature",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Boundary::RouteInput | Boundary::RouteResponse => {
                "a route's inputs and response are JSON a client sees; read the secret on the server with `secret(name)` from std/secrets and send only what it proves, such as a `Bool`"
            }
            Boundary::StoreField => {
                "a store keeps its fields as JSON; keep the secret in the environment and read it with `secret(name)` from std/secrets when it is needed"
            }
            Boundary::PromptParameter => {
                "no secret may reach a model; call the tool that needs the secret yourself and give the prompt only its result"
            }
            Boundary::PromptResult => "a model cannot produce a secret; return the text the model writes as a `String`",
            Boundary::ConnectorOperation => {
                "a connector's credential is declared once on the connector, as `auth`, and never travels in its operations; pass a secret to a `tool` instead"
            }
            Boundary::MainAgent => {
                "`main` is called from the command line, which prints its result and passes its arguments as text; read the secret inside with `secret(name)` and return what it proves, such as a `Bool`"
            }
        }
    }
}

impl<'a> Checker<'a> {
    /// The type of `l op r` when either side is a `Secret`, or `None`
    /// to leave the operator to the ordinary rules. `lt` and `rt` have
    /// `Grounded` and `Tainted` stripped.
    pub(super) fn check_secret_binop(&mut self, op: BinaryOp, l: &Expr, r: &Expr, lt: &Type, rt: &Type) -> Option<Type> {
        if !matches!(lt, Type::Secret) && !matches!(rt, Type::Secret) {
            return None;
        }
        use BinaryOp::*;
        let is_comparison = matches!(op, Eq | NotEq | Lt | LtEq | Gt | GtEq);
        let recovered = if is_comparison { Type::Bool } else { Type::Unknown };
        let hint = match (op, lt, rt) {
            (_, Type::Unknown, _) | (_, _, Type::Unknown) => return Some(recovered),
            (Eq | NotEq, Type::Secret, Type::Secret) => return Some(Type::Bool),
            (Lt | LtEq | Gt | GtEq, Type::Secret, Type::Secret) => {
                "secrets have no order: an ordered comparison stops at the first differing character, which leaks it; compare with `==`, which is constant-time"
            }
            (_, Type::String, _) | (_, _, Type::String) => {
                "a secret and text do not mix, because the result would be ordinary text holding the secret; to check a supplied value, read both as secrets and compare them with `==`"
            }
            _ => "a `Secret` supports only `==` and `!=` with another `Secret`, which are constant-time",
        };
        self.errors.push(TypeError::new(
            TypeErrorKind::SecretOperatorMismatch {
                left: lt.display_name(),
                right: rt.display_name(),
                hint: hint.to_string(),
            },
            l.span().merge(r.span()),
        ));
        Some(recovered)
    }

    /// Refuses `secret.method(...)` unless it is a cryptography method
    /// of the builtin-method table. The only one, `decode_base64_after`,
    /// returns another secret, so no method can return the text; the
    /// methods every type has, such as `to_json`, are refused here too.
    /// Returns the recovered result type when the call is refused, or
    /// `None` to continue ordinary dispatch.
    pub(super) fn refuse_secret_method(&mut self, receiver: &Type, method: &Ident, args: &[Expr]) -> Option<Type> {
        if !matches!(receiver.ungrounded().untainted(), Type::Secret) {
            return None;
        }
        if crate::builtin_methods::builtin_method(receiver, &method.name).is_some_and(|sig| sig.kind.is_crypto()) {
            return None;
        }
        self.errors.push(TypeError::new(
            TypeErrorKind::SecretMethod { method: method.name.clone() },
            method.span,
        ));
        for arg in args {
            let _ = self.check_expr(arg);
        }
        Some(Type::Unknown)
    }

    /// Refuses a secret in what `ask` returns (a person's answer is
    /// recorded) or `choose` shows (the options are shown to a person).
    pub(super) fn refuse_secret_for_person(&mut self, builtin: &str, ty: &Type, span: Span) {
        if let Some(at) = self.secret_reached(ty) {
            self.errors.push(TypeError::new(
                TypeErrorKind::SecretShownToPerson { builtin: builtin.to_string(), at },
                span,
            ));
        }
    }

    /// Runs after `record_signature_types`, so every type is resolved.
    /// Refuses a secret at each boundary and records, for the
    /// application contract, every type reference that holds one.
    pub(super) fn check_secret_boundaries(&mut self, file: &File) {
        for decl in &file.decls {
            match decl {
                Decl::Server(server) => {
                    for route in &server.routes {
                        let inputs = route
                            .path_params
                            .iter()
                            .map(|param| &param.ty)
                            .chain(route.query_ty.iter())
                            .chain(route.body_ty.iter());
                        for ty in inputs {
                            self.refuse_at(ty, Boundary::RouteInput);
                        }
                        self.refuse_at(&route.response.ty, Boundary::RouteResponse);
                    }
                }
                Decl::Store(store) => {
                    for field in &store.fields {
                        self.refuse_at(&field.ty, Boundary::StoreField);
                    }
                }
                Decl::Table(table) => self.refuse_secret_columns(&table.record.fields),
                Decl::Prompt(prompt) => self.refuse_in_prompt(&prompt.params, &prompt.return_ty),
                Decl::Connector(connector) => {
                    for op in &connector.operations {
                        self.refuse_in_signature(&op.params, &op.return_ty, Boundary::ConnectorOperation);
                    }
                }
                Decl::Agent(agent) if agent.name.name == "main" => {
                    self.refuse_in_signature(&agent.params, &agent.return_ty, Boundary::MainAgent);
                }
                Decl::Extend(ext) => {
                    for method in &ext.methods {
                        if let ExtendMethodKind::Prompt(prompt) = &method.kind {
                            self.refuse_in_prompt(&prompt.params, &prompt.return_ty);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut refs: Vec<(Span, Type)> = self.type_refs.iter().map(|(span, ty)| (*span, ty.clone())).collect();
        refs.sort_by_key(|(span, _)| (span.start, span.end));
        for (span, ty) in refs {
            if self.secret_reached(&ty).is_some() {
                self.secret_type_refs.insert(span);
            }
        }
    }

    fn refuse_in_prompt(&mut self, params: &[Param], return_ty: &TypeRef) {
        for param in params {
            self.refuse_at(&param.ty, Boundary::PromptParameter);
        }
        self.refuse_at(return_ty, Boundary::PromptResult);
    }

    fn refuse_in_signature(&mut self, params: &[Param], return_ty: &TypeRef, boundary: Boundary) {
        for param in params {
            self.refuse_at(&param.ty, boundary);
        }
        self.refuse_at(return_ty, boundary);
    }

    fn refuse_at(&mut self, ty: &TypeRef, boundary: Boundary) {
        let Some(resolved) = self.type_refs.get(&ty.span()).cloned() else { return };
        if let Some(at) = self.secret_reached(&resolved) {
            self.errors.push(TypeError::new(
                TypeErrorKind::SecretAtBoundary {
                    boundary: boundary.describe().to_string(),
                    at,
                    hint: boundary.hint().to_string(),
                },
                ty.span(),
            ));
        }
    }

    fn refuse_secret_columns(&mut self, fields: &[Field]) {
        for field in fields {
            let Some(resolved) = self.type_refs.get(&field.ty.span()).cloned() else { continue };
            if let Some(at) = self.secret_reached(&resolved) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::SecretTableColumn { column: field.name.name.clone(), at },
                    field.ty.span(),
                ));
            }
        }
    }

    /// Where `ty` holds a `Secret`: `Some("")` when it is or names one
    /// among its type arguments, `Some(" in field `T.f`")` when a
    /// declared record or variant holds one, `None` when it holds none.
    pub(super) fn secret_reached(&mut self, ty: &Type) -> Option<String> {
        self.secret_reached_in(ty, &mut Vec::new())
    }

    fn secret_reached_in(&mut self, ty: &Type, seen: &mut Vec<(String, DefId)>) -> Option<String> {
        match ty {
            Type::Secret => Some(String::new()),
            Type::Struct(id) => {
                let key = (String::new(), *id);
                if seen.contains(&key) {
                    return None;
                }
                let decl = self.types_by_id.get(id).copied()?;
                seen.push(key);
                let found = self.secret_in_declaration(decl, None, seen);
                seen.pop();
                found
            }
            Type::ImportedStruct(imported) => {
                let key = (imported.module_path.clone(), imported.def_id);
                if seen.contains(&key) {
                    return None;
                }
                let module = self
                    .module_resolution
                    .and_then(|modules| modules.lookup_by_path(Path::new(&imported.module_path)))?;
                let decl = imported_type_decl(module, imported.def_id)?;
                seen.push(key);
                let found = self.secret_in_declaration(decl, Some(module), seen);
                seen.pop();
                found
            }
            Type::List(inner)
            | Type::Option(inner)
            | Type::Tainted(inner)
            | Type::Grounded(inner)
            | Type::Stream(inner)
            | Type::Partial(inner)
            | Type::ResumeToken(inner)
            | Type::Upload(inner)
            | Type::Page(inner)
            | Type::Weak(inner, _) => self.secret_reached_in(inner, seen),
            Type::Map(a, b) | Type::Result(a, b) => {
                let first = self.secret_reached_in(a, seen);
                first.or_else(|| self.secret_reached_in(b, seen))
            }
            Type::Function { params, ret, .. } => {
                for param in params {
                    if let Some(at) = self.secret_reached_in(param, seen) {
                        return Some(at);
                    }
                }
                self.secret_reached_in(ret, seen)
            }
            Type::RouteParams(params) => {
                for (_, param) in params {
                    if let Some(at) = self.secret_reached_in(param, seen) {
                        return Some(at);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// The first field of a declared record or sum type that holds a
    /// `Secret`, as ` in field `T.f``.
    fn secret_in_declaration(
        &mut self,
        decl: &TypeDecl,
        module: Option<&corvid_resolve::ResolvedModule>,
        seen: &mut Vec<(String, DefId)>,
    ) -> Option<String> {
        let fields: Vec<&Field> =
            decl.fields.iter().chain(decl.variants.iter().flat_map(|variant| variant.fields.iter())).collect();
        for field in fields {
            // The declaration's own check already reported any bad
            // field type; resolving it again must not repeat that.
            let mark = self.errors.len();
            let field_ty = match module {
                Some(module) => self.imported_type_ref_to_type(&field.ty, module),
                None => self.type_ref_to_type(&field.ty),
            };
            self.errors.truncate(mark);
            if let Some(through) = self.secret_reached_in(&field_ty, seen) {
                return Some(format!(" in field `{}.{}`{through}", decl.name.name, field.name.name));
            }
        }
        None
    }
}
