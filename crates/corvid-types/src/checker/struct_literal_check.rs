//! Named struct literal checking (slice 45n).
//!
//! `Decision { refund: true, amount, ..base }` — every declared
//! field must be provided exactly once, either explicitly, by
//! shorthand (`amount` reads the local of that name), or through
//! the `..base` update spread (which must be the same struct
//! type). A bare `..` (no expression) is only meaningful when the
//! statement parser reinterprets the literal as a destructuring
//! pattern; reaching the checker as an EXPRESSION is an error.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{Expr, Ident, Span, StructLiteralField};
use corvid_resolve::{Binding, DeclKind};

impl<'a> Checker<'a> {
    pub(super) fn check_struct_literal(
        &mut self,
        name: &Ident,
        fields: &[StructLiteralField],
        spread: Option<&Expr>,
        rest: bool,
        span: Span,
    ) -> Type {
        let invalid = |message: &str| TypeErrorKind::StructLiteralInvalid {
            type_name: name.name.clone(),
            message: message.into(),
        };

        if rest {
            self.errors.push(TypeError::new(
                invalid("bare `..` is only valid when destructuring — as an expression, fill the remaining fields or spread `..base`"),
                span,
            ));
        }

        // The name must resolve to a RECORD type declaration
        // (aliases expand transparently to their record target).
        let def_id = match self.bindings.get(&name.span) {
            Some(Binding::Decl(id)) => *id,
            _ => {
                self.errors.push(TypeError::new(
                    invalid("the name does not resolve to a type declaration"),
                    name.span,
                ));
                for f in fields {
                    if let Some(v) = &f.value {
                        let _ = self.check_expr(v);
                    }
                }
                if let Some(s) = spread {
                    let _ = self.check_expr(s);
                }
                return Type::Unknown;
            }
        };
        if self.symbols.get(def_id).kind != DeclKind::Type {
            self.errors.push(TypeError::new(
                invalid("the name resolves to a declaration that is not a type"),
                name.span,
            ));
            return Type::Unknown;
        }
        let def_id = match self.expand_possible_alias(def_id, name.span) {
            Type::Struct(id) => id,
            Type::Unknown => return Type::Unknown,
            other => {
                self.errors.push(TypeError::new(
                    invalid(&format!(
                        "`{}` is not a record type (it aliases `{}`)",
                        name.name,
                        other.display_name()
                    )),
                    name.span,
                ));
                return Type::Unknown;
            }
        };
        let ty_decl = *self.types_by_id.get(&def_id).expect("type DefId not indexed");
        if !ty_decl.variants.is_empty() {
            self.errors.push(TypeError::new(
                invalid("sum types are constructed with `Variant(...)`, not `{ ... }`"),
                name.span,
            ));
            return Type::Unknown;
        }

        // Check each provided field once, against its declared type.
        let decl_fields: Vec<(String, corvid_ast::TypeRef)> = ty_decl
            .fields
            .iter()
            .map(|f| (f.name.name.clone(), f.ty.clone()))
            .collect();
        let mut provided: Vec<&str> = Vec::with_capacity(fields.len());
        for f in fields {
            if provided.iter().any(|p| *p == f.name.name) {
                self.errors.push(TypeError::new(
                    invalid(&format!("field `{}` is provided twice", f.name.name)),
                    f.span,
                ));
                continue;
            }
            let Some((_, field_tr)) = decl_fields.iter().find(|(n, _)| *n == f.name.name) else {
                self.errors.push(TypeError::new(
                    invalid(&format!(
                        "`{}` has no field named `{}`",
                        name.name, f.name.name
                    )),
                    f.span,
                ));
                if let Some(v) = &f.value {
                    let _ = self.check_expr(v);
                }
                continue;
            };
            let field_ty = self.type_ref_to_type(&field_tr.clone());
            let got = match &f.value {
                Some(v) => self.check_expr_as(v, Some(&field_ty)),
                // Shorthand: the resolver bound the field name to a
                // local read.
                None => match self.bindings.get(&f.name.span) {
                    Some(Binding::Local(lid)) => self
                        .local_types
                        .get(lid)
                        .cloned()
                        .unwrap_or(Type::Unknown),
                    _ => Type::Unknown,
                },
            };
            if !got.is_assignable_to(&field_ty) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: field_ty.display_name(),
                        got: got.display_name(),
                        context: format!("field `{}` of `{}`", f.name.name, name.name),
                    },
                    f.span,
                ));
            }
            provided.push(&f.name.name);
        }

        // Coverage: spread fills whatever was not named; without a
        // spread every declared field is required.
        match spread {
            Some(s) => {
                let spread_ty = self.check_expr(s);
                let matches = matches!(&spread_ty, Type::Struct(id) if *id == def_id)
                    || matches!(spread_ty, Type::Unknown);
                if !matches {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: name.name.clone(),
                            got: spread_ty.display_name(),
                            context: format!("`..base` spread of `{}`", name.name),
                        },
                        s.span(),
                    ));
                }
            }
            None => {
                let missing: Vec<&str> = decl_fields
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .filter(|n| !provided.contains(n))
                    .collect();
                if !missing.is_empty() {
                    self.errors.push(TypeError::new(
                        invalid(&format!(
                            "missing field(s) `{}` — name them or fill from a base with `..base`",
                            missing.join("`, `")
                        )),
                        span,
                    ));
                }
            }
        }

        Type::Struct(def_id)
    }
}
