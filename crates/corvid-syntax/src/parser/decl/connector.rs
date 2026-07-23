//! `connector <name>:` declaration parsing (slice 52g) — the
//! protocol-typed integration surface.
//!
//! A connector declares its base URL, authentication (via `secret(...)`
//! references — never a literal), and reliability posture, then a set of
//! `operation`s. Each operation is a tool with a declarative HTTP body:
//! an HTTP method + path, an optional `body`/`form` parameter, an effect
//! row so budgets / approval / replay / taint compose, and `on status`
//! mappings from response codes to typed errors.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::{describe_token, Parser};
use crate::token::TokKind;
use corvid_ast::{
    BodyEncoding, ConnectorAuth, ConnectorDecl, ConnectorMode, Effect, Ident, OperationBody,
    OperationDecl, RateLimitConfig, SecretRef, StatusErrorMapping, Visibility,
};

impl<'a> Parser<'a> {
    pub(super) fn parse_connector_decl(&mut self) -> Result<ConnectorDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // connector

        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after connector name")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut base_url: Option<String> = None;
        let mut auth: Option<ConnectorAuth> = None;
        let mut retry: Option<u64> = None;
        let mut rate_limit: Option<RateLimitConfig> = None;
        let mut circuit_breaker: Option<u64> = None;
        let mut modes: Vec<ConnectorMode> = Vec::new();
        let mut operations = Vec::new();

        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            if self.peek_ident_is("operation") {
                operations.push(self.parse_operation_decl()?);
                continue;
            }
            // `retry` is a reserved keyword (also used by `@retry`), so it
            // does not lex as an identifier — handle it explicitly.
            if matches!(self.peek(), TokKind::KwRetry) {
                self.bump();
                self.expect(TokKind::Colon, "`:` after `retry`")?;
                retry = Some(self.parse_u64_literal("retry count")?);
                self.expect_newline()?;
                continue;
            }
            let (key, key_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a connector key")?;
            match key.as_str() {
                "base_url" => base_url = Some(self.expect_string_literal("base_url")?.0),
                "auth" => auth = Some(self.parse_connector_auth()?),
                "rate_limit" => rate_limit = Some(self.parse_rate_limit()?),
                "circuit_breaker" => {
                    circuit_breaker = Some(self.parse_u64_literal("circuit_breaker threshold")?)
                }
                "modes" => modes = self.parse_connector_modes()?,
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("connector key `{key}`"),
                            expected: "`base_url`, `auth`, `retry`, `rate_limit`, `circuit_breaker`, `modes`, or `operation ...`".into(),
                        },
                        span: key_span,
                    });
                }
            }
            self.expect_newline()?;
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }

        let base_url = base_url.ok_or_else(|| ParseError {
            kind: ParseErrorKind::UnexpectedToken {
                got: "a `connector` block without `base_url`".into(),
                expected: "`base_url: \"https://...\"`".into(),
            },
            span: start.merge(end),
        })?;

        Ok(ConnectorDecl {
            name: Ident::new(name, name_span),
            base_url,
            auth,
            retry,
            rate_limit,
            circuit_breaker,
            modes,
            operations,
            visibility: Visibility::Private,
            span: start.merge(end),
        })
    }

    /// `modes: [mock, replay, real]` — the allowed execution modes.
    /// An empty or absent list is caught by the checker (a connector
    /// MUST declare its allowed modes — there is no default).
    fn parse_connector_modes(&mut self) -> Result<Vec<ConnectorMode>, ParseError> {
        self.expect(TokKind::LBracket, "`[` to open the modes list")?;
        let mut modes = Vec::new();
        while !matches!(self.peek(), TokKind::RBracket | TokKind::Eof) {
            // `mock` (KwMock) and `replay` (KwReplay) are reserved
            // keywords; `real` lexes as an identifier. Match all three
            // by token kind.
            let mode_span = self.peek_span();
            let mode = match self.peek() {
                TokKind::KwMock => {
                    self.bump();
                    ConnectorMode::Mock
                }
                TokKind::KwReplay => {
                    self.bump();
                    ConnectorMode::Replay
                }
                _ if self.peek_ident_is("real") => {
                    self.bump();
                    ConnectorMode::Real
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: describe_token(self.peek()),
                            expected: "`mock`, `replay`, or `real` in the modes list".into(),
                        },
                        span: mode_span,
                    });
                }
            };
            modes.push(mode);
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokKind::RBracket, "`]` to close the modes list")?;
        Ok(modes)
    }

    /// `auth: bearer(secret("X"))` | `header("Name", secret("X"))` |
    /// `basic(secret("U"), secret("P"))`.
    fn parse_connector_auth(&mut self) -> Result<ConnectorAuth, ParseError> {
        let (kind, kind_span) = self.expect_ident()?;
        self.expect(TokKind::LParen, "`(` after auth scheme")?;
        let auth = match kind.as_str() {
            "bearer" => ConnectorAuth::Bearer(self.parse_secret_ref()?),
            "header" => {
                let name = self.expect_string_literal("header name")?.0;
                self.expect(TokKind::Comma, "`,` between header name and value")?;
                let value = self.parse_secret_ref()?;
                ConnectorAuth::Header { name, value }
            }
            "basic" => {
                let username = self.parse_secret_ref()?;
                self.expect(TokKind::Comma, "`,` between basic username and password")?;
                let password = self.parse_secret_ref()?;
                ConnectorAuth::Basic { username, password }
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("auth scheme `{kind}`"),
                        expected: "`bearer(...)`, `header(...)`, or `basic(...)`".into(),
                    },
                    span: kind_span,
                });
            }
        };
        self.expect(TokKind::RParen, "`)` to close the auth scheme")?;
        Ok(auth)
    }

    /// `secret("NAME")` — a reference to a named secret, never a literal.
    fn parse_secret_ref(&mut self) -> Result<SecretRef, ParseError> {
        let span = self.peek_span();
        let (kw, kw_span) = self.expect_ident()?;
        if kw != "secret" {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("`{kw}`"),
                    expected: "`secret(\"NAME\")` — credentials are never literals".into(),
                },
                span: kw_span,
            });
        }
        self.expect(TokKind::LParen, "`(` after `secret`")?;
        let name = self.expect_string_literal("secret name")?.0;
        self.expect(TokKind::RParen, "`)` to close `secret(...)`")?;
        Ok(SecretRef {
            name,
            span: span.merge(self.peek_span()),
        })
    }

    /// `rate_limit: <limit> per <window>s`.
    fn parse_rate_limit(&mut self) -> Result<RateLimitConfig, ParseError> {
        let limit = self.parse_u64_literal("rate limit")?;
        // `per` is a contextual keyword.
        if !self.peek_ident_is("per") {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(self.peek()),
                    expected: "`per` in `rate_limit: <n> per <seconds>s`".into(),
                },
                span: self.peek_span(),
            });
        }
        self.bump(); // per
        let window_secs = self.parse_rate_limit_window_secs()?;
        Ok(RateLimitConfig { limit, window_secs })
    }

    /// A `<n>s` duration in seconds (the rate-limit window).
    fn parse_rate_limit_window_secs(&mut self) -> Result<u64, ParseError> {
        let secs = self.parse_u64_literal("rate limit window")?;
        // Accept a trailing `s` unit if the lexer produced it as a
        // separate identifier (`60 s`) or fused (`60s` lexes the number,
        // then an ident `s`).
        if self.peek_ident_is("s") {
            self.bump();
        }
        Ok(secs)
    }

    /// `operation <name>(<params>) -> <Ty> [dangerous] [uses ...]:`
    /// followed by the request line and optional `on status` mappings.
    fn parse_operation_decl(&mut self) -> Result<OperationDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // operation

        let (name, name_span) = self.expect_ident()?;
        let params = self.parse_params()?;
        self.expect(TokKind::Arrow, "`->` before an operation return type")?;
        let return_ty = self.parse_type_ref()?;
        let effect = if matches!(self.peek(), TokKind::KwDangerous) {
            self.bump();
            Effect::Dangerous
        } else {
            Effect::Safe
        };
        let effect_row = self.parse_uses_clause()?;
        self.expect(TokKind::Colon, "`:` after an operation signature")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        // The request line: `<METHOD> "<path>" [body <param> | form <param>]`.
        let method = self.parse_http_method()?;
        let path = self.expect_string_literal("operation path")?.0;
        let body = match () {
            _ if self.peek_ident_is("body") => {
                self.bump();
                let (param, param_span) = self.expect_ident()?;
                Some(OperationBody {
                    param: Ident::new(param, param_span),
                    encoding: BodyEncoding::Json,
                })
            }
            _ if self.peek_ident_is("form") => {
                self.bump();
                let (param, param_span) = self.expect_ident()?;
                Some(OperationBody {
                    param: Ident::new(param, param_span),
                    encoding: BodyEncoding::Form,
                })
            }
            _ => None,
        };
        self.expect_newline()?;

        // Optional `on status <code> -> <Variant>` lines and an
        // optional `mock: <expr>` line (the mock-mode payload).
        let mut error_map = Vec::new();
        let mut mock: Option<corvid_ast::Expr> = None;
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            // `mock` is a reserved keyword (also the `mock` test-block
            // decl), so it does not lex as an identifier — match it
            // explicitly, like `retry`/`on`.
            if matches!(self.peek(), TokKind::KwMock) {
                self.bump(); // mock
                self.expect(TokKind::Colon, "`:` after `mock`")?;
                if mock.is_some() {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "a second `mock:` on one operation".into(),
                            expected: "at most one `mock:` payload per operation".into(),
                        },
                        span: self.peek_span(),
                    });
                }
                mock = Some(self.parse_expr()?);
                self.expect_newline()?;
                continue;
            }
            error_map.push(self.parse_status_mapping()?);
            self.expect_newline()?;
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }

        Ok(OperationDecl {
            name: Ident::new(name, name_span),
            params,
            return_ty,
            effect,
            effect_row,
            method,
            path,
            body,
            error_map,
            mock,
            span: start.merge(end),
        })
    }

    /// `on status <code> -> <Variant>`.
    fn parse_status_mapping(&mut self) -> Result<StatusErrorMapping, ParseError> {
        let start = self.peek_span();
        // `on` is a reserved keyword (`KwOn`), not an identifier.
        if !matches!(self.peek(), TokKind::KwOn) {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(self.peek()),
                    expected: "`on status <code> -> <Variant>` inside an operation".into(),
                },
                span: self.peek_span(),
            });
        }
        self.bump(); // on
        if !self.peek_ident_is("status") {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(self.peek()),
                    expected: "`status` after `on`".into(),
                },
                span: self.peek_span(),
            });
        }
        self.bump(); // status
        let code = self.parse_u64_literal("HTTP status code")?;
        if !(100..=599).contains(&code) {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("status `{code}`"),
                    expected: "an HTTP status code in 100..=599".into(),
                },
                span: start.merge(self.peek_span()),
            });
        }
        self.expect(TokKind::Arrow, "`->` after `on status <code>`")?;
        let (variant, variant_span) = self.expect_ident()?;
        Ok(StatusErrorMapping {
            status: code as u16,
            variant: Ident::new(variant, variant_span),
            span: start.merge(variant_span),
        })
    }
}
