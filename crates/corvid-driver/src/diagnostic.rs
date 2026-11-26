//! Unified `Diagnostic` type for every compiler-stage error.
//!
//! The driver collects errors from lex, parse, resolve, and typecheck
//! stages, each with its own error type. This module normalizes them
//! through `From` impls into one `Diagnostic` shape so CLI, REPL,
//! and programmatic consumers see a single renderable error form.
//!
//! Extracted from `lib.rs` as part of Phase 20i responsibility
//! decomposition (20i-audit-driver-a).

use corvid_ast::Span;
use corvid_resolve::ResolveError;
use corvid_syntax::errors::{LexError, ParseError};
use corvid_types::{TypeError, TypeWarning};
use std::fmt;
use std::path::Path;

/// A unified diagnostic from any compiler stage, with a span that can be
/// rendered against the original source.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Imported source and path when the span belongs to another module.
    pub origin: Option<(std::path::PathBuf, std::sync::Arc<String>)>,
    pub span: Span,
    pub message: String,
    pub hint: Option<String>,
    /// The code the diagnostic's kind declares (`E0208`). Every source
    /// declares one, never inferred from the message, so rewording a
    /// message never changes its code.
    pub code: &'static str,
}

impl Diagnostic {
    pub(crate) fn location<'a>(&'a self, path: &'a Path, source: &'a str) -> (&'a Path, &'a str) {
        self.origin.as_ref().map_or((path, source), |(p, s)| (p.as_path(), s.as_str()))
    }

    pub fn render(&self, source_path: &Path, source: &str) -> String {
        let (source_path, source) = self.location(source_path, source);
        let (line, col) = line_col_of(source, self.span.start);
        let mut out = format!(
            "{}:{}:{}: error: {}",
            source_path.display(),
            line,
            col,
            self.message
        );
        if let Some(h) = &self.hint {
            out.push_str("\n  help: ");
            out.push_str(h);
        }
        out
    }
}

impl From<LexError> for Diagnostic {
    fn from(e: LexError) -> Self {
        Diagnostic {
            origin: None,
            span: e.span,
            message: e.kind.to_string(),
            hint: None,
            code: e.kind.code(),
        }
    }
}

impl From<ParseError> for Diagnostic {
    fn from(e: ParseError) -> Self {
        Diagnostic {
            origin: None,
            span: e.span,
            message: e.kind.to_string(),
            hint: None,
            code: e.kind.code(),
        }
    }
}

impl From<ResolveError> for Diagnostic {
    fn from(e: ResolveError) -> Self {
        Diagnostic {
            origin: None,
            span: e.span,
            message: e.kind.to_string(),
            hint: None,
            code: e.kind.code(),
        }
    }
}

impl From<TypeError> for Diagnostic {
    fn from(e: TypeError) -> Self {
        let hint = e.hint();
        let message = e.message();
        Diagnostic {
            origin: None,
            span: e.span,
            message,
            hint,
            code: e.kind.code(),
        }
    }
}

impl From<TypeWarning> for Diagnostic {
    fn from(w: TypeWarning) -> Self {
        Diagnostic {
            origin: None,
            span: w.span,
            message: w.kind.message(),
            hint: w.kind.hint(),
            code: w.kind.code(),
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}..{}] {}", self.span.start, self.span.end, self.message)?;
        if let Some(h) = &self.hint {
            write!(f, "\n  help: {h}")?;
        }
        Ok(())
    }
}

/// Convert a byte offset into 1-based (line, column) coordinates.
///
/// Columns count Unicode characters, not bytes. Lines split on `\n`.
pub(crate) fn line_col_of(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Render a list of diagnostics as one concatenated string plus a summary
/// line. Used by the CLI to print every error in one pass.
pub fn summarize_diagnostics(
    diags: &[Diagnostic],
    source_path: &Path,
    source: &str,
) -> String {
    let mut out = String::new();
    for d in diags {
        out.push_str(&d.render(source_path, source));
        out.push('\n');
    }
    out.push_str(&format!("\n{} error(s) found.\n", diags.len()));
    out
}

#[cfg(test)]
mod code_tests {
    /// Every diagnostic kind's code, from each `code()` match and each code
    /// the driver writes on a diagnostic it builds itself. A code names
    /// exactly one kind across all of them, so it can be searched for.
    #[test]
    fn every_diagnostic_code_is_well_formed_and_names_one_kind() {
        let sources = [
            ("lexer and parser", include_str!("../../corvid-syntax/src/errors.rs")),
            ("resolver", include_str!("../../corvid-resolve/src/errors.rs")),
            ("checker errors", include_str!("../../corvid-types/src/errors/error_kind.rs")),
            ("checker warnings", include_str!("../../corvid-types/src/errors/warning_kind.rs")),
        ];
        let mut seen = std::collections::BTreeMap::<String, String>::new();
        let mut record = |code: &str, owner: String| {
            let well_formed = code.len() == 5
                && (code.starts_with('E') || code.starts_with('W'))
                && code[1..].chars().all(|c| c.is_ascii_digit());
            assert!(well_formed, "{owner}: `{code}`");
            if let Some(other) = seen.insert(code.to_string(), owner.clone()) {
                panic!("{other} and {owner} share {code}");
            }
        };
        for (area, source) in sources {
            let mut rest = source;
            while let Some(start) = rest.find("pub fn code(&self)") {
                let body = &rest[start..];
                let body = &body[..body.find("\n    }\n").expect("end of code()")];
                for line in body.lines().filter(|line| line.contains("=> \"")) {
                    let kind = line.split_whitespace().next().unwrap();
                    record(line.split('"').nth(1).unwrap(), format!("{area} {kind}"));
                }
                rest = &rest[start + body.len()..];
            }
        }
        for (file, source) in [
            ("pipeline", include_str!("pipeline/mod.rs")),
            ("table_schema", include_str!("table_schema.rs")),
            ("build", include_str!("build/mod.rs")),
        ] {
            for (n, piece) in source.split("code: \"").skip(1).enumerate() {
                record(&piece[..5], format!("{file} diagnostic {n}"));
            }
        }
        assert!(seen.len() > 170, "read the codes: {}", seen.len());
    }
}
