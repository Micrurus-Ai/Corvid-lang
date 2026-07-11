//! `match` expression typing + exhaustiveness (slice 45i).
//!
//! Pattern typing walks each arm's pattern against the scrutinee's
//! type, binding pattern variables into the checker's local-type
//! table. Exhaustiveness is decided per scrutinee kind:
//!
//! - **Sum types**: every variant must be irrefutably covered (a
//!   `Variant(...)` pattern whose subpatterns are all irrefutable,
//!   with no guard) or a catch-all arm must exist.
//! - **Option**: `Some(<irrefutable>)` + `None` (or catch-all).
//! - **Result**: `Ok(<irrefutable>)` + `Err(<irrefutable>)`.
//! - **Bool**: `true` + `false` literals.
//! - **Everything else** (Int, String, structs, …) requires a
//!   catch-all arm — literals can never enumerate these types.
//!
//! A guard makes an arm refutable: guarded arms never count toward
//! exhaustiveness.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{Expr, MatchArm, Pattern, Span};
use corvid_resolve::Binding;
use std::collections::BTreeSet;

impl<'a> Checker<'a> {
    pub(super) fn check_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let scrut_ty = self.check_expr(scrutinee);

        let mut result_ty: Option<Type> = None;
        for arm in arms {
            self.check_pattern(&arm.pattern, &scrut_ty);
            if let Some(guard) = &arm.guard {
                let g_ty = self.check_expr(guard);
                if !matches!(g_ty, Type::Bool | Type::Unknown) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Bool".into(),
                            got: g_ty.display_name(),
                            context: "match arm guard".into(),
                        },
                        guard.span(),
                    ));
                }
            }
            let body_ty = self.check_expr_as(&arm.body, expected.or(result_ty.as_ref()));
            result_ty = Some(match result_ty {
                None => body_ty,
                Some(prev) => {
                    if body_ty.is_assignable_to(&prev) {
                        prev
                    } else if prev.is_assignable_to(&body_ty) {
                        // Int -> Float style widening across arms.
                        body_ty
                    } else {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::TypeMismatch {
                                expected: prev.display_name(),
                                got: body_ty.display_name(),
                                context: "match arm result".into(),
                            },
                            arm.body.span(),
                        ));
                        prev
                    }
                }
            });
        }

        self.check_exhaustiveness(&scrut_ty, arms, span);
        result_ty.unwrap_or(Type::Unknown)
    }

    /// Type one pattern against the scrutinee type, binding pattern
    /// variables.
    pub(super) fn check_pattern(&mut self, pattern: &Pattern, scrut_ty: &Type) {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Literal { value, span } => {
                let lit_ty = literal_type(value);
                if !lit_ty.is_assignable_to(scrut_ty) && !scrut_ty.is_assignable_to(&lit_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: scrut_ty.display_name(),
                            got: lit_ty.display_name(),
                            context: "literal pattern".into(),
                        },
                        *span,
                    ));
                }
            }
            Pattern::Name { name, .. } => {
                match self.bindings.get(&name.span) {
                    // Unit-variant pattern: verify it belongs to the
                    // scrutinee's sum type.
                    Some(Binding::Decl(def_id)) => {
                        self.check_variant_pattern_owner(*def_id, scrut_ty, name.span);
                    }
                    // `None` pattern over Option.
                    Some(Binding::BuiltIn(_)) => {
                        if !matches!(scrut_ty, Type::Option(_) | Type::Unknown) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::TypeMismatch {
                                    expected: scrut_ty.display_name(),
                                    got: "None".into(),
                                    context: "pattern".into(),
                                },
                                name.span,
                            ));
                        }
                    }
                    // Binding: the name takes the scrutinee's type.
                    Some(Binding::Local(local_id)) => {
                        self.local_types.insert(*local_id, scrut_ty.clone());
                    }
                    _ => {}
                }
            }
            Pattern::At { name, inner, .. } => {
                if let Some(Binding::Local(local_id)) = self.bindings.get(&name.span) {
                    self.local_types.insert(*local_id, scrut_ty.clone());
                }
                self.check_pattern(inner, scrut_ty);
            }
            Pattern::Variant { name, args, span } => {
                match self.bindings.get(&name.span).cloned() {
                    Some(Binding::Decl(def_id)) => {
                        self.check_variant_pattern_owner(def_id, scrut_ty, name.span);
                        // Subpatterns against the variant's declared
                        // field types.
                        let field_refs: Vec<corvid_ast::TypeRef> = self
                            .variant_owners
                            .get(&def_id)
                            .copied()
                            .and_then(|(owner, idx)| {
                                self.types_by_id.get(&owner).map(|td| {
                                    td.variants
                                        .get(idx as usize)
                                        .map(|v| {
                                            v.fields
                                                .iter()
                                                .map(|f| f.ty.clone())
                                                .collect::<Vec<_>>()
                                        })
                                        .unwrap_or_default()
                                })
                            })
                            .unwrap_or_default();
                        let field_tys: Vec<Type> = field_refs
                            .iter()
                            .map(|t| self.type_ref_to_type(t))
                            .collect();
                        if args.len() != field_tys.len() {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::ArityMismatch {
                                    callee: name.name.clone(),
                                    expected: field_tys.len(),
                                    got: args.len(),
                                },
                                *span,
                            ));
                        }
                        for (arg, fty) in args.iter().zip(field_tys.iter()) {
                            self.check_pattern(arg, fty);
                        }
                    }
                    Some(Binding::BuiltIn(b)) => {
                        use corvid_resolve::BuiltIn;
                        let inner_ty = match (b, scrut_ty) {
                            (BuiltIn::Some, Type::Option(inner)) => (**inner).clone(),
                            (BuiltIn::Ok, Type::Result(ok, _)) => (**ok).clone(),
                            (BuiltIn::Err, Type::Result(_, err)) => (**err).clone(),
                            (_, Type::Unknown) => Type::Unknown,
                            _ => {
                                self.errors.push(TypeError::new(
                                    TypeErrorKind::TypeMismatch {
                                        expected: scrut_ty.display_name(),
                                        got: format!("a `{}` pattern", name.name),
                                        context: "pattern".into(),
                                    },
                                    *span,
                                ));
                                Type::Unknown
                            }
                        };
                        if args.len() != 1 {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::ArityMismatch {
                                    callee: name.name.clone(),
                                    expected: 1,
                                    got: args.len(),
                                },
                                *span,
                            ));
                        }
                        for arg in args {
                            self.check_pattern(arg, &inner_ty);
                        }
                    }
                    _ => {}
                }
            }
            Pattern::Record {
                name,
                fields,
                rest,
                span,
            } => {
                // The named type must BE the scrutinee's record type.
                let owner = match scrut_ty {
                    Type::Struct(id) => Some(*id),
                    _ => None,
                };
                let named = match self.bindings.get(&name.span) {
                    Some(Binding::Decl(id)) => Some(*id),
                    _ => None,
                };
                if let (Some(o), Some(n)) = (owner, named) {
                    if o != n {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::TypeMismatch {
                                expected: scrut_ty.display_name(),
                                got: name.name.clone(),
                                context: "record pattern".into(),
                            },
                            *span,
                        ));
                    }
                }
                let Some(ty_decl) = named.and_then(|n| self.types_by_id.get(&n).copied()) else {
                    return;
                };
                let mut seen: BTreeSet<String> = BTreeSet::new();
                for fp in fields {
                    seen.insert(fp.name.name.clone());
                    let Some(field) = ty_decl
                        .fields
                        .iter()
                        .find(|f| f.name.name == fp.name.name)
                    else {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::UnknownField {
                                struct_name: name.name.clone(),
                                field: fp.name.name.clone(),
                            },
                            fp.span,
                        ));
                        continue;
                    };
                    let fty = self.type_ref_to_type(&field.ty);
                    match &fp.pattern {
                        Some(sub) => self.check_pattern(sub, &fty),
                        None => {
                            // Shorthand binds the field name.
                            if let Some(Binding::Local(local_id)) =
                                self.bindings.get(&fp.name.span)
                            {
                                self.local_types.insert(*local_id, fty);
                            }
                        }
                    }
                }
                if !rest && seen.len() != ty_decl.fields.len() {
                    let missing: Vec<String> = ty_decl
                        .fields
                        .iter()
                        .filter(|f| !seen.contains(&f.name.name))
                        .map(|f| f.name.name.clone())
                        .collect();
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: format!(
                                "all fields of `{}` (missing: {}) or a `..` rest marker",
                                name.name,
                                missing.join(", ")
                            ),
                            got: format!("{} field pattern(s)", fields.len()),
                            context: "record pattern".into(),
                        },
                        *span,
                    ));
                }
            }
        }
    }

    fn check_variant_pattern_owner(&mut self, variant_id: corvid_resolve::DefId, scrut_ty: &Type, span: Span) {
        let Some((owner, _)) = self.variant_owners.get(&variant_id).copied() else {
            return;
        };
        match scrut_ty {
            Type::Struct(id) if *id == owner => {}
            Type::Unknown => {}
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: other.display_name(),
                        got: self.symbols.get(variant_id).name.clone(),
                        context: "variant pattern".into(),
                    },
                    span,
                ));
            }
        }
    }

    /// Decide exhaustiveness for the scrutinee type; report a single
    /// error naming what's missing.
    fn check_exhaustiveness(&mut self, scrut_ty: &Type, arms: &[MatchArm], span: Span) {
        // A catch-all arm (no guard, irrefutable pattern) covers
        // every scrutinee type.
        if arms
            .iter()
            .any(|a| a.guard.is_none() && self.pattern_is_irrefutable(&a.pattern))
        {
            return;
        }
        let missing: Option<String> = match scrut_ty {
            Type::Struct(owner) => {
                let Some(ty_decl) = self.types_by_id.get(owner).copied() else {
                    return;
                };
                if ty_decl.variants.is_empty() {
                    // Record scrutinee without catch-all.
                    Some("a catch-all arm (`_ -> ...` or a binding)".into())
                } else {
                    let covered: BTreeSet<String> = arms
                        .iter()
                        .filter(|a| a.guard.is_none())
                        .filter_map(|a| self.irrefutably_covered_variant(&a.pattern))
                        .collect();
                    let missing_variants: Vec<String> = ty_decl
                        .variants
                        .iter()
                        .map(|v| v.name.name.clone())
                        .filter(|v| !covered.contains(v))
                        .collect();
                    if missing_variants.is_empty() {
                        None
                    } else {
                        Some(format!("variant(s) `{}`", missing_variants.join("`, `")))
                    }
                }
            }
            Type::Option(_) => {
                let mut has_some = false;
                let mut has_none = false;
                for arm in arms.iter().filter(|a| a.guard.is_none()) {
                    match &arm.pattern {
                        Pattern::Variant { name, args, .. }
                            if name.name == "Some"
                                && args.iter().all(|p| self.pattern_is_irrefutable(p)) =>
                        {
                            has_some = true
                        }
                        Pattern::Name { name, .. } if name.name == "None" => has_none = true,
                        _ => {}
                    }
                }
                match (has_some, has_none) {
                    (true, true) => None,
                    (false, true) => Some("`Some(_)`".into()),
                    (true, false) => Some("`None`".into()),
                    (false, false) => Some("`Some(_)` and `None`".into()),
                }
            }
            Type::Result(_, _) => {
                let mut has_ok = false;
                let mut has_err = false;
                for arm in arms.iter().filter(|a| a.guard.is_none()) {
                    if let Pattern::Variant { name, args, .. } = &arm.pattern {
                        if args.iter().all(|p| self.pattern_is_irrefutable(p)) {
                            match name.name.as_str() {
                                "Ok" => has_ok = true,
                                "Err" => has_err = true,
                                _ => {}
                            }
                        }
                    }
                }
                match (has_ok, has_err) {
                    (true, true) => None,
                    (false, true) => Some("`Ok(_)`".into()),
                    (true, false) => Some("`Err(_)`".into()),
                    (false, false) => Some("`Ok(_)` and `Err(_)`".into()),
                }
            }
            Type::Bool => {
                let mut has_true = false;
                let mut has_false = false;
                for arm in arms.iter().filter(|a| a.guard.is_none()) {
                    if let Pattern::Literal {
                        value: corvid_ast::Literal::Bool(b),
                        ..
                    } = &arm.pattern
                    {
                        if *b {
                            has_true = true
                        } else {
                            has_false = true
                        }
                    }
                }
                match (has_true, has_false) {
                    (true, true) => None,
                    _ => Some("both `true` and `false`".into()),
                }
            }
            Type::Unknown => None,
            _ => Some("a catch-all arm (`_ -> ...` or a binding)".into()),
        };
        if let Some(missing) = missing {
            self.errors.push(TypeError::new(
                TypeErrorKind::NonExhaustiveMatch { missing },
                span,
            ));
        }
    }

    /// Does this pattern match EVERY value of its type?
    pub(super) fn pattern_is_irrefutable(&self, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Wildcard { .. } => true,
            Pattern::Name { name, .. } => {
                // A bare name is irrefutable only if it's a BINDING —
                // a unit-variant pattern is refutable.
                matches!(self.bindings.get(&name.span), Some(Binding::Local(_)))
            }
            Pattern::At { inner, .. } => self.pattern_is_irrefutable(inner),
            Pattern::Literal { .. } | Pattern::Variant { .. } => false,
            Pattern::Record { fields, .. } => fields.iter().all(|fp| match &fp.pattern {
                Some(sub) => self.pattern_is_irrefutable(sub),
                None => true,
            }),
        }
    }

    /// If this no-guard pattern irrefutably covers exactly one sum
    /// variant, return its name.
    fn irrefutably_covered_variant(&self, pattern: &Pattern) -> Option<String> {
        match pattern {
            Pattern::Name { name, .. } => {
                if let Some(Binding::Decl(id)) = self.bindings.get(&name.span) {
                    if self.variant_owners.contains_key(id) {
                        return Some(name.name.clone());
                    }
                }
                None
            }
            Pattern::Variant { name, args, .. } => {
                if args.iter().all(|p| self.pattern_is_irrefutable(p)) {
                    Some(name.name.clone())
                } else {
                    None
                }
            }
            Pattern::At { inner, .. } => self.irrefutably_covered_variant(inner),
            _ => None,
        }
    }
}

fn literal_type(lit: &corvid_ast::Literal) -> Type {
    match lit {
        corvid_ast::Literal::Int(_) => Type::Int,
        corvid_ast::Literal::Float(_) => Type::Float,
        corvid_ast::Literal::String(_) => Type::String,
        corvid_ast::Literal::Bool(_) => Type::Bool,
        corvid_ast::Literal::Nothing => Type::Nothing,
    }
}
