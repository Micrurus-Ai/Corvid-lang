use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{AgentDecl, OwnershipAnnotation, OwnershipMode};

impl<'a> Checker<'a> {
    pub(super) fn check_extern_c_signature(&mut self, a: &AgentDecl) {
        for param in &a.params {
            let ty = self.type_ref_to_type(&param.ty);
            if !self.extern_c_param_type_supported(&ty) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::NonScalarInExternC {
                        agent: a.name.name.clone(),
                        offender_type: ty.display_name(),
                        position: format!("parameter `{}`", param.name.name),
                    },
                    param.span,
                ));
                continue;
            }
            match infer_extern_param_ownership(&ty) {
                Ok(inferred) => {
                    if let Some(declared) = param.ownership.as_ref() {
                        if !ownership_matches(declared, &inferred) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::ExternOwnershipMismatch {
                                    agent: a.name.name.clone(),
                                    position: format!("parameter `{}`", param.name.name),
                                    declared: ownership_label_declared(declared),
                                    inferred: ownership_label_inferred(&inferred),
                                    reason: inferred.reason.clone(),
                                },
                                param.span,
                            ));
                        }
                    }
                }
                Err(reason) => {
                    if param.ownership.is_none() {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::AmbiguousExternOwnership {
                                agent: a.name.name.clone(),
                                position: format!("parameter `{}`", param.name.name),
                            },
                            param.span,
                        ));
                    } else {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::ExternOwnershipMismatch {
                                agent: a.name.name.clone(),
                                position: format!("parameter `{}`", param.name.name),
                                declared: ownership_label_declared(
                                    param.ownership.as_ref().unwrap(),
                                ),
                                inferred: "ambiguous".into(),
                                reason,
                            },
                            param.span,
                        ));
                    }
                }
            }
        }
        let ret = self.type_ref_to_type(&a.return_ty);
        if !self.extern_c_return_type_supported(&ret) {
            self.errors.push(TypeError::new(
                TypeErrorKind::NonScalarInExternC {
                    agent: a.name.name.clone(),
                    offender_type: ret.display_name(),
                    position: "return type".into(),
                },
                a.return_ty.span(),
            ));
            return;
        }
        match infer_extern_return_ownership(&ret) {
            Ok(inferred) => {
                if let Some(declared) = a.return_ownership.as_ref() {
                    if !ownership_matches(declared, &inferred) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::ExternOwnershipMismatch {
                                agent: a.name.name.clone(),
                                position: "return type".into(),
                                declared: ownership_label_declared(declared),
                                inferred: ownership_label_inferred(&inferred),
                                reason: inferred.reason.clone(),
                            },
                            a.return_ty.span(),
                        ));
                    }
                }
            }
            Err(reason) => {
                if a.return_ownership.is_none() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::AmbiguousExternOwnership {
                            agent: a.name.name.clone(),
                            position: "return type".into(),
                        },
                        a.return_ty.span(),
                    ));
                } else {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ExternOwnershipMismatch {
                            agent: a.name.name.clone(),
                            position: "return type".into(),
                            declared: ownership_label_declared(
                                a.return_ownership.as_ref().unwrap(),
                            ),
                            inferred: "ambiguous".into(),
                            reason,
                        },
                        a.return_ty.span(),
                    ));
                }
            }
        }
    }

    /// The types a `pub extern "c"` parameter takes: `Int`, `Float`,
    /// `Bool` and `String` as C scalars, and any record, variant, list,
    /// option, result or map with a JSON form, which crosses as a borrowed
    /// `const char*` holding its public JSON, decoded on entry. The JSON
    /// rule is `from_json`'s (`json_wire_problem`), so the C boundary takes
    /// exactly what a route body takes.
    pub(super) fn extern_c_param_type_supported(&mut self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::String => true,
            ty if ty.crosses_c_abi_as_json() => self.json_wire_problem(ty, false).is_none(),
            _ => false,
        }
    }

    /// The types a `pub extern "c"` agent returns: the C scalars,
    /// `Nothing`, `Grounded<scalar>` (with its attestation handle), and any
    /// structured type with a JSON form, which leaves as an owned
    /// `const char*` of its public JSON, by `to_json`'s rule.
    pub(super) fn extern_c_return_type_supported(&mut self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::String | Type::Nothing => true,
            Type::Grounded(inner) => matches!(
                &**inner,
                Type::Int | Type::Float | Type::Bool | Type::String
            ),
            ty if ty.crosses_c_abi_as_json() => self.json_wire_problem(ty, true).is_none(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
struct InferredOwnership {
    mode: OwnershipMode,
    lifetime: Option<String>,
    reason: String,
}

fn infer_extern_param_ownership(ty: &Type) -> Result<InferredOwnership, String> {
    match ty {
        Type::String | Type::TraceId => Ok(InferredOwnership {
            mode: OwnershipMode::Borrowed,
            lifetime: Some("call".to_string()),
            reason: "string-like extern parameters are passed as borrowed call-frame inputs".into(),
        }),
        Type::Int | Type::Float | Type::Bool => Ok(InferredOwnership {
            mode: OwnershipMode::Owned,
            lifetime: None,
            reason: "scalar copy parameters transfer no lifetime obligations back to the caller"
                .into(),
        }),
        // Slice 33Q8: struct parameters cross the boundary as a
        // borrowed `*const c_char` whose JSON is decoded by 20n-C's
        // struct decoder on entry. The lifetime semantics match the
        // string-borrowed shape — the caller owns the buffer for the
        // duration of the call only.
        ty if ty.crosses_c_abi_as_json() => Ok(InferredOwnership {
            mode: OwnershipMode::Borrowed,
            lifetime: Some("call".to_string()),
            reason: "structured extern parameters cross the boundary as borrowed JSON buffers \
                     decoded by the cdylib on entry"
                .into(),
        }),
        other => Err(format!(
            "the compiler cannot infer a stable ownership mode for extern parameter type `{}`",
            other.display_name()
        )),
    }
}

fn infer_extern_return_ownership(ty: &Type) -> Result<InferredOwnership, String> {
    match ty {
        Type::Int | Type::Float | Type::Bool | Type::Nothing | Type::String | Type::TraceId => {
            Ok(InferredOwnership {
                mode: OwnershipMode::Owned,
                lifetime: None,
                reason: "extern return values cross the boundary as owned results".into(),
            })
        }
        Type::Grounded(inner)
            if matches!(
                &**inner,
                Type::Int | Type::Float | Type::Bool | Type::String | Type::TraceId
            ) =>
        {
            Ok(InferredOwnership {
                mode: OwnershipMode::Owned,
                lifetime: None,
                reason: "grounded handles must be returned as owned lifecycle objects".into(),
            })
        }
        // Slice 33Q8: struct returns leave the boundary as an owned
        // `*mut c_char` JSON buffer the caller is responsible for
        // freeing (the existing String-return convention applies).
        ty if ty.crosses_c_abi_as_json() => Ok(InferredOwnership {
            mode: OwnershipMode::Owned,
            lifetime: None,
            reason: "structured extern returns leave the boundary as owned JSON buffers".into(),
        }),
        other => Err(format!(
            "the compiler cannot infer a stable ownership mode for extern return type `{}`",
            other.display_name()
        )),
    }
}

fn ownership_matches(declared: &OwnershipAnnotation, inferred: &InferredOwnership) -> bool {
    if declared.mode != inferred.mode {
        return false;
    }
    let declared_lifetime = declared.lifetime.as_deref().unwrap_or_else(|| {
        if matches!(declared.mode, OwnershipMode::Borrowed) {
            "call"
        } else {
            ""
        }
    });
    let inferred_lifetime = inferred.lifetime.as_deref().unwrap_or_else(|| {
        if matches!(inferred.mode, OwnershipMode::Borrowed) {
            "call"
        } else {
            ""
        }
    });
    declared_lifetime == inferred_lifetime
}

fn ownership_label_declared(annotation: &OwnershipAnnotation) -> String {
    ownership_label(annotation.mode, annotation.lifetime.as_deref())
}

fn ownership_label_inferred(annotation: &InferredOwnership) -> String {
    ownership_label(annotation.mode, annotation.lifetime.as_deref())
}

fn ownership_label(mode: OwnershipMode, lifetime: Option<&str>) -> String {
    match mode {
        OwnershipMode::Owned => "@owned".into(),
        OwnershipMode::Borrowed => match lifetime {
            Some("call") | None => "@borrowed".into(),
            Some(name) => format!("@borrowed<'{name}>"),
        },
        OwnershipMode::Shared => "@shared".into(),
        OwnershipMode::Static => "@static".into(),
    }
}
