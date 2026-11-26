//! Collect schemas reachable from a file-backed application contract. Resolve
//! each field in the module that declared it before assigning a contract name.
use corvid_ast::{Decl, File, Ident, Param, TypeDecl, TypeRef, Visibility};
use corvid_resolve::{DeclKind, ModuleResolution};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

type TypeKey = (PathBuf, String);

#[derive(Default)]
pub(super) struct Imports {
    aliases: HashMap<String, PathBuf>,
    lifted: HashMap<String, TypeKey>,
}

impl From<&ModuleResolution> for Imports {
    fn from(graph: &ModuleResolution) -> Self {
        Self {
            aliases: graph
                .modules
                .iter()
                .map(|(alias, m)| (alias.clone(), m.path.clone()))
                .collect(),
            lifted: graph
                .imported_uses
                .iter()
                .filter(|(_, t)| t.export.kind == DeclKind::Type)
                .map(|(alias, t)| {
                    (
                        alias.clone(),
                        (t.module_path.clone(), t.export.name.clone()),
                    )
                })
                .collect(),
        }
    }
}

/// `graph` is `None` for a program without imports; its routes still
/// publish schemas for the root file's own non-public types.
pub(super) fn include_exchange_types(
    root: &mut File,
    graph: Option<&ModuleResolution>,
    scopes: &HashMap<PathBuf, Imports>,
) {
    let root_path = PathBuf::new();
    let mut types = HashMap::new();
    for (path, file) in std::iter::once((&root_path, &*root))
        .chain(graph.into_iter().flat_map(|g| g.all_modules.iter().map(|(p, m)| (p, m.file.as_ref()))))
    {
        for decl in &file.decls {
            let record = match decl {
                Decl::Type(t) => t.clone(),
                Decl::Table(table) => {
                    // Reachable rows need a synthetic public schema even if
                    // the table itself was exported for other Corvid modules.
                    let mut record = table.record.clone();
                    record.visibility = Visibility::Private;
                    record
                }
                _ => continue,
            };
            types.insert((path.clone(), record.name.name.clone()), record);
        }
    }
    let mut schemas = Schemas {
        declarations: types,
        names: HashMap::new(),
        reserved: root
            .decls
            .iter()
            .filter_map(|decl| match decl {
                Decl::Type(t) => Some(t.name.name.clone()),
                Decl::Table(table) => Some(table.name().name.clone()),
                _ => None,
            })
            .collect(),
        pending: Vec::new(),
    };
    let imports = graph.map(Imports::from).unwrap_or_default();
    for decl in &mut root.decls {
        match decl {
            Decl::Server(server) => {
                for route in &mut server.routes {
                    for p in &mut route.path_params {
                        schemas.resolve(&mut p.ty, &root_path, &imports);
                    }
                    for ty in route
                        .query_ty
                        .iter_mut()
                        .chain(&mut route.body_ty)
                        .chain(std::iter::once(&mut route.response.ty))
                    {
                        schemas.resolve(ty, &root_path, &imports);
                    }
                }
            }
            Decl::Type(t) if t.visibility.is_callable_from_outside_file() => {
                schemas.fields(t, &root_path, &imports);
            }
            Decl::Agent(a) if a.visibility.is_callable_from_outside_file() => {
                schemas.signature(&mut a.params, &mut a.return_ty, &root_path, &imports)
            }
            Decl::Prompt(p) if p.visibility.is_callable_from_outside_file() => {
                schemas.signature(&mut p.params, &mut p.return_ty, &root_path, &imports)
            }
            _ => {}
        }
    }
    let empty = Imports::default();
    while let Some(key) = schemas.pending.pop() {
        let mut record = schemas.declarations[&key].clone();
        let scope = if key.0 == root_path {
            &imports
        } else {
            scopes.get(&key.0).unwrap_or(&empty)
        };
        schemas.fields(&mut record, &key.0, scope);
        record.name.name = schemas.names[&key].clone();
        record.visibility = Visibility::Public;
        root.decls.push(Decl::Type(record));
    }
}

struct Schemas {
    declarations: HashMap<TypeKey, TypeDecl>,
    names: HashMap<TypeKey, String>,
    reserved: HashSet<String>,
    pending: Vec<TypeKey>,
}

impl Schemas {
    fn signature(
        &mut self,
        params: &mut [Param],
        result: &mut TypeRef,
        path: &PathBuf,
        imports: &Imports,
    ) {
        for p in params {
            self.resolve(&mut p.ty, path, imports);
        }
        self.resolve(result, path, imports);
    }

    fn fields(&mut self, t: &mut TypeDecl, path: &PathBuf, imports: &Imports) {
        if let Some(target) = &mut t.alias {
            self.resolve(target, path, imports);
        }
        for field in t
            .fields
            .iter_mut()
            .chain(t.variants.iter_mut().flat_map(|v| &mut v.fields))
        {
            self.resolve(&mut field.ty, path, imports);
        }
    }

    fn resolve(&mut self, ty: &mut TypeRef, path: &PathBuf, imports: &Imports) {
        let key = match ty {
            TypeRef::Named { name, .. } => imports
                .lifted
                .get(&name.name)
                .cloned()
                .unwrap_or_else(|| (path.clone(), name.name.clone())),
            TypeRef::Qualified { alias, name, .. } => {
                let Some(target) = imports.aliases.get(&alias.name) else {
                    return;
                };
                (target.clone(), name.name.clone())
            }
            TypeRef::Generic { args, .. } => {
                for arg in args {
                    self.resolve(arg, path, imports);
                }
                return;
            }
            TypeRef::Weak { inner, .. } => {
                self.resolve(inner, path, imports);
                return;
            }
            TypeRef::Function { .. } => return,
        };
        let Some(record) = self.declarations.get(&key) else {
            return;
        };
        let is_root = key.0.as_os_str().is_empty();
        if is_root && record.visibility.is_callable_from_outside_file() {
            return;
        }
        let name = self.names.entry(key.clone()).or_insert_with(|| {
            let mut name = key.1.clone();
            if !is_root {
                let mut suffix = 2;
                while !self.reserved.insert(name.clone()) {
                    name = format!("{}_{suffix}", key.1);
                    suffix += 1;
                }
            }
            self.pending.push(key);
            name
        });
        let span = ty.span();
        *ty = TypeRef::Named {
            name: Ident::new(name.clone(), span),
            span,
        };
    }
}
