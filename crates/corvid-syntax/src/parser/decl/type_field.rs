//! `type` declaration parsing — record-type definitions plus
//! the per-field parser shared with no other decl family.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::Parser;
use crate::token::TokKind;
use corvid_ast::{Field, Ident, TypeDecl, Visibility};

impl<'a> Parser<'a> {
    pub(super) fn parse_type_decl(
        &mut self,
        visibility: Visibility,
    ) -> Result<TypeDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // type

        let (name, name_span) = self.expect_ident()?;

        // Type alias (slice 45n): `type CustomerId = String`.
        if matches!(self.peek(), TokKind::Assign) {
            self.bump(); // =
            let target = self.parse_type_ref()?;
            let end = self.prev_span();
            self.expect_newline()?;
            return Ok(TypeDecl {
                name: Ident::new(name, name_span),
                fields: Vec::new(),
                variants: Vec::new(),
                alias: Some(target),
                visibility,
                span: start.merge(end),
            });
        }

        self.expect(TokKind::Colon, "`:` after type name")?;
        self.expect_newline()?;

        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut fields = Vec::new();
        let mut variants = Vec::new();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            // Sum-type variant line (slice 45h): `| Name` or
            // `| Name(field: Type, ...)`, optionally preceded by
            // `@status(code)` / `@ui(...)` variant attributes (51e).
            if matches!(self.peek(), TokKind::Pipe) || self.variant_attrs_precede_pipe() {
                match self.parse_sum_variant() {
                    Ok(v) => variants.push(v),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            match self.parse_field() {
                Ok(f) => fields.push(f),
                Err(e) => {
                    self.errors.push(e);
                    self.sync_to_statement_boundary();
                }
            }
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }

        // A type is a record XOR a sum — mixing field lines and
        // variant lines is a parse error.
        if !fields.is_empty() && !variants.is_empty() {
            self.errors.push(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: "a type declaration mixing record fields and sum variants".into(),
                    expected: "either `name: Type` field lines OR `| Variant` lines, not both"
                        .into(),
                },
                span: start.merge(end),
            });
        }

        Ok(TypeDecl {
            name: Ident::new(name, name_span),
            fields,
            variants,
            alias: None,
            visibility,
            span: start.merge(end),
        })
    }

    /// `'|' IDENT ('(' field_list ')')? NEWLINE` — one sum-type
    /// variant (slice 45h).
    /// Lookahead: `@ident(...)` groups (newline-separated) followed
    /// by a `|`? Distinguishes a variant carrying `@status`/`@ui`
    /// from a field carrying `@ui` (whose group is followed by an
    /// identifier). Slice 51e.
    fn variant_attrs_precede_pipe(&self) -> bool {
        if !matches!(self.peek(), TokKind::At) {
            return false;
        }
        let mut i = 0usize;
        loop {
            match self.peek_ahead(i) {
                TokKind::Newline => i += 1,
                TokKind::At => {
                    i += 1;
                    if !matches!(self.peek_ahead(i), TokKind::Ident(_)) {
                        return false;
                    }
                    i += 1;
                    if matches!(self.peek_ahead(i), TokKind::LParen) {
                        i += 1;
                        let mut depth = 1i32;
                        while depth > 0 {
                            match self.peek_ahead(i) {
                                TokKind::LParen => depth += 1,
                                TokKind::RParen => depth -= 1,
                                TokKind::Eof => return false,
                                _ => {}
                            }
                            i += 1;
                        }
                    }
                }
                TokKind::Pipe => return true,
                _ => return false,
            }
        }
    }

    fn parse_sum_variant(&mut self) -> Result<corvid_ast::SumVariant, ParseError> {
        let start = self.peek_span();
        let mut status = None;
        let mut ui = Vec::new();
        while matches!(self.peek(), TokKind::At) {
            self.bump(); // @
            let (attr, attr_span) = self.expect_ident()?;
            match attr.as_str() {
                "status" => {
                    self.expect(TokKind::LParen, "`(` after `@status`")?;
                    let (code, _) = self.expect_positive_int_literal("an HTTP status code")?;
                    self.expect(TokKind::RParen, "`)` after `@status`")?;
                    status = Some(code);
                }
                "ui" => {
                    self.expect(TokKind::LParen, "`(` after `@ui`")?;
                    while !matches!(self.peek(), TokKind::RParen | TokKind::Eof) {
                        let hstart = self.peek_span();
                        let (key, key_span) = self.expect_ident()?;
                        self.expect(TokKind::Colon, "`:` after a `@ui` hint key")?;
                        let value = match self.peek().clone() {
                            TokKind::StringLit(s) => {
                                self.bump();
                                corvid_ast::UiHintValue::Str(s)
                            }
                            TokKind::KwTrue => {
                                self.bump();
                                corvid_ast::UiHintValue::Bool(true)
                            }
                            TokKind::KwFalse => {
                                self.bump();
                                corvid_ast::UiHintValue::Bool(false)
                            }
                            TokKind::Int(n) => {
                                self.bump();
                                corvid_ast::UiHintValue::Int(n)
                            }
                            other => {
                                return Err(ParseError {
                                    kind: ParseErrorKind::UnexpectedToken {
                                        got: format!("{other:?}"),
                                        expected: "a string, boolean, or integer `@ui` value"
                                            .into(),
                                    },
                                    span: self.peek_span(),
                                });
                            }
                        };
                        ui.push(corvid_ast::UiHint {
                            key: Ident::new(key, key_span),
                            value,
                            span: hstart.merge(self.prev_span()),
                        });
                        if matches!(self.peek(), TokKind::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.expect(TokKind::RParen, "`)` after `@ui`")?;
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("variant attribute `@{attr}`"),
                            expected: "`@status(code)` or `@ui(...)` on a variant".into(),
                        },
                        span: attr_span,
                    });
                }
            }
            self.skip_newlines();
        }
        self.expect(TokKind::Pipe, "`|` before a sum-type variant")?;
        let (name, name_span) = self.expect_ident()?;
        let mut fields = Vec::new();
        if matches!(self.peek(), TokKind::LParen) {
            self.bump();
            loop {
                let fstart = self.peek_span();
                let (fname, fname_span) = self.expect_ident()?;
                self.expect(TokKind::Colon, "`:` between variant field name and type")?;
                let ty = self.parse_type_ref()?;
                let fend = ty.span();
                fields.push(Field {
                    name: Ident::new(fname, fname_span),
                    ty,
                    refinement: None,
                    ui: Vec::new(),
                    span: fstart.merge(fend),
                });
                if !matches!(self.peek(), TokKind::Comma) {
                    break;
                }
                self.bump();
            }
            self.expect(TokKind::RParen, "`)` after variant fields")?;
        }
        let end = self.prev_span();
        self.expect_newline()?;
        Ok(corvid_ast::SumVariant {
            name: Ident::new(name, name_span),
            fields,
            status,
            ui,
            span: start.merge(end),
        })
    }

    pub(super) fn parse_field(&mut self) -> Result<Field, ParseError> {
        let start = self.peek_span();
        let ui = self.parse_field_ui_hints()?;
        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` between field name and type")?;
        let ty = self.parse_type_ref()?;
        let refinement = self.parse_field_refinement()?;
        let end = self.prev_span();
        self.expect_newline()?;
        Ok(Field {
            name: Ident::new(name, name_span),
            ty,
            refinement,
            ui,
            span: start.merge(end),
        })
    }

    /// Optional `@ui(key: value, ...)` presentation hints preceding a
    /// field (slice 51d). Multi-line is fine — the lexer suppresses
    /// newlines inside the parens.
    fn parse_field_ui_hints(&mut self) -> Result<Vec<corvid_ast::UiHint>, ParseError> {
        if !matches!(self.peek(), TokKind::At) {
            return Ok(Vec::new());
        }
        // Only `@ui(` is a field hint; leave other `@` for callers.
        if !matches!(self.peek_ahead(1), TokKind::Ident(w) if w == "ui") {
            return Ok(Vec::new());
        }
        self.bump(); // @
        self.bump(); // ui
        self.expect(TokKind::LParen, "`(` after `@ui`")?;
        let mut hints = Vec::new();
        while !matches!(self.peek(), TokKind::RParen | TokKind::Eof) {
            let start = self.peek_span();
            let (key, key_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a `@ui` hint key")?;
            let value = match self.peek().clone() {
                TokKind::StringLit(s) => {
                    self.bump();
                    corvid_ast::UiHintValue::Str(s)
                }
                TokKind::KwTrue => {
                    self.bump();
                    corvid_ast::UiHintValue::Bool(true)
                }
                TokKind::KwFalse => {
                    self.bump();
                    corvid_ast::UiHintValue::Bool(false)
                }
                TokKind::Int(n) => {
                    self.bump();
                    corvid_ast::UiHintValue::Int(n)
                }
                other => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("{other:?}"),
                            expected: "a string, boolean, or integer `@ui` hint value".into(),
                        },
                        span: self.peek_span(),
                    });
                }
            };
            hints.push(corvid_ast::UiHint {
                key: corvid_ast::Ident::new(key, key_span),
                value,
                span: start.merge(self.prev_span()),
            });
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokKind::RParen, "`)` after `@ui` hints")?;
        self.skip_newlines();
        Ok(hints)
    }

    /// Optional field refinement (slice 50j): contextual `where`
    /// followed by `between(min, max)` or `len_between(min, max)`.
    /// `where` stays an ordinary identifier everywhere else.
    fn parse_field_refinement(
        &mut self,
    ) -> Result<Option<corvid_ast::Refinement>, ParseError> {
        if !matches!(self.peek(), TokKind::Ident(w) if w == "where") {
            return Ok(None);
        }
        self.bump(); // where
        let (form, form_span) = self.expect_ident()?;
        self.expect(TokKind::LParen, "`(` after the refinement form")?;
        let min = self.expect_refinement_int()?;
        self.expect(TokKind::Comma, "`,` between refinement bounds")?;
        let max = self.expect_refinement_int()?;
        self.expect(TokKind::RParen, "`)` after refinement bounds")?;
        match form.as_str() {
            "between" => Ok(Some(corvid_ast::Refinement::Between { min, max })),
            "len_between" => {
                if min < 0 || max < 0 {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "a negative length bound".into(),
                            expected: "non-negative bounds for `len_between`".into(),
                        },
                        span: form_span,
                    });
                }
                Ok(Some(corvid_ast::Refinement::LenBetween {
                    min: min as u64,
                    max: max as u64,
                }))
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("refinement form `{other}`"),
                    expected: "`between(min, max)` or `len_between(min, max)`".into(),
                },
                span: form_span,
            }),
        }
    }

    /// An integer bound, with optional leading `-`.
    fn expect_refinement_int(&mut self) -> Result<i64, ParseError> {
        let negative = if matches!(self.peek(), TokKind::Minus) {
            self.bump();
            true
        } else {
            false
        };
        match self.peek().clone() {
            TokKind::Int(n) => {
                self.bump();
                Ok(if negative { -n } else { n })
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("{other:?}"),
                    expected: "an integer refinement bound".into(),
                },
                span: self.peek_span(),
            }),
        }
    }
}
