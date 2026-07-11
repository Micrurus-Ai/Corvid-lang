//! Lambda expression checking (slice 45j).
//!
//! `fn (x) -> expr` is checked CONTEXTUALLY: when the expected type
//! at the use site is a function type (a function-type annotation,
//! a builtin-method parameter like `map`'s, or a callable local's
//! parameter), unannotated lambda parameters take their types from
//! it and the body is checked against its return type. Explicit
//! annotations win over context. With no context at all,
//! unannotated parameters are `Unknown` (lenient) — annotate to opt
//! into strict checking.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{Effect, Expr, LambdaParam, Span};
use corvid_resolve::Binding;

impl<'a> Checker<'a> {
    pub(super) fn check_lambda(
        &mut self,
        params: &[LambdaParam],
        body: &Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let expected_fn = match expected {
            Some(Type::Function { params, ret, .. }) => Some((params.clone(), (**ret).clone())),
            _ => None,
        };

        if let Some((eps, _)) = &expected_fn {
            if eps.len() != params.len() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::ArityMismatch {
                        callee: "lambda".into(),
                        expected: eps.len(),
                        got: params.len(),
                    },
                    span,
                ));
            }
        }

        let mut param_tys = Vec::with_capacity(params.len());
        for (i, p) in params.iter().enumerate() {
            let ty = match &p.ty {
                Some(tr) => self.type_ref_to_type(tr),
                None => expected_fn
                    .as_ref()
                    .and_then(|(eps, _)| eps.get(i).cloned())
                    .unwrap_or(Type::Unknown),
            };
            if let Some(Binding::Local(lid)) = self.bindings.get(&p.name.span).cloned() {
                self.local_types.insert(lid, ty.clone());
            }
            param_tys.push(ty);
        }

        let ret_ty = match &expected_fn {
            Some((_, expected_ret)) if !matches!(expected_ret, Type::Unknown) => {
                let body_ty = self.check_expr_as(body, Some(expected_ret));
                if !body_ty.is_assignable_to(expected_ret) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: expected_ret.display_name(),
                            got: body_ty.display_name(),
                            context: "lambda body".into(),
                        },
                        body.span(),
                    ));
                }
                expected_ret.clone()
            }
            _ => self.check_expr(body),
        };

        Type::Function {
            params: param_tys,
            ret: Box::new(ret_ty),
            effect: Effect::Safe,
        }
    }
}
