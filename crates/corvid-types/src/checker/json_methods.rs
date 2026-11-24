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
use crate::types::Type;
use corvid_ast::{Expr, Field, Ident, Span, TypeDecl};
use corvid_resolve::{Binding, DeclKind, DefId};
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
        let Expr::Ident { name, .. } = target else { return None };
        let Some(Binding::Decl(def_id)) = self.bindings.get(&name.span).cloned() else {
            return None;
        };
        if self.symbols.get(def_id).kind != DeclKind::Type {
            return None;
        }
        let target_ty = self.expand_possible_alias(def_id, name.span);
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
                    ty: name.name.clone(),
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

    /// Whether `ty` has a JSON form in `direction`. `Ok(tainted)`
    /// reports whether any part of it is `Tainted`. `seen` stops
    /// recursion through self-referential records.
    fn json_form(&mut self, ty: &Type, direction: Direction, seen: &mut Vec<DefId>) -> Result<bool, NoForm> {
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
            Type::Page(inner) => match direction {
                Direction::Encode => self.json_form(inner, direction, seen),
                Direction::Decode => Err(NoForm::new(
                    "a `Page` is produced by a paginated route, not read from JSON",
                    "decode the items into a record with a `List` field",
                )),
            },
            Type::Struct(id) => {
                if seen.contains(id) {
                    return Ok(false);
                }
                seen.push(*id);
                let Some(decl) = self.types_by_id.get(id).copied() else {
                    return Ok(false);
                };
                let result = self.declared_json_form(decl, direction, seen, None);
                seen.pop();
                result
            }
            Type::ImportedStruct(imported) => {
                if direction == Direction::Decode {
                    return Err(NoForm::new(
                        format!("`{}` is declared in another module, and decoding imported types is not supported yet", imported.name),
                        "declare the record in this module",
                    ));
                }
                let Some(module) = self
                    .module_resolution
                    .and_then(|modules| modules.lookup_by_path(Path::new(&imported.module_path)))
                else {
                    return Ok(false);
                };
                let Some(decl) = module.file.decls.iter().find_map(|decl| match decl {
                    corvid_ast::Decl::Type(t)
                        if module.resolved.symbols.lookup_def(&t.name.name) == Some(imported.def_id) =>
                    {
                        Some(t)
                    }
                    _ => None,
                }) else {
                    return Ok(false);
                };
                self.declared_json_form(decl, direction, seen, Some(module))
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
        seen: &mut Vec<DefId>,
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
                    return Err(NoForm::new(
                        format!("field `{}.{}`: {}", decl.name.name, field.name.name, no_form.reason),
                        no_form.hint,
                    ))
                }
            }
        }
        Ok(tainted)
    }

    /// A type name for JSON diagnostics that names records by their
    /// declaration rather than `struct`.
    fn json_type_name(&self, ty: &Type) -> String {
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
