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
            // `| Name(field: Type, ...)`.
            if matches!(self.peek(), TokKind::Pipe) {
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
            visibility,
            span: start.merge(end),
        })
    }

    /// `'|' IDENT ('(' field_list ')')? NEWLINE` — one sum-type
    /// variant (slice 45h).
    fn parse_sum_variant(&mut self) -> Result<corvid_ast::SumVariant, ParseError> {
        let start = self.peek_span();
        self.bump(); // |
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
            span: start.merge(end),
        })
    }

    pub(super) fn parse_field(&mut self) -> Result<Field, ParseError> {
        let start = self.peek_span();
        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` between field name and type")?;
        let ty = self.parse_type_ref()?;
        let end = ty.span();
        self.expect_newline()?;
        Ok(Field {
            name: Ident::new(name, name_span),
            ty,
            span: start.merge(end),
        })
    }
}
