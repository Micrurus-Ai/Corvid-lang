//! `match` expression parsing (slice 45i).
//!
//! ```text
//! match_expr ::= 'match' expr ':' INDENT match_arm+ DEDENT
//! match_arm  ::= pattern ('if' expr)? '->' expr NEWLINE
//! pattern    ::= literal
//!              | '_'                                # wildcard
//!              | IDENT                              # binding
//!              | IDENT '@' pattern                  # bind + narrow
//!              | IDENT '(' pattern (',' pattern)* ')'   # variant / Some / Ok / Err
//!              | IDENT '{' field_pattern (',' field_pattern)* (',' '..')? '}'
//! ```
//!
//! Mirrors `replay_expr.rs`'s block-in-expression shape: the arms
//! live in an `INDENT … DEDENT` block that the expression parser
//! consumes explicitly.

use super::{describe_token, Parser};
use crate::errors::{ParseError, ParseErrorKind};
use crate::token::TokKind;
use corvid_ast::{Expr, FieldPattern, Ident, Literal, MatchArm, Pattern};

impl<'a> Parser<'a> {
    pub(super) fn parse_match_expr(&mut self) -> Result<Expr, ParseError> {
        let start = self.peek_span();
        self.bump(); // `match`

        let scrutinee = self.parse_expr()?;
        self.expect(TokKind::Colon, "`:` after `match <expr>`")?;
        self.expect_newline()?;

        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut arms: Vec<MatchArm> = Vec::new();
        loop {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            arms.push(self.parse_match_arm()?);
        }

        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }
        // The arm block's DEDENT terminates the enclosing statement.
        self.block_expr_terminated = true;
        if arms.is_empty() {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: "a `match` with no arms".into(),
                    expected: "at least one `pattern -> expr` arm".into(),
                },
                span: start.merge(end),
            });
        }

        Ok(Expr::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            span: start.merge(end),
        })
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, ParseError> {
        let start = self.peek_span();
        let pattern = self.parse_pattern()?;
        let guard = if matches!(self.peek(), TokKind::KwIf) {
            self.bump();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(TokKind::Arrow, "`->` between pattern and arm body")?;
        let body = self.parse_expr()?;
        let end = body.span();
        self.expect_newline()?;
        Ok(MatchArm {
            pattern,
            guard,
            body,
            span: start.merge(end),
        })
    }

    pub(super) fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        let start = self.peek_span();
        match self.peek().clone() {
            TokKind::Int(v) => {
                self.bump();
                Ok(Pattern::Literal {
                    value: Literal::Int(v),
                    span: start,
                })
            }
            TokKind::Float(v) => {
                self.bump();
                Ok(Pattern::Literal {
                    value: Literal::Float(v),
                    span: start,
                })
            }
            TokKind::Minus => {
                // Negative numeric literal pattern.
                self.bump();
                match self.peek().clone() {
                    TokKind::Int(v) => {
                        self.bump();
                        Ok(Pattern::Literal {
                            value: Literal::Int(-v),
                            span: start.merge(self.prev_span()),
                        })
                    }
                    TokKind::Float(v) => {
                        self.bump();
                        Ok(Pattern::Literal {
                            value: Literal::Float(-v),
                            span: start.merge(self.prev_span()),
                        })
                    }
                    other => Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: describe_token(&other),
                            expected: "a numeric literal after `-` in a pattern".into(),
                        },
                        span: self.peek_span(),
                    }),
                }
            }
            TokKind::StringLit(s) => {
                self.bump();
                Ok(Pattern::Literal {
                    value: Literal::String(s),
                    span: start,
                })
            }
            TokKind::KwTrue => {
                self.bump();
                Ok(Pattern::Literal {
                    value: Literal::Bool(true),
                    span: start,
                })
            }
            TokKind::KwFalse => {
                self.bump();
                Ok(Pattern::Literal {
                    value: Literal::Bool(false),
                    span: start,
                })
            }
            TokKind::Ident(name) => {
                self.bump();
                let name_span = start;
                if name == "_" {
                    return Ok(Pattern::Wildcard { span: name_span });
                }
                match self.peek() {
                    // `x @ pattern` — bind the whole value, then narrow.
                    TokKind::At => {
                        self.bump();
                        let inner = self.parse_pattern()?;
                        let span = name_span.merge(self.prev_span());
                        Ok(Pattern::At {
                            name: Ident::new(name, name_span),
                            inner: Box::new(inner),
                            span,
                        })
                    }
                    // `Approved(p1, ...)` — variant / Some / Ok / Err.
                    TokKind::LParen => {
                        self.bump();
                        let mut args = Vec::new();
                        if !matches!(self.peek(), TokKind::RParen) {
                            args.push(self.parse_pattern()?);
                            while matches!(self.peek(), TokKind::Comma) {
                                self.bump();
                                if matches!(self.peek(), TokKind::RParen) {
                                    break;
                                }
                                args.push(self.parse_pattern()?);
                            }
                        }
                        self.expect(TokKind::RParen, "`)` after variant pattern fields")?;
                        let span = name_span.merge(self.prev_span());
                        Ok(Pattern::Variant {
                            name: Ident::new(name, name_span),
                            args,
                            span,
                        })
                    }
                    // `Decision { refund: true, amount, .. }`.
                    TokKind::LBrace => {
                        self.bump();
                        let mut fields: Vec<FieldPattern> = Vec::new();
                        let mut rest = false;
                        loop {
                            self.skip_newlines();
                            if matches!(self.peek(), TokKind::RBrace) {
                                break;
                            }
                            // `..` rest marker (two dots).
                            if matches!(self.peek(), TokKind::Dot) {
                                self.bump();
                                self.expect(TokKind::Dot, "`..` rest marker in record pattern")?;
                                rest = true;
                                if matches!(self.peek(), TokKind::Comma) {
                                    self.bump();
                                }
                                continue;
                            }
                            let fstart = self.peek_span();
                            let (fname, fname_span) = self.expect_ident()?;
                            let sub = if matches!(self.peek(), TokKind::Colon) {
                                self.bump();
                                Some(self.parse_pattern()?)
                            } else {
                                None // shorthand: binds the field name
                            };
                            fields.push(FieldPattern {
                                name: Ident::new(fname, fname_span),
                                pattern: sub,
                                span: fstart.merge(self.prev_span()),
                            });
                            if matches!(self.peek(), TokKind::Comma) {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                        self.expect(TokKind::RBrace, "`}` after record pattern")?;
                        let span = name_span.merge(self.prev_span());
                        Ok(Pattern::Record {
                            name: Ident::new(name, name_span),
                            fields,
                            rest,
                            span,
                        })
                    }
                    // Bare identifier: unit variant OR binding — the
                    // resolver disambiguates (a name resolving to a
                    // variant/None is a variant pattern; anything else
                    // binds).
                    _ => Ok(Pattern::Name {
                        name: Ident::new(name, name_span),
                        span: name_span,
                    }),
                }
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: "a pattern: literal, `_`, a binding name, `Variant(...)`, or `Type { ... }`"
                        .into(),
                },
                span: start,
            }),
        }
    }
}
