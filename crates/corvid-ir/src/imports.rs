//! Import-specific lowering support.
//!
//! `lower.rs` owns AST-to-IR traversal. This module owns the
//! cross-file identity problem: imported modules have their own
//! per-file `DefId` spaces, so IR lowering assigns synthetic root
//! `DefId`s before imported declarations are appended to the root IR.

use corvid_resolve::{DeclKind, DefId, ModuleResolution, Resolved, ResolvedModule};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImportedDefKey {
    pub(crate) module_path: String,
    pub(crate) def_id: DefId,
}

pub(crate) fn build_imported_def_ids(
    resolved: &Resolved,
    modules: &ModuleResolution,
) -> HashMap<ImportedDefKey, DefId> {
    let mut next = resolved
        .symbols
        .entries()
        .iter()
        .map(|entry| entry.id.0)
        .max()
        .map(|max| max + 1)
        .unwrap_or(0);
    let mut out = HashMap::new();
    let mut loaded = modules.all_modules.values().collect::<Vec<_>>();
    loaded.sort_by(|a, b| a.path.cmp(&b.path));
    for module in loaded {
        let module_path = module.path.to_string_lossy().into_owned();
        for entry in module.resolved.symbols.entries() {
            if matches!(
                entry.kind,
                DeclKind::Type | DeclKind::Tool | DeclKind::Prompt | DeclKind::Agent | DeclKind::Fn
            ) {
                out.insert(
                    ImportedDefKey {
                        module_path: module_path.clone(),
                        def_id: entry.id,
                    },
                    DefId(next),
                );
                next += 1;
            }
        }
    }
    out
}

/// Source modules keyed by the same remapped IDs used by cross-file IR calls.
pub fn imported_source_modules<'a>(
    resolved: &Resolved,
    modules: &'a ModuleResolution,
) -> HashMap<DefId, &'a ResolvedModule> {
    build_imported_def_ids(resolved, modules).into_iter().filter_map(|(key, id)| {
        modules.lookup_by_path(std::path::Path::new(&key.module_path)).map(|module| (id, module))
    }).collect()
}
