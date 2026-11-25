//! Typed JSON method checks (B01): `T.from_json(text)`,
//! `T.from_json_strict(text)` and `value.to_json()`.
//!
//! The method table (`builtin_methods`) names the methods and their
//! signatures. This file decides what the table cannot see: whether a
//! type, including every field of its records and variants, has a
//! public JSON form in the requested direction, and whether encoding
//! it must keep a `Tainted` marking on the resulting text. It also
//! refuses tools declared for the retired `decode_<Type>_from_json`
//! convention, which these methods replace.

use super::Checker;
use crate::builtin_methods::{type_method, BuiltinMethodSig};
use crate::errors::{TypeError, TypeErrorKind};
use crate::type_receiver::{type_receiver, TypeReceiver};
use crate::types::Type;
use corvid_ast::{Expr, Field, Ident, Span, TypeDecl};
use corvid_resolve::{DefId, ResolvedModule};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Decode,
    Encode,
}

/// Why a type has no JSON form, with what to write instead.
struct NoForm {
    reason: String,
    hint: String,
}

impl NoForm {
    fn new(reason: impl Into<String>, hint: impl Into<String>) -> Self {
        Self { reason: reason.into(), hint: hint.into() }
    }
}

impl<'a> Checker<'a> {
    /// `Order.from_json(text)`: when the receiver names a type rather
    /// than a value, check the call as a type method. Returns `None`
    /// when `target` is not a type name or `method` is not a type
    /// method, so ordinary method dispatch continues.
    pub(super) fn check_type_method_call(
        &mut self,
        target: &Expr,
        method: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Option<Type> {
        let receiver = type_receiver(target, self.bindings, self.symbols, self.module_resolution)?;
        // Only a type method makes the receiver a type name; otherwise
        // ordinary dispatch reports the misuse.
        type_method(&Type::Unknown, &method.name)?;
        let type_name = receiver_display_name(target);
        let target_ty = match receiver {
            TypeReceiver::Local(def_id) => self.expand_possible_alias(def_id, target.span()),
            TypeReceiver::Imported { module, def_id } => {
                self.imported_named_type(module, def_id, &type_name_of(module, def_id))
            }
        };
        let BuiltinMethodSig { params, ret, .. } = type_method(&target_ty, &method.name)?;

        if args.len() != params.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: method.name.clone(),
                    expected: params.len(),
                    got: args.len(),
                },
                span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Some(ret);
        }
        let got = self.check_expr_as(&args[0], Some(&params[0]));
        if !got.is_assignable_to(&params[0]) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: params[0].display_name(),
                    got: got.display_name(),
                    context: format!("argument to `{}`", method.name),
                },
                args[0].span(),
            ));
        }
        if let Err(no_form) = self.json_form(&target_ty, Direction::Decode, &mut Vec::new()) {
            self.errors.push(TypeError::new(
                TypeErrorKind::NoJsonForm {
                    method: method.name.clone(),
                    ty: type_name,
                    reason: no_form.reason,
                    hint: no_form.hint,
                },
                span,
            ));
        }
        Some(ret)
    }

    /// The result type of `value.to_json()` on a receiver of type
    /// `receiver`: `String`, or `Tainted<String>` when the receiver
    /// carries tainted data anywhere inside it. Reports receivers with
    /// no JSON form.
    pub(super) fn to_json_result(&mut self, receiver: &Type, span: Span) -> Type {
        match self.json_form(receiver, Direction::Encode, &mut Vec::new()) {
            Ok(true) => Type::Tainted(Box::new(Type::String)),
            Ok(false) => Type::String,
            Err(no_form) => {
                let ty = self.json_type_name(receiver);
                self.errors.push(TypeError::new(
                    TypeErrorKind::NoJsonForm {
                        method: "to_json".into(),
                        ty,
                        reason: no_form.reason,
                        hint: no_form.hint,
                    },
                    span,
                ));
                Type::String
            }
        }
    }

    /// Refuses a tool declared in the shape of the retired
    /// `decode_<Type>_from_json` convention: a name of that form and a
    /// `Result<T, String>` return. The runtime used to decode JSON for
    /// such a tool in place of calling it; it no longer does, so the
    /// declaration would compile into a tool with no implementation.
    /// The condition is the one the runtime matched, so a tool that was
    /// never decoded by the runtime is unaffected.
    pub(super) fn check_retired_json_decoder_tool(&mut self, tool: &corvid_ast::ToolDecl) {
        let name = tool.name.name.as_str();
        let Some(middle) = name.strip_prefix("decode_").and_then(|rest| rest.strip_suffix("_from_json")) else {
            return;
        };
        if middle.is_empty() {
            return;
        }
        // The signature's own check already reported a bad return
        // type; resolving it again must not repeat that.
        let mark = self.errors.len();
        let return_ty = self.type_ref_to_type(&tool.return_ty);
        self.errors.truncate(mark);
        let Type::Result(target, err) = return_ty else { return };
        if !matches!(*err, Type::String) {
            return;
        }
        let replacement = match target.as_ref() {
            Type::Struct(_) | Type::ImportedStruct(_) => {
                let target = self.json_type_name(&target);
                format!("delete the tool and call `{target}.from_json(text)`, which returns `Result<{target}, JsonError>`")
            }
            other => format!(
                "delete the tool, name the target with `type Decoded = {}`, and call `Decoded.from_json(text)`",
                self.json_type_name(other)
            ),
        };
        self.errors.push(TypeError::new(
            TypeErrorKind::RetiredJsonDecoderTool { tool: name.to_string(), replacement },
            tool.name.span,
        ));
    }

    /// Why `ty` cannot be sent (`encode`) or read as JSON, with what to
    /// write instead; `None` when it can. For connector bodies and
    /// responses, which travel as the same public JSON as `to_json`.
    pub(super) fn json_wire_problem(&mut self, ty: &Type, encode: bool) -> Option<String> {
        let direction = if encode { Direction::Encode } else { Direction::Decode };
        self.json_form(ty, direction, &mut Vec::new())
            .err()
            .map(|no| format!("{}; {}", no.reason, no.hint))
    }

    /// Whether `ty` has a JSON form in `direction`. `Ok(tainted)`
    /// reports whether any part of it is `Tainted`. `seen` stops
    /// recursion through self-referential records.
    fn json_form(&mut self, ty: &Type, direction: Direction, seen: &mut Vec<(String, DefId)>) -> Result<bool, NoForm> {
        match ty {
            Type::Int
            | Type::Float
            | Type::String
            | Type::Bool
            | Type::Nothing
            | Type::JsonValue
            | Type::Unknown => Ok(false),
            Type::Tainted(inner) => self.json_form(inner, direction, seen).map(|_| true),
            Type::Option(inner) | Type::List(inner) => self.json_form(inner, direction, seen),
            Type::Map(a, b) | Type::Result(a, b) => {
                let first = self.json_form(a, direction, seen)?;
                Ok(self.json_form(b, direction, seen)? || first)
            }
            Type::Grounded(inner) => match direction {
                // A reply carries the value and its sources.
                Direction::Encode => self.json_form(inner, direction, seen),
                Direction::Decode => Err(NoForm::new(
                    "JSON text cannot prove where a `Grounded` value came from",
                    "decode the plain value, then ground it through a retrieval tool",
                )),
            },
            Type::JsonError => match direction {
                Direction::Encode => Ok(false),
                Direction::Decode => Err(NoForm::new(
                    "a `JsonError` is produced by `from_json`, not read from JSON",
                    "decode the error's fields into a record you declare",
                )),
            },
            // B06 — each crosses JSON as its exact canonical text (see
            // `corvid_public_json::text_scalar`), and `Bytes` as base64url
            // text. A table stores each in a column of its own (B09 P8a
            // and P8c), and inside a JSON-stored column as that text.
            Type::Decimal
            | Type::Duration
            | Type::Instant
            | Type::TimeZone
            | Type::ZonedTime
            | Type::Currency
            | Type::Locale
            | Type::Bytes => {
                Ok(false)
            }
            // A secret's text never leaves the program, so it has no
            // JSON form in either direction, and a record holding one has
            // none either.
            Type::Secret => Err(NoForm::new(
                "a `Secret` has no JSON form, because its text must never leave the program",
                "keep the secret out of the encoded value and pass it straight to the tool that needs it",
            )),
            Type::RegexMatch => Err(NoForm::new(
                "a `RegexMatch` describes a position in one text and has no JSON form",
                "copy the fields you need into a record you declare",
            )),
            Type::Page(inner) => match direction {
                Direction::Encode => self.json_form(inner, direction, seen),
                Direction::Decode => Err(NoForm::new(
                    "a `Page` is produced by a paginated route, not read from JSON",
                    "decode the items into a record with a `List` field",
                )),
            },
            Type::Struct(id) => {
                let key = (String::new(), *id);
                if seen.contains(&key) {
                    return Ok(false);
                }
                let Some(decl) = self.types_by_id.get(id).copied() else {
                    return Ok(false);
                };
                seen.push(key);
                let result = self.declared_json_form(decl, direction, seen, None);
                seen.pop();
                result
            }
            Type::ImportedStruct(imported) => {
                // Ids are per module, so the module path is part of the key.
                let key = (imported.module_path.clone(), imported.def_id);
                if seen.contains(&key) {
                    return Ok(false);
                }
                let Some(module) = self
                    .module_resolution
                    .and_then(|modules| modules.lookup_by_path(Path::new(&imported.module_path)))
                else {
                    return Ok(false);
                };
                let Some(decl) = imported_type_decl(module, imported.def_id) else {
                    return Ok(false);
                };
                seen.push(key);
                let result = self.declared_json_form(decl, direction, seen, Some(module));
                seen.pop();
                result
            }
            Type::Function { .. } => Err(NoForm::new("a function has no JSON form", "encode the data the function works on")),
            Type::Stream(_) => Err(NoForm::new(
                "a stream has no single JSON form",
                "collect the stream into a `List` first",
            )),
            Type::Weak(..) => Err(NoForm::new("a weak reference has no JSON form", "upgrade it to the value first")),
            Type::Partial(_) => Err(NoForm::new(
                "a partial value is still streaming",
                "wait for the complete value",
            )),
            Type::ResumeToken(_) => Err(NoForm::new("a resume token has no JSON form", "keep it as a value")),
            Type::Upload(_) => Err(NoForm::new(
                "an upload has no JSON form",
                "read it with `text()` or `bytes()` first",
            )),
            Type::Response(_) => Err(NoForm::new(
                "a connector response has no JSON form",
                "encode its `body()`, or the `status()` and headers you need",
            )),
            Type::DbHandle => Err(NoForm::new("a database handle has no JSON form", "encode the rows you read")),
            Type::JsonBuilder => Err(NoForm::new(
                "a JSON builder has no JSON form",
                "call `json_object_finish` to get its text",
            )),
            Type::TraceId => Err(NoForm::new("a trace identifier has no JSON form", "encode the trace path as a String")),
            Type::RouteParams(_) => Err(NoForm::new(
                "route path parameters have no JSON form",
                "copy them into a record you declare",
            )),
        }
    }

    /// The JSON form of a declared record or sum type, walking every
    /// field and variant payload. `module` is set for a type declared
    /// in an imported module.
    fn declared_json_form(
        &mut self,
        decl: &TypeDecl,
        direction: Direction,
        seen: &mut Vec<(String, DefId)>,
        module: Option<&corvid_resolve::ResolvedModule>,
    ) -> Result<bool, NoForm> {
        let fields: Vec<&Field> = decl
            .fields
            .iter()
            .chain(decl.variants.iter().flat_map(|variant| variant.fields.iter()))
            .collect();
        let mut tainted = false;
        for field in fields {
            // The declaration's own check already reported any bad
            // field type; resolving it again must not repeat that.
            let mark = self.errors.len();
            let field_ty = match module {
                Some(module) => self.imported_type_ref_to_type(&field.ty, module),
                None => self.type_ref_to_type(&field.ty),
            };
            self.errors.truncate(mark);
            match self.json_form(&field_ty, direction, seen) {
                Ok(field_tainted) => tainted |= field_tainted,
                Err(no_form) => {
                    return Err(NoForm {
                        reason: format!("field `{}.{}`: {}", decl.name.name, field.name.name, no_form.reason),
                        ..no_form
                    })
                }
            }
        }
        Ok(tainted)
    }

}

/// The declaration of type `def_id` in an imported `module`.
pub(super) fn imported_type_decl(module: &ResolvedModule, def_id: DefId) -> Option<&TypeDecl> {
    module.file.decls.iter().find_map(|decl| match decl {
        corvid_ast::Decl::Type(t) if module.resolved.symbols.lookup_def(&t.name.name) == Some(def_id) => Some(t),
        _ => None,
    })
}

/// The declared name of type `def_id` in an imported `module`.
fn type_name_of(module: &ResolvedModule, def_id: DefId) -> String {
    module.resolved.symbols.get(def_id).name.clone()
}

/// The receiver as the program wrote it, for diagnostics: `Order`,
/// `Customer` or `m.Customer`.
fn receiver_display_name(target: &Expr) -> String {
    match target {
        Expr::Ident { name, .. } => name.name.clone(),
        Expr::FieldAccess { target, field, .. } => format!("{}.{}", receiver_display_name(target), field.name),
        _ => "<expression>".into(),
    }
}

impl<'a> Checker<'a> {
    /// A type name for diagnostics that names records by their
    /// declaration rather than `struct`. Used for JSON and connector
    /// diagnostics.
    pub(super) fn json_type_name(&self, ty: &Type) -> String {
        match ty {
            Type::Struct(id) => self
                .types_by_id
                .get(id)
                .map(|decl| decl.name.name.clone())
                .unwrap_or_else(|| ty.display_name()),
            Type::List(inner) => format!("List<{}>", self.json_type_name(inner)),
            Type::Option(inner) => format!("Option<{}>", self.json_type_name(inner)),
            other => other.display_name(),
        }
    }
}
