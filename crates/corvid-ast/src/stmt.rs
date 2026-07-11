//! Statements — things that execute inside a block but don't produce a value.

use crate::expr::Expr;
use crate::span::{Ident, Span};
use crate::ty::TypeRef;
use serde::{Deserialize, Serialize};

/// A block: a sequence of statements that share a lexical scope.
/// Used for agent bodies, function bodies, and branches of `if`/`for`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// Any statement in Corvid source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Stmt {
    /// Variable binding: `order = get_order(id)` or `order: Order = ...`.
    Let {
        name: Ident,
        ty: Option<TypeRef>,
        value: Expr,
        span: Span,
    },

    /// Return from a function/agent: `return decision`.
    Return {
        value: Option<Expr>,
        span: Span,
    },

    /// Yield one element from a streaming agent body.
    Yield {
        value: Expr,
        span: Span,
    },

    /// Conditional: `if cond: ... else: ...`.
    If {
        cond: Expr,
        then_block: Block,
        else_block: Option<Block>,
        span: Span,
    },

    /// Iteration: `for item in items: ...`.
    For {
        var: Ident,
        iter: Expr,
        body: Block,
        span: Span,
    },

    /// Destructuring binding (slice 45n): `Decision { refund,
    /// amount, .. } = compute()`. The pattern must be IRREFUTABLE —
    /// shorthand field bindings, renamed bindings (`field: name`),
    /// and `..` only; literal or nested sub-patterns are rejected
    /// by the checker (use `match` for refutable shapes).
    Destructure {
        pattern: crate::expr::Pattern,
        value: Expr,
        span: Span,
    },

    /// Conditional loop: `while cond:` (slice 45k). The condition
    /// is re-evaluated before every iteration; `break`/`continue`
    /// apply to the innermost enclosing loop of either kind.
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },

    /// `break` — exit the innermost enclosing loop (slice 45k
    /// promoted this from a sentinel-`Ident` encoding to a real
    /// variant).
    Break { span: Span },

    /// `continue` — skip to the next iteration of the innermost
    /// enclosing loop.
    Continue { span: Span },

    /// `pass` — explicit no-op statement.
    Pass { span: Span },

    /// The approval gate — the core of Corvid's safety story.
    ///
    /// `approve Action(...)` must precede any `Irreversible` tool call
    /// in the same block whose signature matches `Action`.
    Approve { action: Expr, span: Span },

    /// An expression evaluated for its side effects: `issue_refund(...)`.
    Expr { expr: Expr, span: Span },

    /// Assignment through a place: `x.field = v`, `xs[i] = v`, and
    /// compound forms `x += v` / `x.field -= v` (slice 45b).
    ///
    /// `target` is restricted by the parser to an assignable place —
    /// an identifier, a field access, or an index expression. `op` is
    /// `Some` for compound assignment; the operator lives in the AST
    /// rather than being desugared into `target = target op value`,
    /// so an index expression with side effects never evaluates
    /// twice.
    ///
    /// Semantics are reference semantics: structs and lists are
    /// shared heap cells, so mutation through one binding is visible
    /// through every alias (matching the Phase 17 memory model).
    Assign {
        target: Expr,
        op: Option<crate::expr::BinaryOp>,
        value: Expr,
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Yield { span, .. }
            | Stmt::If { span, .. }
            | Stmt::For { span, .. }
            | Stmt::Destructure { span, .. }
            | Stmt::While { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::Pass { span }
            | Stmt::Approve { span, .. }
            | Stmt::Expr { span, .. }
            | Stmt::Assign { span, .. } => *span,
        }
    }
}
