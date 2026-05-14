//! Browser-facing typechecker entry for the Corvid playground.
//!
//! The 33J7 playground (browser-based code editor at
//! `corvid-lang.org/playground`) compiles user-written `.cor` source
//! in the browser via this crate, then renders the resulting
//! diagnostics with `guarantee_id` badges that link to
//! `/docs/reference/guarantees#<id>`.
//!
//! Pipeline (mirrors `corvid-driver/src/pipeline/compile.rs` for the
//! typecheck-only subset):
//!   1. `corvid_syntax::lex` — tokenize
//!   2. `corvid_syntax::parse_file` — parse to AST
//!   3. `corvid_resolve::resolve` — name resolution
//!   4. `corvid_types::typecheck_with_config` — effect & type check
//!
//! Out of scope for the WASM build: codegen, runtime, LLM provider
//! calls, connectors, replay, jobs. The playground is typecheck-only;
//! running agents happens locally after `curl install`.
//!
//! Imports are explicitly refused: the playground is single-file by
//! design. An `import` declaration produces a single
//! `browser.imports_not_supported` diagnostic so the editor surfaces
//! the limitation cleanly.

use std::collections::HashMap;

use corvid_ast::Span;
use corvid_resolve::{resolve, ResolveError};
use corvid_syntax::errors::{LexError, ParseError};
use corvid_syntax::{lex, parse_file};
use corvid_types::{typecheck_with_config, typecheck_with_config_and_modules, TypeError};
use serde::Serialize;

mod examples;
mod multi_file;

pub use examples::{check_example, list_examples, ExampleCatalog, ExampleMeta};

/// Wire-format diagnostic. Flat schema by design (one primary span per
/// diagnostic plus an optional help string). Multi-span / related-info
/// fields can be added later as additive `Option<Vec<...>>` fields —
/// the `version` field on `CheckResult` signals schema-level changes
/// so older renderers can detect mismatches.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    /// Stable id of the compile-time guarantee this diagnostic
    /// enforces. `None` for general well-formedness errors that do
    /// not back a public Corvid promise. When present, the website
    /// renderer should link to
    /// `/docs/reference/guarantees#<guarantee_id>`.
    pub guarantee_id: Option<&'static str>,
    pub severity: Severity,
    pub message: String,
    pub span: BrowserSpan,
    pub help: Option<String>,
    /// Source file the diagnostic belongs to, in projects with more
    /// than one file. `None` for the single-file [`check`] entry
    /// (the diagnostic is unambiguously about the one source). `Some`
    /// for [`check_project`], where the playground needs to route
    /// the squiggle to the right editor tab.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// 1-indexed line and column, both inclusive. Columns count Unicode
/// characters rather than bytes so editor squiggles align with what
/// the user sees, not what the parser sees.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BrowserSpan {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// The wire-format response from [`check`].
#[derive(Clone, Debug, Serialize)]
pub struct CheckResult {
    /// Schema version. Bump if the wire format changes in a
    /// non-additive way. Renderers may assert on this.
    pub version: &'static str,
    /// `true` iff there are zero `Severity::Error` diagnostics.
    /// Warnings do not flip `ok` to `false`.
    pub ok: bool,
    pub diagnostics: Vec<Diagnostic>,
}

pub(crate) const SCHEMA_VERSION: &str = "v1";

/// Typecheck `source` and return diagnostics as a flat wire-format
/// `CheckResult`. This is the only function the playground needs to
/// call. Native callers can use this rlib directly; the WASM build
/// re-exports it as a `wasm-bindgen` entry under the same name.
pub fn check(source: &str) -> CheckResult {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    let tokens = match lex(source) {
        Ok(t) => t,
        Err(errs) => {
            for e in errs {
                diagnostics.push(diag_from_lex(e, source));
            }
            return finalize(diagnostics);
        }
    };

    let (file, parse_errs) = parse_file(&tokens);
    for e in parse_errs {
        diagnostics.push(diag_from_parse(e, source));
    }

    // Refuse imports in the single-file entry. The website
    // playground routes multi-file projects through `check_project`
    // (slice 33J7a) instead. Surface ONE diagnostic per `import`
    // declaration so the editor highlights each one cleanly rather
    // than cascading downstream resolve / typecheck errors that
    // would mislead the user.
    for import in imports_in(&file) {
        diagnostics.push(Diagnostic {
            guarantee_id: None,
            severity: Severity::Error,
            message: "imports are not supported in the single-file \
                      `check` entry. Use `check_project` for multi-file \
                      sources, or install Corvid locally (`curl -fsSL \
                      https://corvid-lang.org/install.sh | sh`) to run \
                      full projects."
                .to_string(),
            span: browser_span_of(source, import),
            help: Some(
                "remove the `import` declaration, or route this file \
                 through `check_project`."
                    .to_string(),
            ),
            path: None,
        });
    }
    // If any imports were found, stop here. Resolve and typecheck
    // would cascade with "unknown name" errors that confuse the user.
    if diagnostics.iter().any(|d| {
        d.guarantee_id.is_none()
            && d.message.starts_with("imports are not supported")
    }) {
        return finalize(diagnostics);
    }

    let resolved = resolve(&file);
    for e in resolved.errors.iter().cloned() {
        diagnostics.push(diag_from_resolve(e, source));
    }

    let checked = typecheck_with_config(&file, &resolved, None);
    for e in checked.errors.iter().cloned() {
        diagnostics.push(diag_from_type(e, source));
    }

    finalize(diagnostics)
}

fn finalize(diagnostics: Vec<Diagnostic>) -> CheckResult {
    let ok = !diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error));
    CheckResult {
        version: SCHEMA_VERSION,
        ok,
        diagnostics,
    }
}

fn diag_from_lex(e: LexError, source: &str) -> Diagnostic {
    Diagnostic {
        guarantee_id: None,
        severity: Severity::Error,
        message: e.kind.to_string(),
        span: browser_span_of(source, e.span),
        help: None,
        path: None,
    }
}

fn diag_from_parse(e: ParseError, source: &str) -> Diagnostic {
    Diagnostic {
        guarantee_id: None,
        severity: Severity::Error,
        message: e.kind.to_string(),
        span: browser_span_of(source, e.span),
        help: None,
        path: None,
    }
}

fn diag_from_resolve(e: ResolveError, source: &str) -> Diagnostic {
    Diagnostic {
        guarantee_id: None,
        severity: Severity::Error,
        message: e.kind.to_string(),
        span: browser_span_of(source, e.span),
        help: None,
        path: None,
    }
}

fn diag_from_type(e: TypeError, source: &str) -> Diagnostic {
    let help = e.hint();
    let message = e.message();
    Diagnostic {
        guarantee_id: e.guarantee_id,
        severity: Severity::Error,
        message,
        span: browser_span_of(source, e.span),
        help,
        path: None,
    }
}

/// Multi-file typecheck for the playground (slice 33J7a).
///
/// Like [`check`], but accepts a `HashMap<String, String>` of
/// path-keyed source files. Resolves imports through the map
/// instead of the filesystem. The `entry` key names the file to
/// typecheck against; transitive `import "./other"` references are
/// loaded from the same map.
///
/// All diagnostics carry their source file in
/// [`Diagnostic::path`] so the playground can route squiggles to
/// the right editor tab.
///
/// Imports outside the map refuse with a "module not found"
/// diagnostic anchored at the import site. Python / remote /
/// package imports refuse with playground-sandbox diagnostics.
/// Cycles surface as a single diagnostic at the import that closed
/// the back-edge.
pub fn check_project(files: &HashMap<String, String>, entry: &str) -> CheckResult {
    let mut load_output = multi_file::load_project(files, entry);
    let mut diagnostics = std::mem::take(&mut load_output.diagnostics);

    let Some(entry_path) = load_output.entry_path else {
        return finalize(diagnostics);
    };

    let resolution = multi_file::build_resolution(&entry_path, &load_output.modules);

    let entry_module = load_output
        .modules
        .get(&entry_path)
        .expect("entry_path was populated only after successful load");
    let entry_path_key = entry_path
        .to_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| entry_path.display().to_string());

    let checked = typecheck_with_config_and_modules(
        &entry_module.file,
        &entry_module.resolved,
        None,
        &resolution,
    );

    for e in checked.errors.iter().cloned() {
        let mut d = diag_from_type(e, entry_module.source.as_str());
        d.path = Some(entry_path_key.clone());
        diagnostics.push(d);
    }

    finalize(diagnostics)
}

/// Convert an internal byte-offset `Span` into a 1-indexed
/// (line, col) wire-format `BrowserSpan`. Columns count Unicode
/// characters, not bytes; this aligns editor squiggles with what the
/// user sees rather than what the parser sees.
fn browser_span_of(source: &str, span: Span) -> BrowserSpan {
    let (start_line, start_col) = line_col_of(source, span.start);
    let (end_line, end_col) = line_col_of(source, span.end);
    BrowserSpan {
        start_line,
        start_col,
        end_line,
        end_col,
    }
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

fn imports_in(file: &corvid_ast::File) -> Vec<Span> {
    file.decls
        .iter()
        .filter_map(|d| match d {
            corvid_ast::Decl::Import(i) => Some(i.span),
            _ => None,
        })
        .collect()
}

// -----------------------------------------------------------------
// wasm-bindgen entry — only compiled for wasm32-unknown-unknown.
// Native builds use `check` directly through the rlib target.
// -----------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = check)]
pub fn check_wasm(source: &str) -> JsValue {
    let result = check(source);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// `wasm-bindgen` entry for [`check_project`]. The browser passes
/// a plain JS object `{ "path/to/file.cor": "source", ... }` plus an
/// entry path string; we reflect it into a HashMap via
/// `serde-wasm-bindgen` and call the rlib function.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = checkProject)]
pub fn check_project_wasm(files: JsValue, entry: &str) -> JsValue {
    let files: HashMap<String, String> =
        match serde_wasm_bindgen::from_value(files) {
            Ok(f) => f,
            Err(_) => {
                let err = CheckResult {
                    version: SCHEMA_VERSION,
                    ok: false,
                    diagnostics: vec![Diagnostic {
                        guarantee_id: None,
                        severity: Severity::Error,
                        message:
                            "checkProject expected an object mapping path → source".into(),
                        span: BrowserSpan {
                            start_line: 1,
                            start_col: 1,
                            end_line: 1,
                            end_col: 1,
                        },
                        help: None,
                        path: None,
                    }],
                };
                return serde_wasm_bindgen::to_value(&err).unwrap_or(JsValue::NULL);
            }
        };
    let result = check_project(&files, entry);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}
