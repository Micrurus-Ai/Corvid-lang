//! In-memory multi-file resolution for the playground.
//!
//! Mirrors `corvid-driver/src/modules.rs::build_module_resolution`
//! for the browser context. Key differences:
//!
//! - Reads `.cor` source from an in-memory `HashMap<String, String>`
//!   keyed by path, not from `std::fs`.
//! - Only `ImportSource::Corvid` (local) imports are loaded. Python,
//!   remote, and package imports all refuse with a playground-
//!   sandbox diagnostic.
//! - No `Corvid.lock` file. Playground source is whatever the user
//!   has in the editor.
//! - Cycle detection is in-memory: an `in_progress` set tracks the
//!   DFS frontier and any back-edge produces a single diagnostic.
//!
//! This module duplicates a small subset of the driver's module-
//! loading machinery rather than extracting the driver's logic
//! behind a trait. Per the 33J7a scope ("additive, no refactor"),
//! the trait extraction is deferred to 33J7b's runtime split,
//! where the right boundary can be drawn deliberately rather than
//! under launch pressure.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use corvid_ast::{File, ImportSource, Span};
use corvid_resolve::{
    collect_public_exports, resolve, ImportedUseTarget, ModuleResolution, Resolved,
    ResolvedModule,
};
use corvid_syntax::{lex, parse_file};

use crate::{BrowserSpan, Diagnostic, Severity};

/// One loaded module — its source path, parsed AST, and per-file
/// resolver output. Built before assembly into [`ModuleResolution`].
pub(crate) struct LoadedModule {
    // path field omitted — modules are keyed by path in the parent
    // HashMap, so storing it inline would duplicate the key. If a
    // future call site needs the path inline, add it back as
    // `pub(crate) path: PathBuf`.
    pub(crate) source: Arc<String>,
    pub(crate) file: Arc<File>,
    pub(crate) resolved: Arc<Resolved>,
}

/// Output of the in-memory module loader. The `modules` map carries
/// every successfully-loaded `.cor` file in the project (entry plus
/// transitively-reached imports). `diagnostics` carries pre-typecheck
/// errors surfaced during loading: parse errors in any file, refused
/// imports (Python / remote / package), unresolvable paths, and
/// import cycles.
pub(crate) struct LoadOutput {
    pub(crate) modules: HashMap<PathBuf, LoadedModule>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Path of the entry file inside `modules`, if it loaded
    /// successfully. `None` if the entry's source couldn't be parsed
    /// at all (lex error, etc.).
    pub(crate) entry_path: Option<PathBuf>,
}

/// Load + parse + resolve every file reachable from `entry` through
/// the in-memory `files` map. The map's keys are path-like strings
/// (e.g. `"src/main.cor"`); paths are interpreted relative to each
/// importing file.
pub(crate) fn load_project(
    files: &HashMap<String, String>,
    entry: &str,
) -> LoadOutput {
    let mut modules: HashMap<PathBuf, LoadedModule> = HashMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut in_progress: HashSet<PathBuf> = HashSet::new();

    let entry_key = normalize_web_path(entry);
    let entry_path = PathBuf::from(&entry_key);

    if files.get(&entry_key).is_none() && !files.contains_key(entry) {
        diagnostics.push(Diagnostic {
            guarantee_id: None,
            severity: Severity::Error,
            message: format!(
                "entry file `{entry}` is not in the project's file map. \
                 Provide the entry source under its path key."
            ),
            span: BrowserSpan {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 1,
            },
            help: None,
            path: Some(entry.to_string()),
        });
        return LoadOutput {
            modules,
            diagnostics,
            entry_path: None,
        };
    }

    let loaded = load_dfs(
        &entry_path,
        files,
        &mut modules,
        &mut in_progress,
        &mut diagnostics,
    );

    LoadOutput {
        modules,
        diagnostics,
        entry_path: if loaded { Some(entry_path) } else { None },
    }
}

/// DFS-load `path` and its transitive Corvid imports. Returns
/// `true` if `path` itself was parsed and resolved (the caller may
/// still see import-error diagnostics for children). Returns
/// `false` if `path` couldn't be parsed at all.
fn load_dfs(
    path: &Path,
    files: &HashMap<String, String>,
    modules: &mut HashMap<PathBuf, LoadedModule>,
    in_progress: &mut HashSet<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if modules.contains_key(path) {
        return true;
    }

    if in_progress.contains(path) {
        // Cycle: a sibling DFS branch is already walking this
        // path. The caller surfaces a single diagnostic at the
        // import that closed the cycle, not here, because here we
        // don't know which import triggered the back-edge.
        return false;
    }

    // Use a canonical web-style key for the file-map lookup so the
    // user's keys ("src/main.cor") match what the import resolver
    // computes (`resolve_import_key`).
    let path_key = normalize_web_path(&path.to_string_lossy());
    let source = match files.get(&path_key).or_else(|| {
        // Fallback: look up by the raw key as the caller wrote it.
        // Covers the entry path arriving with non-canonical separators.
        path.to_str().and_then(|s| files.get(s))
    }) {
        Some(s) => s,
        None => {
            // Caller surfaces the "module not found" diagnostic at the
            // import that referenced this path. We bail silently.
            return false;
        }
    };

    in_progress.insert(path.to_path_buf());

    let source_arc = Arc::new(source.clone());
    let tokens = match lex(source) {
        Ok(t) => t,
        Err(errs) => {
            for e in errs {
                diagnostics.push(Diagnostic {
                    guarantee_id: None,
                    severity: Severity::Error,
                    message: e.kind.to_string(),
                    span: span_at(source, e.span),
                    help: None,
                    path: Some(path_key.clone()),
                });
            }
            in_progress.remove(path);
            return false;
        }
    };

    let (file, parse_errs) = parse_file(&tokens);
    for e in parse_errs {
        diagnostics.push(Diagnostic {
            guarantee_id: None,
            severity: Severity::Error,
            message: e.kind.to_string(),
            span: span_at(source, e.span),
            help: None,
            path: Some(path_key.clone()),
        });
    }
    let file_arc = Arc::new(file);

    // DFS into Corvid imports BEFORE resolving this file. Cycle
    // detection happens at the import site, not at the parse site.
    for import in file_arc.decls.iter() {
        let corvid_ast::Decl::Import(imp) = import else {
            continue;
        };
        match imp.source {
            ImportSource::Python => {
                diagnostics.push(Diagnostic {
                    guarantee_id: None,
                    severity: Severity::Error,
                    message: "Python imports are not supported in the playground. \
                              Install Corvid locally to use the Python FFI."
                        .to_string(),
                    span: span_at(source, imp.span),
                    help: None,
                    path: Some(path_key.clone()),
                });
                continue;
            }
            ImportSource::RemoteCorvid => {
                diagnostics.push(Diagnostic {
                    guarantee_id: None,
                    severity: Severity::Error,
                    message: "remote Corvid imports (`import \"https://...\"`) \
                              are not supported in the playground. Copy the \
                              imported file's contents into a tab instead."
                        .to_string(),
                    span: span_at(source, imp.span),
                    help: None,
                    path: Some(path_key.clone()),
                });
                continue;
            }
            ImportSource::PackageCorvid => {
                diagnostics.push(Diagnostic {
                    guarantee_id: None,
                    severity: Severity::Error,
                    message: "package imports (`import \"corvid://...\"`) are \
                              not supported in the playground. Install Corvid \
                              locally to use the package manager."
                        .to_string(),
                    span: span_at(source, imp.span),
                    help: None,
                    path: Some(path_key.clone()),
                });
                continue;
            }
            ImportSource::Corvid => {}
        }

        let target_key = resolve_import_key(path, &imp.module);
        let target = PathBuf::from(&target_key);

        if in_progress.contains(&target) {
            diagnostics.push(Diagnostic {
                guarantee_id: None,
                severity: Severity::Error,
                message: format!(
                    "import cycle: `{target_key}` is part of an in-progress \
                     import chain. Refactor to break the cycle."
                ),
                span: span_at(source, imp.span),
                help: None,
                path: Some(path_key.clone()),
            });
            continue;
        }

        if !modules.contains_key(&target) {
            if !files.contains_key(&target_key) {
                diagnostics.push(Diagnostic {
                    guarantee_id: None,
                    severity: Severity::Error,
                    message: format!(
                        "module not found: `{target_key}`. Add a tab for this \
                         file in the playground, or remove the import."
                    ),
                    span: span_at(source, imp.span),
                    help: None,
                    path: Some(path_key.clone()),
                });
                continue;
            }

            load_dfs(&target, files, modules, in_progress, diagnostics);
        }
    }

    let resolved = Arc::new(resolve(&file_arc));
    for e in resolved.errors.iter().cloned() {
        diagnostics.push(Diagnostic {
            guarantee_id: None,
            severity: Severity::Error,
            message: e.kind.to_string(),
            span: span_at(source, e.span),
            help: None,
            path: Some(path_key.clone()),
        });
    }

    modules.insert(
        path.to_path_buf(),
        LoadedModule {
            source: source_arc,
            file: file_arc,
            resolved,
        },
    );

    in_progress.remove(path);
    true
}

/// Assemble a [`ModuleResolution`] from the loaded modules, given
/// the entry file. The shape mirrors what the driver's
/// `build_module_resolution` produces for the typechecker's
/// consumption.
pub(crate) fn build_resolution(
    entry: &Path,
    loaded: &HashMap<PathBuf, LoadedModule>,
) -> ModuleResolution {
    let mut modules: HashMap<String, ResolvedModule> = HashMap::new();
    let mut imported_uses: HashMap<String, ImportedUseTarget> = HashMap::new();
    let mut root_imports: HashMap<String, ResolvedModule> = HashMap::new();
    let mut all_modules: HashMap<PathBuf, ResolvedModule> = HashMap::new();

    // Pass 1: build a ResolvedModule for every loaded file. The
    // typechecker uses all_modules[path] to follow type references
    // through transitive imports.
    for (path, loaded_module) in loaded {
        let exports = collect_public_exports(&loaded_module.file, &loaded_module.resolved);
        let resolved_module = ResolvedModule {
            path: path.clone(),
            resolved: loaded_module.resolved.clone(),
            file: loaded_module.file.clone(),
            exports,
            semantic_summary: corvid_resolve::ModuleSemanticSummary::default(),
        };
        all_modules.insert(path.clone(), resolved_module);
    }

    // Pass 2: walk the entry file's direct imports to populate
    // `modules` (alias → ResolvedModule), `imported_uses` (per-use
    // bindings), and `root_imports` (module-name → ResolvedModule).
    let Some(entry_module) = loaded.get(entry) else {
        return ModuleResolution {
            modules,
            imported_uses,
            root_imports,
            all_modules,
        };
    };

    for decl in entry_module.file.decls.iter() {
        let corvid_ast::Decl::Import(imp) = decl else {
            continue;
        };
        if !matches!(imp.source, ImportSource::Corvid) {
            continue;
        }

        let target_key = resolve_import_key(entry, &imp.module);
        let target = PathBuf::from(&target_key);
        let Some(target_module) = all_modules.get(&target).cloned() else {
            continue;
        };

        if let Some(alias_ident) = imp.alias.as_ref() {
            modules.insert(alias_ident.name.clone(), target_module.clone());
        }
        root_imports.insert(imp.module.clone(), target_module.clone());

        for use_item in imp.use_items.iter() {
            let local_name = use_item
                .alias
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| use_item.name.name.clone());
            if let Some(export) = target_module.exports.get(&use_item.name.name) {
                imported_uses.insert(
                    local_name,
                    ImportedUseTarget {
                        module_path: target_module.path.clone(),
                        export: export.clone(),
                    },
                );
            }
        }
    }

    ModuleResolution {
        modules,
        imported_uses,
        root_imports,
        all_modules,
    }
}

fn span_at(source: &str, span: Span) -> BrowserSpan {
    let (start_line, start_col) = line_col_of(source, span.start);
    let (end_line, end_col) = line_col_of(source, span.end);
    BrowserSpan {
        start_line,
        start_col,
        end_line,
        end_col,
    }
}

/// Normalize a path string into canonical `forward/slash` form
/// with `.` segments dropped and `..` segments resolved. Used as
/// the canonical key for the in-memory file map so that
/// `import "./policy"` resolves the same regardless of how the
/// caller wrote the file map keys.
///
/// This is web-context normalization, not OS-context: we always
/// use `/` as the separator, never `\`, because the file-map keys
/// come from the playground's editor-tab names which are
/// web-shaped paths.
fn normalize_web_path(raw: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for seg in raw.split(|c| c == '/' || c == '\\') {
        match seg {
            "" | "." => continue,
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    stack.join("/")
}

/// Resolve an import like `./policy` against the importing file's
/// canonical path. Returns the canonical key under which the
/// imported file should be found in the file map.
fn resolve_import_key(importing_path: &Path, module: &str) -> String {
    let base = importing_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut combined = if base.is_empty() {
        module.to_string()
    } else {
        format!("{base}/{module}")
    };
    if !combined.ends_with(".cor") {
        combined.push_str(".cor");
    }
    normalize_web_path(&combined)
}

fn line_col_of(source: &str, offset: usize) -> (u32, u32) {
    let clamped = offset.min(source.len());
    let prefix = &source[..clamped];
    let line = prefix.chars().filter(|&c| c == '\n').count() as u32 + 1;
    let col = match prefix.rfind('\n') {
        Some(nl) => source[nl + 1..clamped].chars().count() as u32 + 1,
        None => prefix.chars().count() as u32 + 1,
    };
    (line, col)
}
