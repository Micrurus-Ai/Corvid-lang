//! Expression parsing — Pratt-style precedence climbing.
//!
//! Top-down flow: parse_expr -> parse_or -> parse_and -> parse_not
//! -> parse_cmp -> parse_add -> parse_mul -> parse_unary ->
//! parse_postfix -> parse_primary (+ parse_try_retry_expr on `?`
//! and `try` forms). parse_u64_literal is a small shared helper
//! used by both expression parsing and statement-level count
//! literals.
//!
//! Extracted from `parser.rs` as part of Phase 20i responsibility
//! decomposition.

use super::{describe_token, Parser};
use crate::errors::{ParseError, ParseErrorKind};
use crate::token::TokKind;
use corvid_ast::{
    Backoff, BinaryOp, Expr, Ident, LambdaParam, Literal, Span, StructLiteralField, UnaryOp,
};

impl<'a> Parser<'a> {
    // ------------------------------------------------------------
    // Expression entry point.
    // ------------------------------------------------------------

    pub(super) fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    /// At a value's line boundary, a named callee can take its list argument
    /// as indented child expressions. Control-flow headers retain their colon.
    pub(super) fn parse_value_line(&mut self) -> Result<Expr, ParseError> {
        let callee = self.parse_expr()?;
        if self.block_expr_terminated || !matches!(self.peek(), TokKind::Colon)
            || !matches!(callee, Expr::Ident { .. } | Expr::FieldAccess { .. })
        {
            return Ok(callee);
        }
        let children = self.parse_child_list()?;
        let span = callee.span().merge(children.span());
        Ok(Expr::Call { callee: Box::new(callee), args: vec![children], span })
    }

    pub(super) fn parse_child_list(&mut self) -> Result<Expr, ParseError> {
        let start = self.expect(TokKind::Colon, ": before child views")?;
        self.expect_newline()?;
        self.expect(TokKind::Indent, "indented child expressions")?;
        let mut items = Vec::new();
        self.skip_newlines();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            let item = if matches!(self.peek(), TokKind::KwIf) {
                self.parse_conditional_child()?
            } else if matches!(self.peek(), TokKind::KwFor) {
                self.parse_keyed_children()?
            } else {
                self.parse_value_line()?
            };
            items.push(item);
            self.expect_newline()?;
            self.skip_newlines();
        }
        if items.is_empty() {
            return Err(ParseError { kind: ParseErrorKind::EmptyBlock, span: start });
        }
        let end = items.last().expect("nonempty children").span();
        self.expect(TokKind::Dedent, "end of child block")?;
        self.block_expr_terminated = true;
        Ok(Expr::List { items, span: start.merge(end) })
    }

    fn field_key_builtin(&self, owner: &Expr) -> &'static str {
        if matches!(owner, Expr::Ident { name, .. } if self.row_bindings.iter().any(|row| row.name == name.name)) {
            "__browser_row_key"
        } else {
            "__browser_field_key"
        }
    }

    fn parse_keyed_children(&mut self) -> Result<Expr, ParseError> {
        let start = self.bump().span;
        let (row, row_span) = if matches!(self.peek(), TokKind::KwAgent) {
            ("agent".to_owned(), self.bump().span)
        } else {
            self.expect_ident()?
        };
        self.expect(TokKind::KwIn, "in before the list of records")?;
        let source = self.parse_expr()?;
        if matches!(self.peek(), TokKind::Colon) {
            return self.parse_live_children(row, source, start);
        }
        self.expect_contextual_ident("by")?;
        if row == "agent" {
            self.expect(TokKind::KwAgent, "the current agent row")?;
        } else {
            self.expect_contextual_ident(&row)?;
        }
        self.expect(TokKind::Dot, "a key field on the current row")?;
        let (key, key_span) = self.expect_ident()?;
        self.row_bindings.push(super::RowBinding { name: row.clone(), live: None });
        let children = self.parse_child_list();
        self.row_bindings.pop();
        let children = children?;
        let end = children.span();
        let alias = format!("__rows_ui_{}", start.start);
        let mut tokens = crate::lex(&format!("import \"std:ui\" as {alias}\n")).expect("UI import");
        for (i, token) in tokens.iter_mut().enumerate() {
            token.span = start.expanded(i);
        }
        self.generated.push(Parser::new(&tokens).parse_decl()?);
        let ident = |name: &str, span| Expr::Ident { name: Ident::new(name, span), span };
        let string = |value, span| Expr::Literal { value: Literal::String(value), span };
        let method = |target, name: &str, args, span| Expr::Call {
            callee: Box::new(Expr::FieldAccess {
                target: Box::new(target), field: Ident::new(name, span), span
            }), args, span
        };
        let key_value = Expr::FieldAccess {
            target: Box::new(ident(&row, start.expanded(20))),
            field: Ident::new(&key, key_span), span: key_span,
        };
        let fields = super::authoring::call("__browser_fields", vec![ident(&row, start.expanded(21))], start.expanded(22));
        let item = method(ident(&alias, start.expanded(23)), "browser_row",
            vec![key_value, string(key, start.expanded(24)), fields, children], start.expanded(25));
        let render = Expr::Lambda {
            params: vec![LambdaParam { name: Ident::new(row.clone(), row_span), ty: None, span: row_span }],
            body: Box::new(item), span: start.expanded(26),
        };
        let rows = method(source, "map", vec![render], start.expanded(27));
        Ok(method(ident(&alias, start.expanded(28)), "browser_rows",
            vec![string(row, start.expanded(29)), rows], start.merge(end)))
    }

    fn parse_browser_field_path(&mut self) -> Result<(Expr, Expr), ParseError> {
        let (owner, owner_span) = if matches!(self.peek(), TokKind::KwAgent)
            && self.row_bindings.iter().any(|row| row.name == "agent") {
            ("agent".to_owned(), self.bump().span)
        } else {
            self.expect_ident()?
        };
        self.expect(TokKind::Dot, "a field on a browser state binding")?;
        let (mut field, mut field_span) = self.expect_ident()?;
        while matches!(self.peek(), TokKind::Dot) {
            self.bump();
            let (part, end) = self.expect_ident()?;
            field.push('.');
            field.push_str(&part);
            field_span = field_span.merge(end);
        }
        let owner = Expr::Ident { name: Ident::new(owner, owner_span), span: owner_span };
        let key = Expr::Literal { value: Literal::String(field), span: field_span };
        Ok((owner, key))
    }

    fn parse_conditional_child(&mut self) -> Result<Expr, ParseError> {
        let start = self.bump().span;
        let (owner, field) = self.parse_browser_field_path()?;
        let (helper_name, expected) = if matches!(self.peek(), TokKind::Colon) {
            ("boolean_children", Expr::Literal { value: Literal::Bool(true), span: field.span() })
        } else {
            self.expect(TokKind::Eq, "a colon after a boolean field or == before a named choice")?;
            let (variant, span) = self.expect_ident()?;
            ("choice_children", Expr::Literal { value: Literal::String(variant), span })
        };
        let condition = super::authoring::call(self.field_key_builtin(&owner),
            vec![owner, field, expected.clone()], start.merge(expected.span()));
        let mut args = vec![condition];
        if matches!(expected, Expr::Literal { value: Literal::String(_), .. }) {
            args.push(expected);
        }
        let children = self.parse_child_list()?;
        let helper = format!("__browser_case_{}", start.start);
        let import = format!("import \"std:ui\" as {helper}_ui use {helper_name} as {helper}\n");
        let mut tokens = crate::lex(&import).expect("valid conditional view import");
        for (index, token) in tokens.iter_mut().enumerate() {
            token.span = start.expanded(index);
        }
        self.generated.push(Parser::new(&tokens).parse_decl()?);
        let span = start.merge(children.span());
        args.push(children);
        Ok(super::authoring::call(&helper, args, span))
    }

    // or_expr := and_expr ('or' and_expr)*
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), TokKind::KwOr) {
            self.bump();
            let right = self.parse_and()?;
            let span = left.span().merge(right.span());
            left = Expr::BinOp {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    // and_expr := not_expr ('and' not_expr)*
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_not()?;
        while matches!(self.peek(), TokKind::KwAnd) {
            self.bump();
            let right = self.parse_not()?;
            let span = left.span().merge(right.span());
            left = Expr::BinOp {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    // not_expr := 'not' not_expr | cmp_expr
    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek(), TokKind::KwNot) {
            let start = self.peek_span().start;
            self.bump();
            let operand = self.parse_not()?;
            let span = Span::new(start, operand.span().end);
            Ok(Expr::UnOp {
                op: UnaryOp::Not,
                operand: Box::new(operand),
                span,
            })
        } else {
            self.parse_cmp()
        }
    }

    // cmp_expr := add_expr (cmp_op add_expr)?
    // chained comparisons (a < b < c) are explicitly rejected.
    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_add()?;
        let op = match self.peek() {
            TokKind::Eq => Some(BinaryOp::Eq),
            TokKind::NotEq => Some(BinaryOp::NotEq),
            TokKind::Lt => Some(BinaryOp::Lt),
            TokKind::LtEq => Some(BinaryOp::LtEq),
            TokKind::Gt => Some(BinaryOp::Gt),
            TokKind::GtEq => Some(BinaryOp::GtEq),
            _ => None,
        };
        let Some(op) = op else { return Ok(left) };
        self.bump();
        let right = self.parse_add()?;

        // Reject a second comparison operator.
        if matches!(
            self.peek(),
            TokKind::Eq
                | TokKind::NotEq
                | TokKind::Lt
                | TokKind::LtEq
                | TokKind::Gt
                | TokKind::GtEq
        ) {
            return Err(ParseError {
                kind: ParseErrorKind::ChainedComparison,
                span: self.peek_span(),
            });
        }

        let span = left.span().merge(right.span());
        Ok(Expr::BinOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
            span,
        })
    }

    // add_expr := mul_expr (('+' | '-') mul_expr)*
    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                TokKind::Plus => BinaryOp::Add,
                TokKind::Minus => BinaryOp::Sub,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_mul()?;
            let span = left.span().merge(right.span());
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
    }

    // mul_expr := unary_expr (('*' | '/' | '%') unary_expr)*
    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                TokKind::Star => BinaryOp::Mul,
                TokKind::Slash => BinaryOp::Div,
                TokKind::Percent => BinaryOp::Mod,
                _ => return Ok(left),
            };
            self.bump();
            let right = self.parse_unary()?;
            let span = left.span().merge(right.span());
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
    }

    // unary_expr := ('-' | '+') unary_expr | postfix_expr
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        let op = match self.peek() {
            TokKind::Minus => Some(UnaryOp::Neg),
            TokKind::Plus => Some(UnaryOp::Pos),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.peek_span().start;
            self.bump();
            let operand = self.parse_unary()?;
            let span = Span::new(start, operand.span().end);
            Ok(Expr::UnOp {
                op,
                operand: Box::new(operand),
                span,
            })
        } else {
            self.parse_postfix()
        }
    }

    // postfix_expr := primary (postfix_op)*
    // postfix_op   := '.' IDENT | '[' expr ']' | '(' args? ')'
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut target = self.parse_primary()?;
        loop {
            match self.peek() {
                TokKind::Dot => {
                    self.bump();
                    let (field_name, field_span) = self.expect_ident()?;
                    let span = target.span().merge(field_span);
                    target = Expr::FieldAccess {
                        target: Box::new(target),
                        field: Ident::new(field_name, field_span),
                        span,
                    };
                }
                TokKind::LBracket => {
                    self.bump();
                    let idx = self.parse_expr()?;
                    let end_span = self.peek_span();
                    if !matches!(self.peek(), TokKind::RBracket) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnclosedBracket,
                            span: end_span,
                        });
                    }
                    self.bump();
                    let span = target.span().merge(end_span);
                    target = Expr::Index {
                        target: Box::new(target),
                        index: Box::new(idx),
                        span,
                    };
                }
                TokKind::LParen => {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), TokKind::RParen) {
                        args.push(self.parse_expr()?);
                        while matches!(self.peek(), TokKind::Comma) {
                            self.bump();
                            // Allow trailing comma.
                            if matches!(self.peek(), TokKind::RParen) {
                                break;
                            }
                            args.push(self.parse_expr()?);
                        }
                    }
                    let end_span = self.peek_span();
                    if !matches!(self.peek(), TokKind::RParen) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnclosedParen,
                            span: end_span,
                        });
                    }
                    self.bump();
                    let span = target.span().merge(end_span);
                    target = Expr::Call {
                        callee: Box::new(target),
                        args,
                        span,
                    };
                }
                TokKind::Question => {
                    let question_span = self.peek_span();
                    let target_span = target.span();
                    self.bump();
                    target = Expr::TryPropagate {
                        inner: Box::new(target),
                        span: target_span.merge(question_span),
                    };
                }
                _ => return Ok(target),
            }
        }
    }

    /// A number just read, followed by a unit word, is a duration
    /// literal (B06): `30 seconds`, `1.5 hours`. A number followed by a
    /// word was a parse error before, so no program changes meaning.
    fn duration_literal(&mut self, count: String, start: corvid_ast::Span) -> Option<Expr> {
        let TokKind::Ident(word) = self.peek() else { return None };
        corvid_ast::duration_unit(word)?;
        let unit = word.clone();
        self.bump();
        Some(Expr::Literal {
            value: Literal::Duration { count, unit },
            span: start.merge(self.prev_span()),
        })
    }

    // primary := literal | IDENT | '(' expr ')' | '[' items? ']'
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let start_span = self.peek_span();
        match self.peek().clone() {
            TokKind::Int(n) => {
                self.bump();
                if let Some(duration) = self.duration_literal(n.to_string(), start_span) {
                    return Ok(duration);
                }
                Ok(Expr::Literal {
                    value: Literal::Int(n),
                    span: start_span,
                })
            }
            TokKind::Float { value: f, text } => {
                self.bump();
                if let Some(duration) = self.duration_literal(text.clone(), start_span) {
                    return Ok(duration);
                }
                Ok(Expr::Literal {
                    value: Literal::Float { value: f, text },
                    span: start_span,
                })
            }
            TokKind::StringLit(s) => {
                self.bump();
                Ok(Expr::Literal {
                    value: Literal::String(s),
                    span: start_span,
                })
            }
            TokKind::KwTrue => {
                self.bump();
                Ok(Expr::Literal {
                    value: Literal::Bool(true),
                    span: start_span,
                })
            }
            TokKind::KwFalse => {
                self.bump();
                Ok(Expr::Literal {
                    value: Literal::Bool(false),
                    span: start_span,
                })
            }
            TokKind::KwNothing => {
                self.bump();
                Ok(Expr::Literal {
                    value: Literal::Nothing,
                    span: start_span,
                })
            }
            TokKind::KwTry => self.parse_try_retry_expr(),
            // Trust boundary (slice 50i): `trusted(expr)`. Contextual
            // — only when the identifier `trusted` is immediately
            // called; a plain `trusted` identifier stays a binding.
            TokKind::Ident(w) if w == "bind" && matches!(self.peek_ahead(1), TokKind::Ident(_) | TokKind::KwAgent) => {
                self.bump();
                let (owner, key) = self.parse_browser_field_path()?;
                let field_span = key.span();
                if let Expr::Ident { name, .. } = &owner {
                    let binding = self.row_bindings.iter().rev().find(|row| row.name == name.name);
                    if let Some((server, live)) = binding.and_then(|row| row.live.as_ref()) {
                        return Ok(super::authoring::call("__live_field",
                            vec![server.clone(), live.clone(), key], start_span.merge(field_span)));
                    }
                }
                Ok(super::authoring::call(self.field_key_builtin(&owner), vec![owner, key], start_span.merge(field_span)))
            }
            TokKind::Ident(w) if w == "ask" && matches!(self.peek_ahead(1), TokKind::Ident(_)) => {
                self.parse_bounded_ask()
            }
            TokKind::Ident(w) if w == "trusted" && matches!(self.peek_ahead(1), TokKind::LParen) => {
                let start = self.peek_span();
                self.bump(); // trusted
                self.bump(); // (
                let inner = self.parse_expr()?;
                self.expect(TokKind::RParen, "`)` after trusted(...)")?;
                Ok(Expr::TrustBoundary {
                    inner: Box::new(inner),
                    span: start.merge(self.prev_span()),
                })
            }
            TokKind::KwAgent if self.row_bindings.iter().any(|row| row.name == "agent") => {
                let span = self.bump().span;
                Ok(Expr::Ident { name: Ident::new("agent", span), span })
            }
            TokKind::KwReplay => self.parse_replay_expr(),
            TokKind::KwMatch => self.parse_match_expr(),
            TokKind::KwFn => self.parse_lambda_expr(),
            TokKind::Ident(name) => {
                self.bump();
                let name = self.parse_namespaced_ident_from(name)?;
                // Named struct literal (45n): `Decision { ... }`.
                if matches!(self.peek(), TokKind::LBrace) {
                    return self.parse_struct_literal(Ident::new(name, start_span));
                }
                Ok(Expr::Ident {
                    name: Ident::new(name, start_span),
                    span: start_span,
                })
            }
            TokKind::LParen => {
                self.bump();
                let inner = self.parse_expr()?;
                let end_span = self.peek_span();
                if !matches!(self.peek(), TokKind::RParen) {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedParen,
                        span: end_span,
                    });
                }
                self.bump();
                Ok(inner)
            }
            TokKind::LBracket => {
                self.bump();
                let mut items = Vec::new();
                if !matches!(self.peek(), TokKind::RBracket) {
                    items.push(self.parse_expr()?);
                    while matches!(self.peek(), TokKind::Comma) {
                        self.bump();
                        if matches!(self.peek(), TokKind::RBracket) {
                            break;
                        }
                        items.push(self.parse_expr()?);
                    }
                }
                let end_span = self.peek_span();
                if !matches!(self.peek(), TokKind::RBracket) {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedBracket,
                        span: end_span,
                    });
                }
                self.bump();
                let span = start_span.merge(end_span);
                Ok(Expr::List { items, span })
            }
            // Map literal `{"a": 1}` (slice 45g). `{}` is the empty
            // map; entries are `expr ':' expr`, comma-separated with
            // an optional trailing comma.
            TokKind::LBrace => {
                self.bump();
                let mut entries = Vec::new();
                if !matches!(self.peek(), TokKind::RBrace) {
                    loop {
                        let key = self.parse_expr()?;
                        self.expect(TokKind::Colon, "`:` between map key and value")?;
                        let value = self.parse_expr()?;
                        entries.push((key, value));
                        if !matches!(self.peek(), TokKind::Comma) {
                            break;
                        }
                        self.bump();
                        if matches!(self.peek(), TokKind::RBrace) {
                            break;
                        }
                    }
                }
                let end_span = self.peek_span();
                if !matches!(self.peek(), TokKind::RBrace) {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnclosedBracket,
                        span: end_span,
                    });
                }
                self.bump();
                let span = start_span.merge(end_span);
                Ok(Expr::MapLiteral { entries, span })
            }
            TokKind::Eof | TokKind::Newline | TokKind::Indent | TokKind::Dedent => {
                Err(ParseError {
                    kind: ParseErrorKind::UnexpectedEof,
                    span: start_span,
                })
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: "an expression".into(),
                },
                span: start_span,
            }),
        }
    }

    /// `Decision { refund: true, amount, ..base }` — named struct
    /// literal (slice 45n). A bare `..` (no expression) sets `rest`
    /// and is only legal when the whole form is reinterpreted as a
    /// destructuring pattern by the statement parser.
    fn parse_struct_literal(&mut self, name: Ident) -> Result<Expr, ParseError> {
        let start = name.span;
        self.bump(); // {
        let mut fields: Vec<StructLiteralField> = Vec::new();
        let mut spread: Option<Box<Expr>> = None;
        let mut rest = false;
        loop {
            if matches!(self.peek(), TokKind::RBrace) {
                break;
            }
            // `..` — either a bare rest marker or `..base` spread.
            if matches!(self.peek(), TokKind::Dot) {
                self.bump();
                self.expect(TokKind::Dot, "`..` in a struct literal")?;
                if matches!(self.peek(), TokKind::RBrace | TokKind::Comma) {
                    rest = true;
                } else {
                    spread = Some(Box::new(self.parse_expr()?));
                }
                // Spread/rest must be LAST.
                if matches!(self.peek(), TokKind::Comma) {
                    self.bump();
                }
                if !matches!(self.peek(), TokKind::RBrace) {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: describe_token(self.peek()),
                            expected: "`}` — `..` must be the LAST entry in a struct literal"
                                .into(),
                        },
                        span: self.peek_span(),
                    });
                }
                break;
            }
            let fstart = self.peek_span();
            let (fname, fname_span) = self.expect_ident()?;
            let value = if matches!(self.peek(), TokKind::Colon) {
                self.bump();
                Some(self.parse_expr()?)
            } else {
                None // shorthand: reads the local of the same name
            };
            fields.push(StructLiteralField {
                name: Ident::new(fname, fname_span),
                value,
                span: fstart.merge(self.prev_span()),
            });
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.peek_span();
        self.expect(TokKind::RBrace, "`}` closing the struct literal")?;
        Ok(Expr::StructLiteral {
            name,
            fields,
            spread,
            rest,
            span: start.merge(end),
        })
    }

    /// `fn (x, y) -> expr` — lambda expression (slice 45j).
    /// Parameters may carry optional annotations: `fn (x: Int) -> x + 1`.
    fn parse_lambda_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        self.bump(); // `fn`
        self.expect(TokKind::LParen, "`(` after `fn` in a lambda")?;
        let mut params: Vec<LambdaParam> = Vec::new();
        if !matches!(self.peek(), TokKind::RParen) {
            loop {
                let pstart = self.peek_span();
                let (pname, pname_span) = self.expect_ident()?;
                let ty = if matches!(self.peek(), TokKind::Colon) {
                    self.bump();
                    Some(self.parse_type_ref()?)
                } else {
                    None
                };
                params.push(LambdaParam {
                    name: Ident::new(pname, pname_span),
                    ty,
                    span: pstart.merge(self.prev_span()),
                });
                if matches!(self.peek(), TokKind::Comma) {
                    self.bump();
                    if matches!(self.peek(), TokKind::RParen) {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        self.expect(TokKind::RParen, "`)` after lambda parameters")?;
        self.expect(TokKind::Arrow, "`->` before the lambda body")?;
        let body = self.parse_expr()?;
        let span = start.merge(body.span());
        Ok(Expr::Lambda {
            params,
            body: Box::new(body),
            span,
        })
    }

    fn parse_try_retry_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        self.bump(); // try
        let body = self.parse_expr()?;

        // Optional per-attempt timeout (slice 50k): contextual
        // `timeout <ms>` — `timeout` stays an ordinary identifier
        // everywhere else.
        let timeout_ms = if matches!(self.peek(), TokKind::Ident(w) if w == "timeout") {
            self.bump();
            Some(self.parse_u64_literal("timeout bound in ms")?)
        } else {
            None
        };

        // The retry clause is optional when a timeout is present:
        // `try slow_call() timeout 2000` stands alone.
        if !matches!(self.peek(), TokKind::KwOn) {
            let Some(timeout_ms) = timeout_ms else {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "a bare `try` with neither clause".into(),
                        expected: "`timeout <ms>`, `on error retry ...`, or both".into(),
                    },
                    span: self.peek_span(),
                });
            };
            return Ok(Expr::TryRetry {
                body: Box::new(body),
                attempts: 0,
                backoff: Backoff::Linear(0),
                timeout_ms: Some(timeout_ms),
                span: start.merge(self.prev_span()),
            });
        }
        self.expect(TokKind::KwOn, "`on` after `try` body")?;
        self.expect(TokKind::KwError, "`error` after `on` in retry expression")?;
        self.expect(TokKind::KwRetry, "`retry` in retry expression")?;
        let attempts = self.parse_u64_literal("retry attempt count")?;
        self.expect(TokKind::KwTimes, "`times` after retry count")?;
        self.expect(TokKind::KwBackoff, "`backoff` after retry count")?;

        let backoff = match self.peek() {
            TokKind::KwLinear => {
                self.bump();
                Backoff::Linear(self.parse_u64_literal("linear backoff delay in ms")?)
            }
            TokKind::KwExponential => {
                self.bump();
                Backoff::Exponential(
                    self.parse_u64_literal("exponential backoff base delay in ms")?,
                )
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(other),
                        expected: "`linear <ms>` or `exponential <ms>`".into(),
                    },
                    span: self.peek_span(),
                });
            }
        };

        Ok(Expr::TryRetry {
            body: Box::new(body),
            attempts,
            backoff,
            timeout_ms,
            span: start.merge(self.prev_span()),
        })
    }

    pub(super) fn parse_u64_literal(&mut self, description: &str) -> Result<u64, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokKind::Int(value) if value >= 0 => {
                self.bump();
                Ok(value as u64)
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: description.into(),
                },
                span,
            }),
        }
    }
}
