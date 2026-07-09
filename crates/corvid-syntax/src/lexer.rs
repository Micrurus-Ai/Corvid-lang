//! Hand-rolled lexer for Corvid.
//!
//! Produces a token stream including Python-style `Indent`, `Dedent`,
//! and `Newline` structural tokens. See `ARCHITECTURE.md` §4.

use crate::errors::{LexError, LexErrorKind};
use crate::token::{TokKind, Token};
use corvid_ast::Span;

/// Lex a full source string. Returns tokens on success or a list of errors.
pub fn lex(source: &str) -> Result<Vec<Token>, Vec<LexError>> {
    let mut lx = Lexer::new(source);
    lx.run();
    if lx.errors.is_empty() {
        Ok(lx.tokens)
    } else {
        Err(lx.errors)
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    errors: Vec<LexError>,
    /// Nesting depth of `(`, `[`. Newlines inside brackets are ignored.
    bracket_depth: i32,
    /// Stack of current indentation column widths. Starts with `[0]`.
    indent_stack: Vec<usize>,
    /// True once we've emitted any non-structural token on the current
    /// logical line. Controls whether a `\n` produces a `Newline` token.
    had_content_on_line: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
            tokens: Vec::new(),
            errors: Vec::new(),
            bracket_depth: 0,
            indent_stack: vec![0],
            had_content_on_line: false,
        }
    }

    fn run(&mut self) {
        if self.bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.pos = 3;
        }

        // Handle indentation of the very first line.
        self.process_line_start();

        while self.pos < self.bytes.len() {
            let c = self.bytes[self.pos];
            match c {
                // Inline whitespace: just skip. `\r` is silently absorbed
                // so CRLF-encoded files (Windows defaults, Git autocrlf)
                // lex identically to LF-only sources.
                b' ' | b'\t' | b'\r' => {
                    self.pos += 1;
                }
                b'\n' => {
                    let nl_start = self.pos;
                    self.pos += 1;
                    if self.bracket_depth == 0 {
                        if self.had_content_on_line {
                            self.emit_structural(
                                TokKind::Newline,
                                Span::new(nl_start, self.pos),
                            );
                            self.had_content_on_line = false;
                        }
                        self.process_line_start();
                    }
                }
                b'#' => {
                    // Line comment: skip to end of line (not consuming \n).
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                b'\\' => self.lex_backslash_continuation(),
                b'0'..=b'9' => self.lex_number(),
                b'"' => self.lex_string(),
                c if is_ident_start(c) => self.lex_ident_or_kw(),
                _ => self.lex_punct(),
            }
        }

        // End-of-file: finish any open line and dedent back to column 0.
        if self.had_content_on_line {
            self.emit_structural(TokKind::Newline, Span::new(self.pos, self.pos));
            self.had_content_on_line = false;
        }
        while self.indent_stack.len() > 1 {
            self.emit_structural(TokKind::Dedent, Span::new(self.pos, self.pos));
            self.indent_stack.pop();
        }
        self.emit_structural(TokKind::Eof, Span::new(self.pos, self.pos));
    }

    /// Called at the start of each physical line (after `\n`) and at file
    /// start. Measures indentation and emits `Indent`/`Dedent` tokens.
    /// Blank or comment-only lines are skipped.
    fn process_line_start(&mut self) {
        // Tolerate a leading `\r` from a CRLF blank line.
        while self.pos < self.bytes.len() && self.bytes[self.pos] == b'\r' {
            self.pos += 1;
        }

        let start = self.pos;
        let mut indent = 0usize;
        let mut had_tab = false;

        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' => {
                    indent += 1;
                    self.pos += 1;
                }
                b'\t' => {
                    had_tab = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }

        // Blank or comment-only line — don't affect indentation.
        if self.pos >= self.bytes.len() {
            return;
        }
        match self.bytes[self.pos] {
            b'\n' | b'\r' | b'#' => return,
            _ => {}
        }

        if had_tab {
            self.errors.push(LexError {
                kind: LexErrorKind::TabIndentation,
                span: Span::new(start, self.pos),
            });
        }

        let current = *self.indent_stack.last().expect("indent stack never empty");
        if indent > current {
            self.indent_stack.push(indent);
            self.emit_structural(TokKind::Indent, Span::new(start, self.pos));
        } else if indent < current {
            while *self.indent_stack.last().unwrap() > indent {
                self.indent_stack.pop();
                self.emit_structural(TokKind::Dedent, Span::new(self.pos, self.pos));
            }
            if *self.indent_stack.last().unwrap() != indent {
                self.errors.push(LexError {
                    kind: LexErrorKind::InconsistentDedent,
                    span: Span::new(start, self.pos),
                });
            }
        }
    }

    fn emit(&mut self, kind: TokKind, span: Span) {
        self.had_content_on_line = true;
        self.tokens.push(Token::new(kind, span));
    }

    fn emit_structural(&mut self, kind: TokKind, span: Span) {
        self.tokens.push(Token::new(kind, span));
    }

    fn lex_number(&mut self) {
        let start = self.pos;
        if self.is_after_hash_digest_prefix(start) {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_hexdigit() {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            self.emit(TokKind::Ident(text.to_string()), Span::new(start, self.pos));
            return;
        }
        while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let is_float = self.pos + 1 < self.bytes.len()
            && self.bytes[self.pos] == b'.'
            && self.bytes[self.pos + 1].is_ascii_digit();
        if is_float {
            self.pos += 1; // consume dot
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let text = &self.src[start..self.pos];
            match text.parse::<f64>() {
                Ok(v) => self.emit(TokKind::Float(v), Span::new(start, self.pos)),
                Err(_) => self.errors.push(LexError {
                    kind: LexErrorKind::InvalidNumber(text.to_string()),
                    span: Span::new(start, self.pos),
                }),
            }
        } else {
            let text = &self.src[start..self.pos];
            match text.parse::<i64>() {
                Ok(v) => self.emit(TokKind::Int(v), Span::new(start, self.pos)),
                Err(_) => self.errors.push(LexError {
                    kind: LexErrorKind::InvalidNumber(text.to_string()),
                    span: Span::new(start, self.pos),
                }),
            }
        }
    }

    fn lex_string(&mut self) {
        // Triple-quoted multi-line string: `"""..."""`.
        if self.pos + 2 < self.bytes.len()
            && self.bytes[self.pos + 1] == b'"'
            && self.bytes[self.pos + 2] == b'"'
        {
            self.lex_triple_string();
        } else {
            self.lex_single_string();
        }
    }

    /// Handle a `\` encountered outside any string. When `\` is
    /// immediately followed by a newline (optionally CRLF), the
    /// backslash plus the newline plus any leading whitespace on
    /// the next physical line are consumed silently — joining the
    /// two physical lines into one logical line. No `Newline`,
    /// `Indent`, or `Dedent` token is emitted, and `had_content_on_line`
    /// is preserved across the boundary.
    ///
    /// `\` not at end-of-line keeps emitting `UnexpectedChar('\\')`
    /// to preserve the existing E0003 diagnostic for stray backslashes.
    ///
    /// Triple-quoted strings already span lines, so this rewriting
    /// only applies to top-level lexing and to single-quoted strings
    /// (handled in `lex_single_string`).
    fn lex_backslash_continuation(&mut self) {
        let bs_start = self.pos;
        if self.is_line_continuation_at(self.pos) {
            self.consume_line_continuation();
        } else {
            self.errors.push(LexError {
                kind: LexErrorKind::UnexpectedChar('\\'),
                span: Span::new(bs_start, bs_start + 1),
            });
            self.pos += 1;
        }
    }

    /// Returns true iff `pos` is on a `\` that is immediately
    /// followed by a newline (optionally CRLF).
    fn is_line_continuation_at(&self, pos: usize) -> bool {
        if pos >= self.bytes.len() || self.bytes[pos] != b'\\' {
            return false;
        }
        let mut probe = pos + 1;
        if probe < self.bytes.len() && self.bytes[probe] == b'\r' {
            probe += 1;
        }
        probe < self.bytes.len() && self.bytes[probe] == b'\n'
    }

    /// Consume `\` + optional `\r` + `\n` + any leading whitespace
    /// on the next physical line. Caller must have verified that
    /// `is_line_continuation_at(self.pos)` is true.
    fn consume_line_continuation(&mut self) {
        // Consume `\`.
        self.pos += 1;
        // Consume optional `\r` and the `\n` itself.
        if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\r' {
            self.pos += 1;
        }
        debug_assert!(
            self.pos < self.bytes.len() && self.bytes[self.pos] == b'\n',
            "consume_line_continuation called without a trailing newline"
        );
        self.pos += 1;
        // Consume leading whitespace on the joined-in line. The
        // continuation merges both physical lines into one logical
        // line, so the next line's indentation must NOT influence
        // `process_line_start`'s Indent/Dedent emission. By eating
        // the whitespace here we never reach `process_line_start`
        // for the continuation line.
        while self.pos < self.bytes.len()
            && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t')
        {
            self.pos += 1;
        }
    }

    fn lex_single_string(&mut self) {
        let start = self.pos;
        self.pos += 1; // consume opening "
        let mut contents = String::new();
        loop {
            if self.pos >= self.bytes.len() {
                self.errors.push(LexError {
                    kind: LexErrorKind::UnterminatedString,
                    span: Span::new(start, self.pos),
                });
                return;
            }
            let c = self.bytes[self.pos];
            match c {
                b'"' => {
                    self.pos += 1;
                    self.emit(TokKind::StringLit(contents), Span::new(start, self.pos));
                    return;
                }
                b'\n' => {
                    // Single-line strings may not span lines.
                    self.errors.push(LexError {
                        kind: LexErrorKind::UnterminatedString,
                        span: Span::new(start, self.pos),
                    });
                    return;
                }
                b'\\' => {
                    // Line continuation inside a single-quoted string:
                    // `\` + newline (+ leading whitespace) is consumed
                    // silently, treating the two physical lines as one
                    // logical string. Triple-quoted strings already span
                    // lines naturally and are not rewritten here.
                    if self.is_line_continuation_at(self.pos) {
                        self.consume_line_continuation();
                    } else if let Some(ch) = self.consume_escape(start) {
                        contents.push(ch);
                    } else {
                        return;
                    }
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    contents.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn lex_triple_string(&mut self) {
        let start = self.pos;
        self.pos += 3; // consume opening """
        let mut contents = String::new();
        loop {
            if self.pos >= self.bytes.len() {
                self.errors.push(LexError {
                    kind: LexErrorKind::UnterminatedString,
                    span: Span::new(start, self.pos),
                });
                return;
            }
            // Closing """
            if self.pos + 2 < self.bytes.len()
                && self.bytes[self.pos] == b'"'
                && self.bytes[self.pos + 1] == b'"'
                && self.bytes[self.pos + 2] == b'"'
            {
                self.pos += 3;
                self.emit(TokKind::StringLit(contents), Span::new(start, self.pos));
                return;
            }
            // Special case: closing """ at exact EOF.
            if self.pos + 3 == self.bytes.len()
                && self.bytes[self.pos] == b'"'
                && self.bytes[self.pos + 1] == b'"'
                && self.bytes[self.pos + 2] == b'"'
            {
                self.pos += 3;
                self.emit(TokKind::StringLit(contents), Span::new(start, self.pos));
                return;
            }

            let c = self.bytes[self.pos];
            if c == b'\\' {
                if let Some(ch) = self.consume_escape(start) {
                    contents.push(ch);
                } else {
                    return;
                }
            } else {
                let ch = self.src[self.pos..].chars().next().unwrap();
                contents.push(ch);
                self.pos += ch.len_utf8();
            }
        }
    }

    /// Consume a `\x` escape, returning the decoded character. On an invalid
    /// escape, record an error but still return the raw character so lexing
    /// can continue. Returns `None` only on EOF after the backslash.
    fn consume_escape(&mut self, _string_start: usize) -> Option<char> {
        let esc_start = self.pos;
        self.pos += 1; // consume backslash
        if self.pos >= self.bytes.len() {
            self.errors.push(LexError {
                kind: LexErrorKind::UnterminatedString,
                span: Span::new(esc_start, self.pos),
            });
            return None;
        }
        let esc = self.bytes[self.pos];
        let ch = match esc {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'\\' => '\\',
            b'"' => '"',
            b'0' => '\0',
            other => {
                self.errors.push(LexError {
                    kind: LexErrorKind::InvalidEscape(other as char),
                    span: Span::new(esc_start, self.pos + 1),
                });
                other as char
            }
        };
        self.pos += 1;
        Some(ch)
    }

    fn lex_ident_or_kw(&mut self) {
        let start = self.pos;
        while self.pos < self.bytes.len() && is_ident_continue(self.bytes[self.pos]) {
            self.pos += 1;
        }
        let text = &self.src[start..self.pos];
        let kind = TokKind::keyword_from(text)
            .unwrap_or_else(|| TokKind::Ident(text.to_string()));
        self.emit(kind, Span::new(start, self.pos));
    }

    fn lex_punct(&mut self) {
        let start = self.pos;
        let c = self.bytes[self.pos];
        let (kind, len): (TokKind, usize) = match c {
            b'(' => {
                self.bracket_depth += 1;
                (TokKind::LParen, 1)
            }
            b')' => {
                self.bracket_depth -= 1;
                (TokKind::RParen, 1)
            }
            b'[' => {
                self.bracket_depth += 1;
                (TokKind::LBracket, 1)
            }
            b']' => {
                self.bracket_depth -= 1;
                (TokKind::RBracket, 1)
            }
            b'{' => (TokKind::LBrace, 1),
            b'}' => (TokKind::RBrace, 1),
            b':' => (TokKind::Colon, 1),
            b',' => (TokKind::Comma, 1),
            b'.' => (TokKind::Dot, 1),
            b'?' => (TokKind::Question, 1),
            b'+' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::PlusEq, 2)
                } else {
                    (TokKind::Plus, 1)
                }
            }
            b'*' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::StarEq, 2)
                } else {
                    (TokKind::Star, 1)
                }
            }
            b'/' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::SlashEq, 2)
                } else {
                    (TokKind::Slash, 1)
                }
            }
            b'%' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::PercentEq, 2)
                } else {
                    (TokKind::Percent, 1)
                }
            }
            b'@' => (TokKind::At, 1),
            b'\'' => (TokKind::Apostrophe, 1),
            b'$' => (TokKind::Dollar, 1),
            b'-' => {
                if self.peek(1) == Some(b'>') {
                    (TokKind::Arrow, 2)
                } else if self.peek(1) == Some(b'=') {
                    (TokKind::MinusEq, 2)
                } else {
                    (TokKind::Minus, 1)
                }
            }
            b'=' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::Eq, 2)
                } else {
                    (TokKind::Assign, 1)
                }
            }
            b'!' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::NotEq, 2)
                } else {
                    self.errors.push(LexError {
                        kind: LexErrorKind::UnexpectedChar('!'),
                        span: Span::new(start, start + 1),
                    });
                    self.pos += 1;
                    return;
                }
            }
            b'<' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::LtEq, 2)
                } else {
                    (TokKind::Lt, 1)
                }
            }
            b'>' => {
                if self.peek(1) == Some(b'=') {
                    (TokKind::GtEq, 2)
                } else {
                    (TokKind::Gt, 1)
                }
            }
            _ => {
                let ch = self.src[self.pos..].chars().next().unwrap_or('?');
                self.errors.push(LexError {
                    kind: LexErrorKind::UnexpectedChar(ch),
                    span: Span::new(start, start + ch.len_utf8()),
                });
                self.pos += ch.len_utf8();
                return;
            }
        };
        self.pos += len;
        self.emit(kind, Span::new(start, start + len));
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn is_after_hash_digest_prefix(&self, start: usize) -> bool {
        self.src[..start].ends_with("hash:sha256:")
    }
}

fn is_ident_start(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphabetic()
}

fn is_ident_continue(c: u8) -> bool {
    c == b'_' || c.is_ascii_alphanumeric()
}

#[cfg(test)]
mod backslash_continuation_tests {
    //! Regression tests for slice 20n-A: `\` end-of-line continuation.
    //!
    //! - Outside any string: `\` + newline + leading whitespace is
    //!   silently consumed; the two physical lines lex as one logical
    //!   line (no `Newline`/`Indent`/`Dedent` emitted at the join).
    //! - Inside a `"..."` single-quoted string: same rewriting; the
    //!   two physical lines join into one string contents value.
    //! - `\` not followed by a newline at top level still produces
    //!   `LexErrorKind::UnexpectedChar('\\')` (preserves E0003).
    //! - Triple-quoted `"""..."""` strings are NOT rewritten (the
    //!   feature only targets single-quoted strings + top-level).

    use super::{lex, LexErrorKind, TokKind};

    fn token_kinds(src: &str) -> Vec<TokKind> {
        lex(src)
            .expect("expected source to lex without errors")
            .into_iter()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn outside_string_continuation_suppresses_newline_and_indent() {
        // The reproduction from the original gap report: a `\` at end
        // of line outside any string lets the program continue onto
        // the next line without inserting a structural `Newline` or
        // `Indent` token.
        let src = "agent main() -> Bool: \\\n    return true\n";
        let kinds = token_kinds(src);
        // No structural Newline immediately after the `:` — the
        // `Return` token sits on the same logical line.
        let colon_idx = kinds
            .iter()
            .position(|k| matches!(k, TokKind::Colon))
            .expect("colon present");
        let return_idx = kinds
            .iter()
            .position(|k| matches!(k, TokKind::KwReturn))
            .expect("return present");
        assert!(return_idx > colon_idx, "return must follow colon");
        let between = &kinds[colon_idx + 1..return_idx];
        for kind in between {
            assert!(
                !matches!(kind, TokKind::Newline | TokKind::Indent),
                "no structural Newline / Indent allowed between `:` and `return` after a `\\` continuation; saw {kind:?}",
            );
        }
    }

    #[test]
    fn inside_single_quoted_string_continuation_joins_lines() {
        // `"foo \<NL>    bar"` lexes as the string "foobar" — the
        // backslash + newline + leading whitespace are silently
        // consumed.
        let src = "agent main() -> String:\n    return \"foo \\\n           bar\"\n";
        let kinds = token_kinds(src);
        let lit = kinds
            .iter()
            .find_map(|k| match k {
                TokKind::StringLit(s) => Some(s.as_str()),
                _ => None,
            })
            .expect("a string literal must be lexed");
        assert_eq!(
            lit, "foo bar",
            "single-quoted string with `\\<NL>` should drop the backslash, the newline, and the leading whitespace; got {lit:?}",
        );
    }

    #[test]
    fn backslash_not_at_eol_outside_string_still_errors() {
        // `\` followed by a non-newline character at top level keeps
        // emitting `UnexpectedChar('\\')` so existing E0003 callers
        // see no behaviour change.
        let src = "agent main() -> Bool: \\foo\n    return true\n";
        let errs = lex(src).expect_err("must reject stray backslash");
        assert!(
            errs.iter()
                .any(|e| matches!(e.kind, LexErrorKind::UnexpectedChar('\\'))),
            "expected UnexpectedChar('\\\\') in errors; got {errs:?}",
        );
    }

    #[test]
    fn triple_quoted_string_is_not_rewritten() {
        // The feature deliberately does NOT touch triple-quoted
        // strings (they already span lines naturally). Lexing a
        // triple-quoted prompt template — the canonical use case —
        // must continue to work unchanged.
        let src = "prompt summarise(text: String) -> String:\n    \"\"\"Summarise {text} in one sentence.\"\"\"\n";
        let kinds = token_kinds(src);
        let lit = kinds
            .iter()
            .find_map(|k| match k {
                TokKind::StringLit(s) => Some(s.as_str()),
                _ => None,
            })
            .expect("a string literal must be lexed");
        assert_eq!(lit, "Summarise {text} in one sentence.");
    }

    #[test]
    fn outside_string_continuation_works_with_crlf() {
        // Windows-style line endings: `\` + `\r` + `\n` + leading
        // whitespace must also be consumed silently. Real .cor files
        // on Windows hosts can carry CRLF endings.
        let src = "agent main() -> Bool: \\\r\n    return true\n";
        let kinds = token_kinds(src);
        let colon_idx = kinds
            .iter()
            .position(|k| matches!(k, TokKind::Colon))
            .expect("colon present");
        let return_idx = kinds
            .iter()
            .position(|k| matches!(k, TokKind::KwReturn))
            .expect("return present");
        let between = &kinds[colon_idx + 1..return_idx];
        for kind in between {
            assert!(
                !matches!(kind, TokKind::Newline | TokKind::Indent),
                "CRLF continuation must suppress Newline / Indent",
            );
        }
    }
}
