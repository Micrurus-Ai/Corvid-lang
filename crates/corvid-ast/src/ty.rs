//! Type references as written in source.
//!
//! The AST stores `TypeRef` — the syntactic form of a type annotation.
//! The type checker later resolves these into fully-known structural types.

use crate::span::{Ident, Span};
use serde::{Deserialize, Serialize};

/// Effect rows attached to `Weak<T, {effects}>`.
///
/// These are not the same as tool declaration effects (`safe` /
/// `dangerous`). They describe which runtime actions invalidate the
/// compiler's proof that a weak reference is still refresh-valid at a
/// given program point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WeakEffect {
    ToolCall,
    Llm,
    Approve,
    Human,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WeakEffectRow {
    pub tool_call: bool,
    pub llm: bool,
    pub approve: bool,
    #[serde(default)]
    pub human: bool,
}

impl WeakEffectRow {
    pub const fn any() -> Self {
        Self {
            tool_call: true,
            llm: true,
            approve: true,
            human: true,
        }
    }

    pub const fn empty() -> Self {
        Self {
            tool_call: false,
            llm: false,
            approve: false,
            human: false,
        }
    }

    pub fn from_effects(effects: &[WeakEffect]) -> Self {
        let mut row = Self::empty();
        for effect in effects {
            match effect {
                WeakEffect::ToolCall => row.tool_call = true,
                WeakEffect::Llm => row.llm = true,
                WeakEffect::Approve => row.approve = true,
                WeakEffect::Human => row.human = true,
            }
        }
        row
    }

    pub fn effects(&self) -> Vec<WeakEffect> {
        let mut effects = Vec::new();
        if self.tool_call {
            effects.push(WeakEffect::ToolCall);
        }
        if self.llm {
            effects.push(WeakEffect::Llm);
        }
        if self.approve {
            effects.push(WeakEffect::Approve);
        }
        if self.human {
            effects.push(WeakEffect::Human);
        }
        effects
    }

    pub fn is_any(&self) -> bool {
        self.tool_call && self.llm && self.approve && self.human
    }
}

/// A type as the user wrote it in source code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeRef {
    /// A named type: `String`, `Ticket`, `Decision`.
    Named { name: Ident, span: Span },

    /// A qualified type resolved through an import alias:
    /// `policy.Receipt`, `types.Verdict`. The `alias` identifier
    /// must bind to a Corvid `.cor` file import; the resolver
    /// looks up `name` inside that imported module's exported
    /// symbol table. Introduced by `lang-cor-imports-basic-parse`;
    /// full resolution lands in `lang-cor-imports-basic-resolve`.
    Qualified {
        alias: Ident,
        name: Ident,
        span: Span,
    },

    /// A generic application: `List[Order]`, `Map[String, Int]`.
    Generic {
        name: Ident,
        args: Vec<TypeRef>,
        span: Span,
    },

    /// `Weak<T>` or `Weak<T, {tool_call, llm}>`.
    Weak {
        inner: Box<TypeRef>,
        effects: Option<WeakEffectRow>,
        span: Span,
    },

    /// A function type: `(Int, Int) -> Int`.
    Function {
        params: Vec<TypeRef>,
        ret: Box<TypeRef>,
        span: Span,
    },
}

impl TypeRef {
    pub fn span(&self) -> Span {
        match self {
            TypeRef::Named { span, .. }
            | TypeRef::Qualified { span, .. }
            | TypeRef::Generic { span, .. }
            | TypeRef::Weak { span, .. }
            | TypeRef::Function { span, .. } => *span,
        }
    }
}

/// A parameter to a function, tool, agent, or prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeRef,
    #[serde(default)]
    pub ownership: Option<OwnershipAnnotation>,
    pub span: Span,
}

/// Ownership contract on an FFI-visible boundary type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipAnnotation {
    pub mode: OwnershipMode,
    #[serde(default)]
    pub lifetime: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipMode {
    Owned,
    Borrowed,
    Shared,
    Static,
}

/// One `@ui(key: value)` presentation hint (slice 51d).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiHint {
    pub key: Ident,
    pub value: UiHintValue,
    pub span: Span,
}

/// The value of a `@ui` hint: a string, boolean, or integer literal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiHintValue {
    Str(String),
    Bool(bool),
    Int(i64),
}

/// A field in a struct-like type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeRef,
    /// Optional `@ui(...)` presentation hints (slice 51d). Purely
    /// advisory — a frontend MAY ignore them; it must never ignore
    /// the field's semantic constraints (type, refinement). Kept in
    /// a separate channel from constraints in the contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ui: Vec<UiHint>,
    /// Value refinement (slice 50j): `where between(0, 150)` on Int
    /// fields, `where len_between(1, 80)` on String fields. Checked
    /// at prompt-output decode, where a violation's message feeds
    /// the structured-output repair loop.
    #[serde(default)]
    pub refinement: Option<Refinement>,
    /// `@upload(...)` constraints (slice 51f) on a field typed
    /// `Upload<Format>`: accepted MIME, max size, and retention. A
    /// SEMANTIC constraint — the boundary rejects an upload that
    /// violates it — not a display hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<UploadSpec>,
    pub span: Span,
}

/// `@upload(max_mb: N, retention_days: N, mime: "a/b, c/d")` on an
/// `Upload<Format>` field (slice 51f). Any subset may be present; the
/// inner format tag supplies default accepted MIME when `mime` is
/// omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSpec {
    /// Maximum accepted size in bytes (from `max_mb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// Retention window before the stored upload is purged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u64>,
    /// Explicit accepted MIME types; overrides the format-tag default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mime: Vec<String>,
    pub span: Span,
}

/// Complete policy for approvals raised while executing one HTTP
/// route. Every field is source-declared: the runtime never invents
/// reviewer authority, risk, data classification, expiry, cost, or
/// reversibility.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalSpec {
    pub role: String,
    pub risk: String,
    pub data: String,
    pub expires_ms: u64,
    pub max_cost_usd: f64,
    pub irreversible: bool,
    pub span: Span,
}

/// A field value refinement (slice 50j). Deliberately small in v1:
/// integer ranges and string-length ranges cover the bulk of the
/// "structurally valid, semantically nonsense" decode failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refinement {
    /// `where between(min, max)` — inclusive Int range.
    Between { min: i64, max: i64 },
    /// `where len_between(min, max)` — inclusive String char-count range.
    LenBetween { min: u64, max: u64 },
}

impl Refinement {
    /// Render for diagnostics and repair feedback.
    pub fn describe(&self) -> String {
        match self {
            Self::Between { min, max } => format!("between({min}, {max})"),
            Self::LenBetween { min, max } => format!("len_between({min}, {max})"),
        }
    }
}
