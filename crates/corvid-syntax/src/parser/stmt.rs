//! Statement + block parsing.
//!
//! Covers `parse_indented_block` (the `Indent … Dedent` block
//! shape used by every body — agent/prompt/eval/if/for) plus each
//! individual statement parser: return, yield, if, for, approve,
//! break/continue/pass, assign-or-expr, and bare-expr statements.
//!
//! Extracted from `parser.rs` as part of Phase 20i responsibility
//! decomposition. All methods operate on `Parser<'a>` state via
//! an additional `impl<'a> Parser<'a>` block.

use super::Parser;
use crate::errors::{ParseError, ParseErrorKind};
use crate::token::TokKind;
use corvid_ast::{BinaryOp, Block, Expr, Ident, Stmt};

impl<'a> Parser<'a> {
    // ------------------------------------------------------------
    // Block parsing.
    // ------------------------------------------------------------

    /// Expect `Indent`, then 1+ statements, then `Dedent`.
    pub(super) fn parse_indented_block(&mut self) -> Result<Block, ParseError> {
        let start_span = self.peek_span();
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: start_span,
            });
        }
        self.bump(); // consume Indent

        let mut stmts = Vec::new();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            match self.parse_stmt() {
                Ok(s) => stmts.push(s),
                Err(e) => {
                    self.errors.push(e);
                    self.sync_to_statement_boundary();
                }
            }
        }
        let end_span = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }

        if stmts.is_empty() {
            self.errors.push(ParseError {
                kind: ParseErrorKind::EmptyBlock,
                span: start_span,
            });
        }

        Ok(Block {
            stmts,
            span: start_span.merge(end_span),
        })
    }

    // ------------------------------------------------------------
    // Statement parsing.
    // ------------------------------------------------------------

    pub(super) fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        // Skip stray newlines (blank lines inside blocks).
        while matches!(self.peek(), TokKind::Newline) {
            self.bump();
        }

        match self.peek() {
            TokKind::KwReturn => self.parse_return_stmt(),
            TokKind::KwYield => self.parse_yield_stmt(),
            TokKind::KwIf => self.parse_if_stmt(),
            TokKind::KwFor => self.parse_for_stmt(),
            TokKind::KwWhile => self.parse_while_stmt(),
            TokKind::KwApprove => self.parse_approve_stmt(),
            TokKind::KwBreak => self.parse_loop_flow_stmt(),
            TokKind::KwContinue => self.parse_loop_flow_stmt(),
            TokKind::KwPass => self.parse_loop_flow_stmt(),
            TokKind::Ident(_) => self.parse_assign_or_expr_stmt(),
            _ => self.parse_expr_stmt(),
        }
    }

    fn parse_return_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // return
        let value = if matches!(self.peek(), TokKind::Newline | TokKind::Eof) {
            None
        } else {
            Some(self.parse_expr()?)
        };
        let end = self.peek_span();
        self.expect_newline()?;
        Ok(Stmt::Return {
            value,
            span: start.merge(end),
        })
    }

    fn parse_yield_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // yield
        let value = self.parse_expr()?;
        let end = value.span();
        self.expect_newline()?;
        Ok(Stmt::Yield {
            value,
            span: start.merge(end),
        })
    }

    fn parse_if_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // if
        let cond = self.parse_expr()?;
        self.expect(TokKind::Colon, "`:` after `if` condition")?;
        self.expect_newline()?;
        let then_block = self.parse_indented_block()?;
        let else_block = if matches!(self.peek(), TokKind::KwElse) {
            self.bump();
            self.expect(TokKind::Colon, "`:` after `else`")?;
            self.expect_newline()?;
            Some(self.parse_indented_block()?)
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map(|b| b.span)
            .unwrap_or(then_block.span);
        Ok(Stmt::If {
            cond,
            then_block,
            else_block,
            span: start.merge(end),
        })
    }

    fn parse_for_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // for
        let (var_name, var_span) = self.expect_ident()?;
        self.expect(TokKind::KwIn, "`in` in `for` loop")?;
        let iter = self.parse_expr()?;
        self.expect(TokKind::Colon, "`:` after `for` clause")?;
        self.expect_newline()?;
        let body = self.parse_indented_block()?;
        let end = body.span;
        Ok(Stmt::For {
            var: Ident::new(var_name, var_span),
            iter,
            body,
            span: start.merge(end),
        })
    }

    fn parse_approve_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // approve
        let action = self.parse_expr()?;
        let end = action.span();
        self.expect_newline()?;
        Ok(Stmt::Approve {
            action,
            span: start.merge(end),
        })
    }

    /// `while cond:` — conditional loop (slice 45k).
    fn parse_while_stmt(&mut self) -> Result<Stmt, ParseError> {
        let start = self.peek_span();
        self.bump(); // while
        let cond = self.parse_expr()?;
        self.expect(TokKind::Colon, "`:` after `while` condition")?;
        self.expect_newline()?;
        let body = self.parse_indented_block()?;
        let end = body.span;
        Ok(Stmt::While {
            cond,
            body,
            span: start.merge(end),
        })
    }

    /// `break`, `continue`, and `pass` — single keyword + newline,
    /// each a real AST variant (promoted from the former
    /// sentinel-`Ident` encoding in slice 45k).
    fn parse_loop_flow_stmt(&mut self) -> Result<Stmt, ParseError> {
        let span = self.peek_span();
        let kw = self.peek().clone();
        self.bump();
        self.expect_newline()?;
        Ok(match kw {
            TokKind::KwBreak => Stmt::Break { span },
            TokKind::KwContinue => Stmt::Continue { span },
            TokKind::KwPass => Stmt::Pass { span },
            _ => unreachable!("dispatched on break/continue/pass only"),
        })
    }

    /// `IDENT '=' expr NEWLINE` is an assignment and
    /// `IDENT ':' type_ref '=' expr NEWLINE` is an annotated
    /// assignment — the same `name: Type` shape fields, params, and
    /// effect dimensions use. Anything else is an expression
    /// statement.
    fn parse_assign_or_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        // Peek two ahead: IDENT then `=` ? → assignment.
        if matches!(self.peek(), TokKind::Ident(_))
            && matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(TokKind::Assign)
            )
        {
            let start = self.peek_span();
            let (name, name_span) = self.expect_ident()?;
            self.bump(); // =
            let value = self.parse_expr()?;
            let end = value.span();
            self.expect_newline()?;
            return Ok(Stmt::Let {
                name: Ident::new(name, name_span),
                ty: None,
                value,
                span: start.merge(end),
            });
        }
        // IDENT then `:` (but not `::`) ? → annotated assignment.
        // The double-colon exclusion matters: path-call expression
        // statements (`Weak::upgrade(w)`) begin `IDENT ':' ':'`, so
        // the annotation lookahead must require exactly one colon.
        if matches!(self.peek(), TokKind::Ident(_))
            && matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(TokKind::Colon)
            )
            && !matches!(
                self.tokens.get(self.pos + 2).map(|t| &t.kind),
                Some(TokKind::Colon)
            )
        {
            let start = self.peek_span();
            let (name, name_span) = self.expect_ident()?;
            self.bump(); // :
            let ty = self.parse_type_ref()?;
            self.expect(
                TokKind::Assign,
                "`=` after the type annotation in an annotated assignment",
            )?;
            let value = self.parse_expr()?;
            let end = value.span();
            self.expect_newline()?;
            return Ok(Stmt::Let {
                name: Ident::new(name, name_span),
                ty: Some(ty),
                value,
                span: start.merge(end),
            });
        }
        self.parse_expr_stmt()
    }

    fn parse_expr_stmt(&mut self) -> Result<Stmt, ParseError> {
        let expr = self.parse_expr()?;

        // Destructuring binding (slice 45n): a struct literal
        // followed by `=` is a PATTERN — `Decision { refund, .. } =
        // compute()`. Reinterpret the parsed literal; only bare
        // names, `field: name` renames, and `..` survive the
        // conversion (anything refutable belongs in `match`).
        if matches!(self.peek(), TokKind::Assign) {
            if let Expr::StructLiteral { .. } = &expr {
                self.bump(); // =
                let pattern = struct_literal_to_pattern(expr)?;
                let value = self.parse_expr()?;
                let end = value.span();
                self.expect_newline()?;
                let span = pattern.span().merge(end);
                return Ok(Stmt::Destructure {
                    pattern,
                    value,
                    span,
                });
            }
        }

        // Slice 45b — place assignment. If the parsed expression is
        // followed by `=` or a compound-assignment operator, it is an
        // assignment target rather than an expression statement.
        let op = match self.peek() {
            TokKind::Assign => Some(None),
            TokKind::PlusEq => Some(Some(BinaryOp::Add)),
            TokKind::MinusEq => Some(Some(BinaryOp::Sub)),
            TokKind::StarEq => Some(Some(BinaryOp::Mul)),
            TokKind::SlashEq => Some(Some(BinaryOp::Div)),
            TokKind::PercentEq => Some(Some(BinaryOp::Mod)),
            _ => None,
        };
        if let Some(op) = op {
            if !is_assignable_place(&expr) {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "an assignment to a non-place expression".into(),
                        expected: "an assignable place: a variable, a field access \
                                   (`x.field`), or an index (`xs[i]`)"
                            .into(),
                    },
                    span: expr.span(),
                });
            }
            let start = expr.span();
            self.bump(); // the assignment operator
            let value = self.parse_expr()?;
            let end = value.span();
            self.expect_newline()?;
            return Ok(Stmt::Assign {
                target: expr,
                op,
                value,
                span: start.merge(end),
            });
        }

        let span = expr.span();
        self.expect_newline()?;
        Ok(Stmt::Expr { expr, span })
    }
}

/// An assignable place is a variable, a field access, or an index
/// expression. Anything else (calls, literals, `?`-propagation, …)
/// cannot be assigned through.
/// Convert a parsed struct literal into an irrefutable
/// destructuring pattern (slice 45n). Shorthand fields bind the
/// field name; `field: name` renames; `..` marks the rest. Any
/// other field value or a `..base` spread is a parse error here —
/// refutable destructuring belongs in `match`.
fn struct_literal_to_pattern(expr: Expr) -> Result<corvid_ast::Pattern, ParseError> {
    use corvid_ast::{FieldPattern, Pattern};
    let Expr::StructLiteral {
        name,
        fields,
        spread,
        rest,
        span,
    } = expr
    else {
        unreachable!("caller matched StructLiteral");
    };
    if let Some(spread) = spread {
        return Err(ParseError {
            kind: ParseErrorKind::UnexpectedToken {
                got: "`..base` spread in a destructuring pattern".into(),
                expected: "a bare `..` (destructuring ignores remaining fields; it cannot source them)"
                    .into(),
            },
            span: spread.span(),
        });
    }
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        let sub = match f.value {
            None => None, // shorthand binds the field name
            Some(Expr::Ident { name: bind, span }) => Some(Pattern::Name {
                name: bind,
                span,
            }),
            Some(other) => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "an expression in a destructuring field".into(),
                        expected: "a bare binding name (`field: new_name`) — refutable patterns belong in `match`"
                            .into(),
                    },
                    span: other.span(),
                });
            }
        };
        out.push(FieldPattern {
            name: f.name,
            pattern: sub,
            span: f.span,
        });
    }
    Ok(Pattern::Record {
        name,
        fields: out,
        rest,
        span,
    })
}

fn is_assignable_place(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident { .. } | Expr::FieldAccess { .. } | Expr::Index { .. }
    )
}
