//! Expressions — things that produce a value.

use crate::replay_expr::ReplayArm;
use crate::span::{Ident, Span};
use serde::{Deserialize, Serialize};

/// Any expression in Corvid source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    /// A constant: `42`, `"hello"`, `true`, `nothing`.
    Literal { value: Literal, span: Span },

    /// A bare identifier: `order`, `ticket`.
    Ident { name: Ident, span: Span },

    /// A call: `f(x, y)` or `StructName(field1, field2)`.
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },

    /// Field access: `ticket.order_id`.
    FieldAccess {
        target: Box<Expr>,
        field: Ident,
        span: Span,
    },

    /// Index access: `items[0]`, `map["key"]`.
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },

    /// Binary operator: `a + b`, `x == y`, `p and q`.
    BinOp {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },

    /// Unary operator: `-x`, `not x`.
    UnOp {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },

    /// List literal: `[1, 2, 3]`.
    List { items: Vec<Expr>, span: Span },

    /// `match` expression (slice 45i): scrutinee + one or more
    /// pattern arms. Exhaustiveness is checked by the typechecker
    /// over sum-type variants, Option, Result, and Bool; other
    /// scrutinee types require an irrefutable final arm.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        span: Span,
    },

    /// Map literal `{"a": 1, "b": 2}` (slice 45g). Entries are
    /// (key, value) expression pairs; empty `{}` is a valid empty
    /// map with element types inferred from context or Unknown.
    MapLiteral {
        entries: Vec<(Expr, Expr)>,
        span: Span,
    },

    /// Postfix propagation: `expr?`.
    TryPropagate {
        inner: Box<Expr>,
        span: Span,
    },

    /// Retry wrapper: `try expr on error retry N times backoff linear 100`.
    TryRetry {
        body: Box<Expr>,
        attempts: u64,
        backoff: Backoff,
        span: Span,
    },

    /// Replay block: `replay <trace>: when <pat> -> <expr> else <expr>`.
    /// Ingests a recorded JSONL trace and dispatches to the first
    /// matching arm, falling back to `else_body` when no `when`
    /// pattern matches. See [`crate::replay_expr`] for the arm + pattern
    /// shapes. Resolver + checker slices (21-inv-E-2, -3) refine
    /// the surface AST into a typed, exhaustiveness-checked form.
    Replay {
        trace: Box<Expr>,
        arms: Vec<ReplayArm>,
        else_body: Box<Expr>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal { span, .. }
            | Expr::Ident { span, .. }
            | Expr::Call { span, .. }
            | Expr::FieldAccess { span, .. }
            | Expr::Index { span, .. }
            | Expr::BinOp { span, .. }
            | Expr::UnOp { span, .. }
            | Expr::List { span, .. }
            | Expr::Match { span, .. }
            | Expr::MapLiteral { span, .. }
            | Expr::TryPropagate { span, .. }
            | Expr::TryRetry { span, .. }
            | Expr::Replay { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backoff {
    Linear(u64),
    Exponential(u64),
}

/// A constant value literal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nothing,
}

/// One arm of a `match` expression (slice 45i).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

/// A `match` pattern (slice 45i).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    /// `_` — matches anything, binds nothing.
    Wildcard { span: Span },
    /// Literal pattern: `0`, `"x"`, `true`, `-3`.
    Literal { value: Literal, span: Span },
    /// A bare name. The RESOLVER disambiguates: if it resolves to a
    /// sum-type unit variant (or builtin `None`), it is a variant
    /// pattern; otherwise it BINDS the scrutinee.
    Name { name: Ident, span: Span },
    /// `x @ pattern` — bind the whole value, then narrow.
    At {
        name: Ident,
        inner: Box<Pattern>,
        span: Span,
    },
    /// `Approved(p1, ...)` — sum variant with payload subpatterns;
    /// also `Some(p)`, `Ok(p)`, `Err(p)`.
    Variant {
        name: Ident,
        args: Vec<Pattern>,
        span: Span,
    },
    /// `Decision { refund: true, amount, .. }` — record destructure.
    /// A field without a subpattern is shorthand for binding the
    /// field's name; `rest` is the `..` marker.
    Record {
        name: Ident,
        fields: Vec<FieldPattern>,
        rest: bool,
        span: Span,
    },
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard { span }
            | Pattern::Literal { span, .. }
            | Pattern::Name { span, .. }
            | Pattern::At { span, .. }
            | Pattern::Variant { span, .. }
            | Pattern::Record { span, .. } => *span,
        }
    }
}

/// One field inside a record pattern (slice 45i).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldPattern {
    pub name: Ident,
    /// `None` is the shorthand form: `{ amount }` binds `amount`.
    pub pattern: Option<Pattern>,
    pub span: Span,
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,

    // Logical
    And,
    Or,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
}
