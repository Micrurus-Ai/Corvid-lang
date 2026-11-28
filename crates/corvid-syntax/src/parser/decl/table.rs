//! `table` and `database` declaration parsing (B09).
//!
//! Both words are contextual: they start a declaration only at the top
//! level when a name and `:` follow, so `table` and `database` remain
//! ordinary identifiers everywhere else.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::{describe_token, Parser};
use crate::token::TokKind;
use corvid_ast::{
    DatabaseDecl, DatabaseLocation, Field, Ident, TableDecl, TableRename, TypeDecl, TypeRef, Visibility,
};

impl<'a> Parser<'a> {
    /// Whether the next tokens start `<word> <Name>:`, or, for a table,
    /// `table <Name> in <database>:`.
    pub(super) fn at_contextual_block_decl(&self, word: &str) -> bool {
        let named = matches!(self.peek(), TokKind::Ident(w) if w == word) && matches!(self.peek_ahead(1), TokKind::Ident(_));
        let placed = word == "table"
            && matches!(self.peek_ahead(2), TokKind::KwIn)
            && matches!(self.peek_ahead(3), TokKind::Ident(_))
            && matches!(self.peek_ahead(4), TokKind::Colon);
        named && (matches!(self.peek_ahead(2), TokKind::Colon) || placed)
    }

    pub(super) fn parse_table_decl(&mut self, visibility: Visibility) -> Result<TableDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // table
        let (name, name_span) = self.expect_ident()?;
        // `in library` names the database that holds the rows.
        let database = if matches!(self.peek(), TokKind::KwIn) {
            self.bump();
            let (database, span) = self.expect_ident()?;
            Some(Ident::new(database, span))
        } else {
            None
        };
        self.expect(TokKind::Colon, "`:` after table name")?;
        self.expect_newline()?;
        self.expect_block_start()?;

        let mut keys = Vec::new();
        let mut fields = Vec::new();
        let mut renamed = Vec::new();
        let mut retired = Vec::new();
        let mut unique = Vec::new();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            // `retired name, other` names stored columns the table no
            // longer has. Without `:` it cannot be a field, so `retired`
            // stays an ordinary field name.
            if matches!(self.peek(), TokKind::Ident(w) if w == "retired")
                && matches!(self.peek_ahead(1), TokKind::Ident(_))
            {
                self.bump();
                if let Err(e) = self.parse_retired_columns(&mut retired) {
                    self.errors.push(e);
                    self.sync_to_statement_boundary();
                }
                continue;
            }
            // `unique email` or `unique tenant, slug`: columns no two rows
            // may share. Like `retired`, it is a field name when `:`
            // follows it.
            if matches!(self.peek(), TokKind::Ident(w) if w == "unique")
                && matches!(self.peek_ahead(1), TokKind::Ident(_))
            {
                self.bump();
                let mut columns = Vec::new();
                match self.parse_retired_columns(&mut columns) {
                    Ok(()) => unique.push(columns),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            // `key name: Type` marks a field of the primary key; several
            // marked fields identify a row together. `key` stays an
            // ordinary field name when `:` follows it directly.
            let marks_key = matches!(self.peek(), TokKind::Ident(w) if w == "key")
                && matches!(self.peek_ahead(1), TokKind::Ident(_));
            if marks_key {
                self.bump();
            }
            match self.parse_table_field(&mut renamed) {
                Ok(field) => {
                    if marks_key {
                        keys.push(field.name.clone());
                    }
                    fields.push(field);
                }
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
        // Every row carries the revision the database advances on each
        // write. Its type reference takes the table name's span, which
        // no other type reference uses.
        fields.push(Field {
            name: Ident::new(TableDecl::REVISION_FIELD, name_span),
            ty: TypeRef::Named { name: Ident::new("Int", name_span), span: name_span },
            ui: Vec::new(),
            refinement: None,
            upload: None,
            json_key: None,
            span: name_span,
        });
        let record = TypeDecl {
            name: Ident::new(name, name_span),
            fields,
            variants: Vec::new(),
            alias: None,
            json_case: None,
            visibility,
            span: start.merge(end),
        };
        Ok(TableDecl { record, database, keys, renamed, retired, unique })
    }

    /// A table field, optionally followed by `renamed from <old name>`.
    fn parse_table_field(&mut self, renamed: &mut Vec<TableRename>) -> Result<Field, ParseError> {
        let field = self.parse_field_before_newline()?;
        if matches!(self.peek(), TokKind::Ident(w) if w == "renamed") {
            self.bump();
            match self.peek() {
                TokKind::Ident(w) if w == "from" => {
                    self.bump();
                }
                other => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: describe_token(other),
                            expected: "`from` and the column's old name after `renamed`".into(),
                        },
                        span: self.peek_span(),
                    });
                }
            }
            let (from, from_span) = self.expect_ident()?;
            renamed.push(TableRename { field: field.name.clone(), from: Ident::new(from, from_span) });
        }
        self.expect_newline()?;
        Ok(field)
    }

    /// The column names after `retired` or `unique`, separated by commas.
    fn parse_retired_columns(&mut self, retired: &mut Vec<Ident>) -> Result<(), ParseError> {
        loop {
            let (name, span) = self.expect_ident()?;
            retired.push(Ident::new(name, span));
            if !matches!(self.peek(), TokKind::Comma) {
                break;
            }
            self.bump();
        }
        self.expect_newline()
    }

    pub(super) fn parse_database_decl(&mut self, visibility: Visibility) -> Result<DatabaseDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // database
        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after database name")?;
        self.expect_newline()?;
        self.expect_block_start()?;

        let mut engine = None;
        let mut location = None;
        let mut tls = None;
        let mut connections = None;
        let mut encryption = None;
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            if self.peek_ident_is("encryption") {
                match self.parse_database_encryption() {
                    Ok(item) => encryption = Some(item),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            if let Err(e) = self.parse_database_setting(&mut engine, &mut location, &mut tls, &mut connections) {
                self.errors.push(e);
                self.sync_to_statement_boundary();
            }
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }
        Ok(DatabaseDecl {
            name: Ident::new(name, name_span),
            engine,
            location,
            tls,
            connections,
            visibility,
            encryption,
            span: start.merge(end),
        })
    }

    /// `encryption: key(secret("NAME"))` (encryption at rest K2). A
    /// rotation's `previous:` key is K4, so it is refused by name rather
    /// than read and ignored.
    fn parse_database_encryption(&mut self) -> Result<corvid_ast::DatabaseEncryption, ParseError> {
        let start = self.peek_span();
        self.bump(); // encryption
        self.expect(TokKind::Colon, "`:` after `encryption`")?;
        let expected = |got: &TokKind, span| ParseError {
            kind: ParseErrorKind::UnexpectedToken {
                got: describe_token(got),
                expected: "`key(secret(\"NAME\"))`, the secret holding the database's data key".into(),
            },
            span,
        };
        if !self.peek_ident_is("key") {
            return Err(expected(self.peek(), self.peek_span()));
        }
        self.bump(); // key
        self.expect(TokKind::LParen, "`(` after `key`")?;
        if !self.peek_ident_is("secret") {
            return Err(expected(self.peek(), self.peek_span()));
        }
        self.bump(); // secret
        self.expect(TokKind::LParen, "`(` after `secret`")?;
        let secret = match self.peek().clone() {
            TokKind::StringLit(name) if !name.trim().is_empty() => {
                self.bump();
                name
            }
            other => return Err(expected(&other, self.peek_span())),
        };
        self.expect(TokKind::RParen, "`)` after the secret's name")?;
        if matches!(self.peek(), TokKind::Comma) {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: "a second argument to `key(...)`".into(),
                    expected: "one key for now; reading rows under a `previous:` key while rotating is encryption at rest K4, on the roadmap".into(),
                },
                span: self.peek_span(),
            });
        }
        self.expect(TokKind::RParen, "`)` after `key(...)`")?;
        let span = start.merge(self.prev_span());
        self.expect_newline()?;
        Ok(corvid_ast::DatabaseEncryption { secret, span })
    }

    fn parse_database_setting(
        &mut self,
        engine: &mut Option<Ident>,
        location: &mut Option<DatabaseLocation>,
        tls: &mut Option<Ident>,
        connections: &mut Option<(i64, corvid_ast::Span)>,
    ) -> Result<(), ParseError> {
        let (setting, setting_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after a database setting")?;
        match setting.as_str() {
            "engine" => {
                let (value, span) = self.expect_ident()?;
                *engine = Some(Ident::new(value, span));
            }
            "location" => *location = Some(self.parse_database_location()?),
            "tls" => {
                let (value, span) = self.expect_ident()?;
                *tls = Some(Ident::new(value, span));
            }
            "connections" => {
                let span = self.peek_span();
                match self.peek().clone() {
                    TokKind::Int(count) => {
                        self.bump();
                        *connections = Some((count, span));
                    }
                    other => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: describe_token(&other),
                                expected: "how many connections the pool holds, such as `connections: 10`".into(),
                            },
                            span,
                        })
                    }
                }
            }
            // B09 P11: a table says which database holds it.
            "tables" => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "database setting `tables`".into(),
                        expected: "a table to say where it lives instead: a module's tables live in its one database, and `table Shelf in library:` names another".into(),
                    },
                    span: setting_span,
                })
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("database setting `{other}`"),
                        expected: "`engine`, `location`, `tls`, `connections` or `encryption`".into(),
                    },
                    span: setting_span,
                })
            }
        }
        self.expect_newline()
    }

    /// `"path"` or `env("NAME")`.
    pub(super) fn parse_database_location(&mut self) -> Result<DatabaseLocation, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokKind::StringLit(path) => {
                self.bump();
                Ok(DatabaseLocation::Path(path))
            }
            TokKind::Ident(word) if word == "env" && matches!(self.peek_ahead(1), TokKind::LParen) => {
                self.bump(); // env
                self.bump(); // (
                let name = match self.peek().clone() {
                    TokKind::StringLit(name) => {
                        self.bump();
                        name
                    }
                    other => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: describe_token(&other),
                                expected: "the environment variable's name as text, such as `env(\"LIBRARY_DATABASE\")`".into(),
                            },
                            span: self.peek_span(),
                        })
                    }
                };
                self.expect(TokKind::RParen, "`)` after the environment variable's name")?;
                Ok(DatabaseLocation::Env(name))
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: "a path as text, or `env(\"NAME\")`".into(),
                },
                span,
            }),
        }
    }

    pub(super) fn expect_block_start(&mut self) -> Result<(), ParseError> {
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(self.peek()),
                    expected: "an indented block".into(),
                },
                span: self.peek_span(),
            });
        }
        self.bump();
        Ok(())
    }
}
