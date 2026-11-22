//! Source locations and identifiers.

use serde::{Deserialize, Serialize};

/// A byte range in a source file.
///
/// Every AST node carries one so diagnostics can point at the exact
/// characters the user wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    /// Token range inside a compiler expansion sharing this source location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expansion: Option<(usize, usize)>,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            expansion: None,
        }
    }

    pub fn expanded(self, token: usize) -> Self {
        Self {
            expansion: Some((token, token)),
            ..self
        }
    }

    /// Merge two spans into one that covers both.
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            expansion: match (self.expansion, other.expansion) {
                (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
                (left, right) => left.or(right),
            },
        }
    }
}

/// An identifier — a name the user wrote in source, plus its location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
        }
    }
}
