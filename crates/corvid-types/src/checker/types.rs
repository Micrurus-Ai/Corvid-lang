//! TypeRef -> Type resolution.
//!
//! Given an AST type reference (what the user wrote), produce the
//! structural `Type` the rest of the checker works with. Handles
//! primitives, user-declared structs, imported structs, and the
//! compiler-known generics (`List`, `Stream`, `Option`, `Result`,
//! `Grounded`, `Partial`, `ResumeToken`, `Weak`), plus arity validation on each generic.

use super::{is_weakable_type, Checker};
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::{ImportedStructType, Type};
use corvid_ast::{TypeRef, WeakEffectRow};

impl<'a> Checker<'a> {
    pub(super) fn type_ref_to_type(&mut self, tr: &TypeRef) -> Type {
        match tr {
            TypeRef::Named { name, .. } => self.named_type_to_type(&name.name, name.span),
            TypeRef::Qualified { alias, name, span } => {
                self.qualified_type_ref_to_type(&alias.name, &name.name, *span)
            }
            TypeRef::Generic { name, args, span } => {
                self.generic_type_ref_to_type(&name.name, args, *span, TypeContext::Root)
            }
            TypeRef::Weak {
                inner,
                effects,
                span,
            } => {
                let inner_ty = self.type_ref_to_type(inner);
                if !is_weakable_type(&inner_ty) && !matches!(inner_ty, Type::Unknown) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidWeakTargetType {
                            got: inner_ty.display_name(),
                        },
                        *span,
                    ));
                    return Type::Weak(
                        Box::new(Type::Unknown),
                        effects.unwrap_or_else(WeakEffectRow::any),
                    );
                }
                Type::Weak(
                    Box::new(inner_ty),
                    effects.unwrap_or_else(WeakEffectRow::any),
                )
            }
            // `(Int, Int) -> Int` (slice 45j) — resolves to a real
            // function type; lambdas are the values that inhabit it.
            TypeRef::Function { params, ret, .. } => Type::Function {
                params: params.iter().map(|p| self.type_ref_to_type(p)).collect(),
                ret: Box::new(self.type_ref_to_type(ret)),
                effect: corvid_ast::Effect::Safe,
            },
        }
    }

    pub(super) fn imported_type_ref_to_type(
        &mut self,
        tr: &TypeRef,
        module: &corvid_resolve::ResolvedModule,
    ) -> Type {
        match tr {
            TypeRef::Named { name, .. } => self.named_type_in_module(&name.name, module),
            TypeRef::Qualified { alias, name, span } => {
                let Some(modules) = self.module_resolution else {
                    return Type::Unknown;
                };
                let Some(target_module) =
                    imported_module_alias_target(module, modules, &alias.name)
                else {
                    return Type::Unknown;
                };
                match target_module.exports.get(&name.name) {
                    Some(export) if matches!(export.kind, corvid_resolve::DeclKind::Type) => {
                        imported_struct_type(target_module, export.def_id, &export.name)
                    }
                    Some(_) => {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::TypeAsValue {
                                name: format!("{}.{}", alias.name, name.name),
                            },
                            *span,
                        ));
                        Type::Unknown
                    }
                    None => Type::Unknown,
                }
            }
            TypeRef::Generic { name, args, span } => self.generic_type_ref_to_type(
                &name.name,
                args,
                *span,
                TypeContext::Imported(module),
            ),
            TypeRef::Weak { inner, effects, .. } => Type::Weak(
                Box::new(self.imported_type_ref_to_type(inner, module)),
                effects.unwrap_or_else(WeakEffectRow::any),
            ),
            TypeRef::Function { params, ret, .. } => Type::Function {
                params: params
                    .iter()
                    .map(|p| self.imported_type_ref_to_type(p, module))
                    .collect(),
                ret: Box::new(self.imported_type_ref_to_type(ret, module)),
                effect: corvid_ast::Effect::Safe,
            },
        }
    }

    pub(super) fn named_type_to_type(&mut self, name: &str, span: corvid_ast::Span) -> Type {
        match name {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Bool" => Type::Bool,
            "Nothing" => Type::Nothing,
            // Phase 33S3a — `DbHandle` is an opaque, refcounted
            // primitive type produced ONLY by the executing
            // `db_open` stdlib tool (see `std/db.cor` from 33S3b).
            // Users cannot construct or fabricate a value of this
            // type; the language-level promise is that any
            // `DbHandle` your code holds came from `db_open`. The
            // value-side representation is `Value::DbHandle` in
            // `corvid-vm`; the JSON marshalling path rejects this
            // type because there is no JSON representation for the
            // refcounted Arc inside.
            "DbHandle" => Type::DbHandle,
            "TraceId" => Type::TraceId,
            // Phase 33R5b-a — JsonValue and JsonBuilder are
            // opaque primitives produced ONLY by the executing
            // stdlib JSON tools. Same shape as DbHandle but
            // without the opacity gate at json_to_value (the
            // payload IS the JSON shape; no underlying registry).
            "JsonValue" => Type::JsonValue,
            "JsonBuilder" => Type::JsonBuilder,
            _ => match self.symbols.lookup_def(name) {
                Some(id) => {
                    let entry = self.symbols.get(id);
                    if entry.kind == corvid_resolve::DeclKind::ImportedUse {
                        return self.imported_use_type_to_type(name, span);
                    }
                    self.expand_possible_alias(id, span)
                }
                None => Type::Unknown,
            },
        }
    }

    /// Expand a transparent type alias (slice 45n): a def whose
    /// declaration is `type X = T` resolves to T everywhere. Chains
    /// expand recursively; a cycle errors instead of recursing
    /// forever.
    pub(super) fn expand_possible_alias(&mut self, id: corvid_resolve::DefId, span: corvid_ast::Span) -> Type {
        let target = match self.types_by_id.get(&id) {
            Some(decl) => match &decl.alias {
                Some(tr) => (tr.clone(), decl.name.name.clone()),
                None => return Type::Struct(id),
            },
            None => return Type::Struct(id),
        };
        if self.alias_depth >= 32 {
            self.errors.push(TypeError::new(
                TypeErrorKind::AliasCycle { name: target.1 },
                span,
            ));
            return Type::Unknown;
        }
        self.alias_depth += 1;
        let ty = self.type_ref_to_type(&target.0);
        self.alias_depth -= 1;
        ty
    }

    fn imported_use_type_to_type(&mut self, name: &str, span: corvid_ast::Span) -> Type {
        let Some(modules) = self.module_resolution else {
            return Type::Unknown;
        };
        let Some(target) = modules.lookup_imported_use(name) else {
            self.errors.push(TypeError::new(
                TypeErrorKind::UnknownImportMember {
                    alias: "<import use>".to_string(),
                    name: name.to_string(),
                },
                span,
            ));
            return Type::Unknown;
        };
        let Some(module) = modules.lookup_by_path(&target.module_path) else {
            return Type::Unknown;
        };
        if !matches!(target.export.kind, corvid_resolve::DeclKind::Type) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeAsValue {
                    name: name.to_string(),
                },
                span,
            ));
            return Type::Unknown;
        }
        imported_struct_type(module, target.export.def_id, &target.export.name)
    }

    fn qualified_type_ref_to_type(
        &mut self,
        alias: &str,
        name: &str,
        span: corvid_ast::Span,
    ) -> Type {
        let Some(modules) = self.module_resolution else {
            self.errors.push(TypeError::new(
                TypeErrorKind::CorvidImportNotYetResolved {
                    alias: alias.to_string(),
                    name: name.to_string(),
                },
                span,
            ));
            return Type::Unknown;
        };

        match modules.lookup_member(alias, name) {
            corvid_resolve::ModuleLookup::Found { module, export } => {
                if !matches!(export.kind, corvid_resolve::DeclKind::Type) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeAsValue {
                            name: format!("{alias}.{name}"),
                        },
                        span,
                    ));
                    return Type::Unknown;
                }
                imported_struct_type(module, export.def_id, &export.name)
            }
            corvid_resolve::ModuleLookup::UnknownAlias => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnknownImportAlias {
                        alias: alias.to_string(),
                    },
                    span,
                ));
                Type::Unknown
            }
            corvid_resolve::ModuleLookup::Private => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::ImportedDeclIsPrivate {
                        alias: alias.to_string(),
                        name: name.to_string(),
                    },
                    span,
                ));
                Type::Unknown
            }
            corvid_resolve::ModuleLookup::UnknownMember => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::UnknownImportMember {
                        alias: alias.to_string(),
                        name: name.to_string(),
                    },
                    span,
                ));
                Type::Unknown
            }
        }
    }

    fn named_type_in_module(&self, name: &str, module: &corvid_resolve::ResolvedModule) -> Type {
        match name {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Bool" => Type::Bool,
            "Nothing" => Type::Nothing,
            "DbHandle" => Type::DbHandle,
            "TraceId" => Type::TraceId,
            "JsonValue" => Type::JsonValue,
            "JsonBuilder" => Type::JsonBuilder,
            _ => match module.resolved.symbols.lookup_def(name) {
                Some(def_id)
                    if matches!(
                        module.resolved.symbols.get(def_id).kind,
                        corvid_resolve::DeclKind::Type
                    ) =>
                {
                    imported_struct_type(module, def_id, name)
                }
                _ => Type::Unknown,
            },
        }
    }

    fn generic_type_ref_to_type(
        &mut self,
        name: &str,
        args: &[TypeRef],
        span: corvid_ast::Span,
        context: TypeContext<'_>,
    ) -> Type {
        let resolve_arg = |checker: &mut Self, arg: &TypeRef| match context {
            TypeContext::Root => checker.type_ref_to_type(arg),
            TypeContext::Imported(module) => checker.imported_type_ref_to_type(arg, module),
        };

        match name {
            "List" | "Stream" | "Option" | "Grounded" | "Tainted" | "Partial" | "ResumeToken" => {
                if args.len() != 1 {
                    if matches!(context, TypeContext::Root) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::GenericArityMismatch {
                                name: name.to_string(),
                                expected: 1,
                                got: args.len(),
                            },
                            span,
                        ));
                    }
                    return Type::Unknown;
                }
                let inner = Box::new(resolve_arg(self, &args[0]));
                match name {
                    "List" => Type::List(inner),
                    "Stream" => Type::Stream(inner),
                    "Option" => Type::Option(inner),
                    "Grounded" => Type::Grounded(inner),
                    "Tainted" => Type::Tainted(inner),
                    "Partial" => Type::Partial(inner),
                    "ResumeToken" => Type::ResumeToken(inner),
                    _ => unreachable!(),
                }
            }
            "Map" => {
                if args.len() != 2 {
                    if matches!(context, TypeContext::Root) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::GenericArityMismatch {
                                name: name.to_string(),
                                expected: 2,
                                got: args.len(),
                            },
                            span,
                        ));
                    }
                    return Type::Unknown;
                }
                Type::Map(
                    Box::new(resolve_arg(self, &args[0])),
                    Box::new(resolve_arg(self, &args[1])),
                )
            }
            "Result" => {
                if args.len() != 2 {
                    if matches!(context, TypeContext::Root) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::GenericArityMismatch {
                                name: name.to_string(),
                                expected: 2,
                                got: args.len(),
                            },
                            span,
                        ));
                    }
                    return Type::Unknown;
                }
                Type::Result(
                    Box::new(resolve_arg(self, &args[0])),
                    Box::new(resolve_arg(self, &args[1])),
                )
            }
            _ => {
                // Slice 45q leniency hardening: an unknown generic
                // head is an ERROR (it used to silently become
                // `Type::Unknown`, which is assignable to
                // everything). Only in root context — imported
                // modules are validated by their own check.
                if matches!(context, TypeContext::Root) {
                    const HEADS: [&str; 9] = [
                        "List",
                        "Map",
                        "Option",
                        "Result",
                        "Stream",
                        "Weak",
                        "Grounded",
                        "Partial",
                        "ResumeToken",
                    ];
                    let suggestion = HEADS
                        .iter()
                        .find(|h| edit_distance(&h.to_lowercase(), &name.to_lowercase()) <= 2)
                        .map(|h| h.to_string());
                    self.errors.push(TypeError::new(
                        TypeErrorKind::UnknownGenericHead {
                            name: name.to_string(),
                            suggestion,
                        },
                        span,
                    ));
                }
                Type::Unknown
            }
        }
    }
}

#[derive(Clone, Copy)]
enum TypeContext<'a> {
    Root,
    Imported(&'a corvid_resolve::ResolvedModule),
}

fn imported_struct_type(
    module: &corvid_resolve::ResolvedModule,
    def_id: corvid_resolve::DefId,
    name: &str,
) -> Type {
    Type::ImportedStruct(ImportedStructType {
        module_path: module.path.to_string_lossy().into_owned(),
        def_id,
        name: name.to_string(),
    })
}

fn imported_module_alias_target<'a>(
    module: &corvid_resolve::ResolvedModule,
    modules: &'a corvid_resolve::ModuleResolution,
    alias: &str,
) -> Option<&'a corvid_resolve::ResolvedModule> {
    let import = module.file.decls.iter().find_map(|decl| match decl {
        corvid_ast::Decl::Import(import)
            if matches!(
                import.source,
                corvid_ast::ImportSource::Corvid
                    | corvid_ast::ImportSource::RemoteCorvid
                    | corvid_ast::ImportSource::PackageCorvid
            ) && import.alias.as_ref().is_some_and(|a| a.name == alias) =>
        {
            Some(import)
        }
        _ => None,
    })?;
    let child = match import.source {
        corvid_ast::ImportSource::Corvid => {
            corvid_resolve::resolve_import_path(&module.path, &import.module)
        }
        corvid_ast::ImportSource::RemoteCorvid => {
            corvid_resolve::remote_import_path(&import.module)
        }
        corvid_ast::ImportSource::PackageCorvid => {
            corvid_resolve::remote_import_path(&import.module)
        }
        corvid_ast::ImportSource::Python => return None,
    };
    modules.lookup_by_path(&child)
}


/// Small Levenshtein distance for did-you-mean suggestions (45q).
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
