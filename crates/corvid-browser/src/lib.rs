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

use corvid_ast::Span;
use corvid_resolve::{resolve, ResolveError};
use corvid_syntax::errors::{LexError, ParseError};
use corvid_syntax::{lex, parse_file};
use corvid_types::{typecheck_with_config, TypeError};
use serde::Serialize;

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

const SCHEMA_VERSION: &str = "v1";

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

    // Refuse imports explicitly. The playground is single-file by
    // design (see slice 33J7-prereq scope decision). Surface ONE
    // diagnostic per `import` declaration so the editor highlights
    // each one clearly rather than cascading downstream resolve /
    // typecheck errors that would mislead the user.
    for import in imports_in(&file) {
        diagnostics.push(Diagnostic {
            guarantee_id: None,
            severity: Severity::Error,
            message: "imports are not supported in the playground; \
                      single-file source only. Install Corvid locally \
                      (`curl -fsSL https://corvid-lang.org/install.sh \
                      | sh`) to run multi-file projects."
                .to_string(),
            span: browser_span_of(source, import),
            help: Some(
                "remove the `import` declaration, or copy the imported \
                 module's contents inline."
                    .to_string(),
            ),
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
    }
}

fn diag_from_parse(e: ParseError, source: &str) -> Diagnostic {
    Diagnostic {
        guarantee_id: None,
        severity: Severity::Error,
        message: e.kind.to_string(),
        span: browser_span_of(source, e.span),
        help: None,
    }
}

fn diag_from_resolve(e: ResolveError, source: &str) -> Diagnostic {
    Diagnostic {
        guarantee_id: None,
        severity: Severity::Error,
        message: e.kind.to_string(),
        span: browser_span_of(source, e.span),
        help: None,
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
    }
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
