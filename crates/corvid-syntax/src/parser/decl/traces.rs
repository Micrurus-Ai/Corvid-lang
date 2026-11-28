//! The `traces:` declaration (lineage L3): `traces: none`, or a block
//! saying where a served program keeps its run traces and how long.
//!
//! ```text
//! traces:
//!     location: env("TRACE_DIR")
//!     keep: 30 days
//! ```
//!
//! `traces` is contextual: only `traces:` at the start of a declaration
//! is this block, so `traces` stays an ordinary name everywhere else.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::{describe_token, Parser};
use crate::token::TokKind;
use corvid_ast::{TraceRecording, TracesDecl};

impl<'a> Parser<'a> {
    /// Whether the next tokens begin a `traces:` declaration.
    pub(super) fn at_traces_decl(&self) -> bool {
        matches!(self.peek(), TokKind::Ident(word) if word == "traces") && matches!(self.peek_ahead(1), TokKind::Colon)
    }

    pub(super) fn parse_traces_decl(&mut self) -> Result<TracesDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // traces
        self.expect(TokKind::Colon, "`:` after `traces`")?;
        if self.peek_ident_is("none") {
            self.bump();
            let span = start.merge(self.prev_span());
            self.expect_newline()?;
            return Ok(TracesDecl { recording: None, span });
        }
        self.expect_newline()?;
        self.expect_block_start()?;
        let mut recording = TraceRecording { location: None, keep: None, span: start };
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            if let Err(e) = self.parse_traces_item(&mut recording) {
                self.errors.push(e);
                self.sync_to_statement_boundary();
            }
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }
        let span = start.merge(end);
        recording.span = span;
        Ok(TracesDecl { recording: Some(recording), span })
    }

    fn parse_traces_item(&mut self, recording: &mut TraceRecording) -> Result<(), ParseError> {
        let (item, item_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after a `traces` item")?;
        match item.as_str() {
            "location" => recording.location = Some(self.parse_database_location()?),
            "keep" => {
                let at = self.peek_span();
                let expected = |got: &TokKind, span| ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(got),
                        expected: "how long a trace is kept, as in `keep: 720 hours`".into(),
                    },
                    span,
                };
                let count = match self.peek().clone() {
                    TokKind::Int(n) => {
                        self.bump();
                        n.to_string()
                    }
                    other => return Err(expected(&other, self.peek_span())),
                };
                let unit = match self.peek().clone() {
                    TokKind::Ident(word) if corvid_ast::duration_unit(&word).is_some() => {
                        self.bump();
                        word
                    }
                    other => return Err(expected(&other, self.peek_span())),
                };
                recording.keep = Some((count, unit, at.merge(self.prev_span())));
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("`traces` item `{other}`"),
                        expected: "`location` or `keep`".into(),
                    },
                    span: item_span,
                })
            }
        }
        self.expect_newline()
    }
}
