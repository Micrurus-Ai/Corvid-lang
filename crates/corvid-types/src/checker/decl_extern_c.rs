use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::{ImportedStructType, Type};
use corvid_ast::{AgentDecl, Field, OwnershipAnnotation, OwnershipMode, TypeRef};
use corvid_resolve::DefId;
use std::path::Path;

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

    /// Slice 33Q8: `pub extern "c"` parameter type support.
    ///
    /// Accepts the v1.0 scalar set (Int / Float / Bool / String) plus
    /// user-declared structs whose fields are themselves all 20n-C
    /// codegen-supported scalars. The struct travels the C ABI as a
    /// borrowed JSON-encoded `*const c_char`; 20n-C's
    /// `lookup_or_emit_struct_decoder` decodes it on entry. Nested
    /// structs / lists / options are rejected here so the typechecker
    /// stays in lock-step with the codegen depth — promoting these
    /// shapes is a follow-up that needs encoder/decoder support first.
    pub(super) fn extern_c_param_type_supported(&self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::String => true,
            Type::Struct(def_id) => self.struct_fields_are_abi_scalars(*def_id),
            Type::ImportedStruct(imported) => {
                self.imported_struct_fields_are_abi_scalars(imported)
            }
            _ => false,
        }
    }

    /// Slice 33Q8: `pub extern "c"` return type support.
    ///
    /// Same struct support as the parameter side. The struct travels
    /// the C ABI as an owned JSON-encoded `*mut c_char`;
    /// `lookup_or_emit_struct_to_json` serializes the value at the
    /// boundary. `Grounded<scalar>` is still accepted as the
    /// pre-33Q8 attestation-handle return shape.
    pub(super) fn extern_c_return_type_supported(&self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::String | Type::Nothing => true,
            Type::Grounded(inner) => matches!(
                &**inner,
                Type::Int | Type::Float | Type::Bool | Type::String
            ),
            Type::Struct(def_id) => self.struct_fields_are_abi_scalars(*def_id),
            Type::ImportedStruct(imported) => {
                self.imported_struct_fields_are_abi_scalars(imported)
            }
            _ => false,
        }
    }

    fn struct_fields_are_abi_scalars(&self, def_id: DefId) -> bool {
        let Some(decl) = self.types_by_id.get(&def_id) else {
            return false;
        };
        decl.fields
            .iter()
            .all(|f| type_ref_is_abi_scalar(&f.ty))
    }

    fn imported_struct_fields_are_abi_scalars(&self, imported: &ImportedStructType) -> bool {
        let Some(module) = self
            .module_resolution
            .and_then(|modules| modules.lookup_by_path(Path::new(&imported.module_path)))
        else {
            return false;
        };
        let Some(fields) = imported_struct_fields(module, imported.def_id) else {
            return false;
        };
        fields.iter().all(|f| type_ref_is_abi_scalar(&f.ty))
    }
}

/// Decide whether a struct field's `TypeRef` resolves to one of the
/// scalars 20n-C's struct decoder + encoder support today. Kept on
/// `TypeRef` rather than the resolved `Type` because struct fields
/// store the syntactic type as written (cross-module struct fields
/// inside an imported module aren't always resolvable from the
/// current checker context).
fn type_ref_is_abi_scalar(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Named { name, .. } => matches!(
            name.name.as_str(),
            "Int" | "Float" | "Bool" | "String"
        ),
        _ => false,
    }
}

fn imported_struct_fields<'m>(
    module: &'m corvid_resolve::ResolvedModule,
    def_id: DefId,
) -> Option<&'m [Field]> {
    module.file.decls.iter().find_map(|decl| match decl {
        corvid_ast::Decl::Type(t)
            if module
                .resolved
                .symbols
                .lookup_def(&t.name.name)
                .is_some_and(|id| id == def_id) =>
        {
            Some(t.fields.as_slice())
        }
        _ => None,
    })
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
        Type::Struct(_) | Type::ImportedStruct(_) => Ok(InferredOwnership {
            mode: OwnershipMode::Borrowed,
            lifetime: Some("call".to_string()),
            reason: "struct extern parameters cross the boundary as borrowed JSON buffers \
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
        Type::Struct(_) | Type::ImportedStruct(_) => Ok(InferredOwnership {
            mode: OwnershipMode::Owned,
            lifetime: None,
            reason: "struct extern returns leave the boundary as owned JSON buffers".into(),
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
