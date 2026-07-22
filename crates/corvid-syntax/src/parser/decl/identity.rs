//! `identity Name:` declaration parsing (slice 51g) — the
//! authenticated-user surface: the identity providers a program
//! accepts and its login-session configuration.
//!
//! The block is deliberately small and declarative. Every OAuth
//! safe-default is the default; the parser records the raw choices and
//! the checker enforces that any unsafe cookie option carries the
//! loud `insecure_opt_out: true` acknowledgement.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::Parser;
use crate::token::TokKind;
use corvid_ast::{
    EmailMatchPolicy, Ident, IdentityDecl, IdentityProvider, LinkingConfig, ProviderKind, SameSite,
    SessionConfig,
};

impl<'a> Parser<'a> {
    pub(super) fn parse_identity_decl(&mut self) -> Result<IdentityDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // identity

        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after identity name")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut providers = Vec::new();
        let mut session = None;
        let mut linking = None;
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            if self.peek_ident_is("provider") {
                match self.parse_identity_provider() {
                    Ok(p) => providers.push(p),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            if matches!(self.peek(), TokKind::KwSession) {
                match self.parse_session_config() {
                    Ok(s) => session = Some(s),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            if self.peek_ident_is("linking") {
                match self.parse_linking_config() {
                    Ok(l) => linking = Some(l),
                    Err(e) => {
                        self.errors.push(e);
                        self.sync_to_statement_boundary();
                    }
                }
                continue;
            }
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: crate::parser::describe_token(self.peek()),
                    expected: "`provider ...`, `session:`, or `linking:` inside an identity block"
                        .into(),
                },
                span: self.peek_span(),
            });
        }
        let end = self.peek_span();
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }

        Ok(IdentityDecl {
            name: Ident::new(name, name_span),
            providers,
            session,
            linking,
            span: start.merge(end),
        })
    }

    /// `linking:` sub-block (slice 51i): only the `email_match` policy
    /// and its `verified_domains` are configurable. The explicit
    /// confirmation flow is structural and not expressible as off.
    fn parse_linking_config(&mut self) -> Result<LinkingConfig, ParseError> {
        let start = self.peek_span();
        self.bump(); // linking
        self.expect(TokKind::Colon, "`:` after `linking`")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut cfg = LinkingConfig::default();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            let (key, key_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a linking key")?;
            match key.as_str() {
                "email_match" => {
                    let (v, v_span) = self.expect_ident()?;
                    cfg.email_match = match v.as_str() {
                        "never" => EmailMatchPolicy::Never,
                        "verified_domain" => EmailMatchPolicy::VerifiedDomain,
                        _ => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: format!("`{v}`"),
                                    expected: "`never` or `verified_domain`".into(),
                                },
                                span: v_span,
                            });
                        }
                    };
                }
                "verified_domains" => match self.peek().clone() {
                    TokKind::StringLit(s) => {
                        self.bump();
                        cfg.verified_domains.extend(
                            s.split(',')
                                .map(|d| d.trim().to_string())
                                .filter(|d| !d.is_empty()),
                        );
                    }
                    _ => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: "a non-string `verified_domains` value".into(),
                                expected: "a comma-separated domain string literal".into(),
                            },
                            span: self.peek_span(),
                        });
                    }
                },
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("linking key `{key}`"),
                            expected: "`email_match` or `verified_domains`".into(),
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
        cfg.span = start.merge(end);
        Ok(cfg)
    }

    /// `provider <builtin>` or `provider oidc "<url>" as <alias>`.
    fn parse_identity_provider(&mut self) -> Result<IdentityProvider, ParseError> {
        let start = self.peek_span();
        self.bump(); // provider
        let (name, name_span) = self.expect_ident()?;
        let kind = if name == "oidc" {
            let discovery_url = match self.peek().clone() {
                TokKind::StringLit(url) => {
                    self.bump();
                    url
                }
                other => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: crate::parser::describe_token(&other),
                            expected: "a discovery-URL string after `provider oidc`".into(),
                        },
                        span: self.peek_span(),
                    });
                }
            };
            self.expect(TokKind::KwAs, "`as <alias>` after the OIDC discovery URL")?;
            let (alias, alias_span) = self.expect_ident()?;
            ProviderKind::Oidc {
                discovery_url,
                alias: Ident::new(alias, alias_span),
            }
        } else {
            match ProviderKind::from_builtin_name(&name) {
                Some(kind) => kind,
                None => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("unknown provider `{name}`"),
                            expected: "`google`, `github`, `microsoft`, `apple`, `discord`, `slack`, or `oidc \"url\" as alias`".into(),
                        },
                        span: name_span,
                    });
                }
            }
        };
        self.expect_newline()?;
        Ok(IdentityProvider {
            kind,
            span: start.merge(self.prev_span()),
        })
    }

    /// `session:` sub-block with lifetime / same_site / cookie flags /
    /// rotation.
    fn parse_session_config(&mut self) -> Result<SessionConfig, ParseError> {
        let start = self.peek_span();
        self.bump(); // session
        self.expect(TokKind::Colon, "`:` after `session`")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut cfg = SessionConfig::default();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            let (key, key_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a session key")?;
            match key.as_str() {
                "lifetime" => {
                    cfg.lifetime_secs = Some(self.parse_duration_secs()?);
                }
                "same_site" => {
                    let (v, v_span) = self.expect_ident()?;
                    cfg.cookie.same_site = match v.as_str() {
                        "strict" => SameSite::Strict,
                        "lax" => SameSite::Lax,
                        "none" => SameSite::None,
                        _ => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: format!("`{v}`"),
                                    expected: "`strict`, `lax`, or `none`".into(),
                                },
                                span: v_span,
                            });
                        }
                    };
                }
                "secure" => cfg.cookie.secure = self.parse_bool_value()?,
                "http_only" => cfg.cookie.http_only = self.parse_bool_value()?,
                "rotate_on_privilege_change" => {
                    cfg.rotate_on_privilege_change = self.parse_bool_value()?;
                }
                "insecure_opt_out" => cfg.cookie.insecure_opt_out = self.parse_bool_value()?,
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("session key `{key}`"),
                            expected: "`lifetime`, `same_site`, `secure`, `http_only`, `rotate_on_privilege_change`, or `insecure_opt_out`".into(),
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
        cfg.span = start.merge(end);
        Ok(cfg)
    }

    fn parse_bool_value(&mut self) -> Result<bool, ParseError> {
        match self.peek().clone() {
            TokKind::KwTrue => {
                self.bump();
                Ok(true)
            }
            TokKind::KwFalse => {
                self.bump();
                Ok(false)
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: crate::parser::describe_token(&other),
                    expected: "`true` or `false`".into(),
                },
                span: self.peek_span(),
            }),
        }
    }

    /// `<int><unit>` where unit is `s`/`m`/`h`/`d`, returned as seconds.
    fn parse_duration_secs(&mut self) -> Result<u64, ParseError> {
        let (value, span) = self.expect_positive_int_literal("a duration value")?;
        let (unit, _) = self.expect_ident()?;
        let secs = match unit.as_str() {
            "s" => value,
            "m" => value.saturating_mul(60),
            "h" => value.saturating_mul(3600),
            "d" => value.saturating_mul(86_400),
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("duration unit `{unit}`"),
                        expected: "`s`, `m`, `h`, or `d`".into(),
                    },
                    span,
                });
            }
        };
        Ok(secs)
    }
}
