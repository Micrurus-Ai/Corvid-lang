//! Which declared type, if any, an expression names when it is the
//! receiver of a type method such as `Order.from_json(text)`.
//!
//! The checker and the IR lowerer both need this answer and must give
//! the same one, so it lives here and both call it. A receiver names a
//! type when it is:
//!
//! - a local type name (`Order`);
//! - a type brought in with `import "./models" use Customer`;
//! - a type reached through a module alias (`m.Customer`).

use corvid_ast::{Expr, Span};
use corvid_resolve::{Binding, DeclKind, DefId, ModuleLookup, ModuleResolution, ResolvedModule, SymbolTable};
use std::collections::HashMap;
use std::path::Path;

/// The type a type-method receiver names.
#[derive(Debug, Clone, Copy)]
pub enum TypeReceiver<'m> {
    /// A type declared in the file being checked.
    Local(DefId),
    /// A public type declared in another module. `def_id` is that
    /// module's own id for it.
    Imported {
        module: &'m ResolvedModule,
        def_id: DefId,
    },
}

/// Resolve `target` as a type-method receiver. Returns `None` when it
/// does not name a type, so ordinary method dispatch applies.
pub fn type_receiver<'m>(
    target: &Expr,
    bindings: &HashMap<Span, Binding>,
    symbols: &SymbolTable,
    modules: Option<&'m ModuleResolution>,
) -> Option<TypeReceiver<'m>> {
    match target {
        Expr::Ident { name, .. } => {
            let Binding::Decl(id) = bindings.get(&name.span)? else { return None };
            match symbols.get(*id).kind {
                DeclKind::Type => Some(TypeReceiver::Local(*id)),
                DeclKind::ImportedUse => {
                    let modules = modules?;
                    let used = modules.lookup_imported_use(&name.name)?;
                    if used.export.kind != DeclKind::Type {
                        return None;
                    }
                    let module = modules.lookup_by_path(Path::new(&used.module_path))?;
                    Some(TypeReceiver::Imported { module, def_id: used.export.def_id })
                }
                _ => None,
            }
        }
        Expr::FieldAccess { target: alias, field, .. } => {
            let Expr::Ident { name: alias, .. } = alias.as_ref() else { return None };
            let Binding::Decl(id) = bindings.get(&alias.span)? else { return None };
            if symbols.get(*id).kind != DeclKind::Import {
                return None;
            }
            match modules?.lookup_member(&alias.name, &field.name) {
                ModuleLookup::Found { module, export } if export.kind == DeclKind::Type => {
                    Some(TypeReceiver::Imported { module, def_id: export.def_id })
                }
                _ => None,
            }
        }
        _ => None,
    }
}
