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
    EmailMatchPolicy, FirstLoginPolicy, Ident, IdentityDecl, IdentityProvider, LinkingConfig,
    ProviderKind, ProvisioningPolicy, RoleDecl, SameSite, SessionConfig, TenantAssignment,
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
        let mut provisioning = None;
        let mut roles = Vec::new();
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
            if self.peek_ident_is("provisioning") {
                match self.parse_provisioning_policy() {
                    Ok(p) => provisioning = Some(p),
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
            if self.peek_ident_is("roles") {
                match self.parse_roles_block() {
                    Ok(r) => roles = r,
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
                    expected:
                        "`provider ...`, `session:`, `linking:`, `provisioning:`, or `roles:` inside an identity block"
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
            provisioning,
            roles,
            span: start.merge(end),
        })
    }

    /// `roles:` sub-block (slice 52f). Each line is
    /// `name: "perm, perm"` — a role and the comma-separated permissions
    /// it grants. `requires role/permission` clauses must reference names
    /// declared here.
    fn parse_roles_block(&mut self) -> Result<Vec<RoleDecl>, ParseError> {
        self.bump(); // roles
        self.expect(TokKind::Colon, "`:` after `roles`")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut roles = Vec::new();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            let (name, name_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a role name")?;
            let raw = match self.peek().clone() {
                TokKind::StringLit(s) => {
                    self.bump();
                    s
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: crate::parser::describe_token(self.peek()),
                            expected: "a comma-separated permission string, e.g. `\"refund:write, user:read\"`".into(),
                        },
                        span: self.peek_span(),
                    });
                }
            };
            let permissions: Vec<String> = raw
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
            roles.push(RoleDecl {
                name,
                permissions,
                span: name_span,
            });
            self.expect_newline()?;
        }
        if matches!(self.peek(), TokKind::Dedent) {
            self.bump();
        }
        Ok(roles)
    }

    /// `provisioning:` sub-block (slice 52e). Both `first_login` and
    /// `tenant` are required — a first-login provisioning policy is
    /// never a silent default. Parses `approval_required` too so the
    /// checker can reject it with a clear "not executable until 52f"
    /// message rather than the value being silently unavailable.
    fn parse_provisioning_policy(&mut self) -> Result<ProvisioningPolicy, ParseError> {
        let start = self.peek_span();
        self.bump(); // provisioning
        self.expect(TokKind::Colon, "`:` after `provisioning`")?;
        self.expect_newline()?;
        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut first_login: Option<FirstLoginPolicy> = None;
        let mut tenant: Option<TenantAssignment> = None;
        let mut default_role: Option<String> = None;
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            let (key, key_span) = self.expect_ident()?;
            self.expect(TokKind::Colon, "`:` after a provisioning key")?;
            match key.as_str() {
                "default_role" => {
                    let (role, _) = self.expect_ident()?;
                    default_role = Some(role);
                }
                "first_login" => {
                    let (v, v_span) = self.expect_ident()?;
                    first_login = Some(match v.as_str() {
                        "open" => FirstLoginPolicy::Open,
                        "invited" => FirstLoginPolicy::Invited,
                        "approval_required" => FirstLoginPolicy::ApprovalRequired,
                        _ => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: format!("`{v}`"),
                                    expected: "`open`, `invited`, or `approval_required`".into(),
                                },
                                span: v_span,
                            });
                        }
                    });
                }
                "tenant" => {
                    tenant = Some(self.parse_tenant_assignment()?);
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("provisioning key `{key}`"),
                            expected: "`first_login`, `tenant`, or `default_role`".into(),
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

        let first_login = first_login.ok_or_else(|| ParseError {
            kind: ParseErrorKind::UnexpectedToken {
                got: "a `provisioning:` block without `first_login`".into(),
                expected: "`first_login: open | invited`".into(),
            },
            span: start.merge(end),
        })?;
        // `approval_required` needs no tenant here — the checker rejects
        // it outright (not executable until 52f), so a missing tenant
        // must not mask that clearer error.
        let tenant = match (&first_login, tenant) {
            (_, Some(t)) => t,
            (FirstLoginPolicy::ApprovalRequired, None) => TenantAssignment::Fixed(String::new()),
            (_, None) => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "a `provisioning:` block without `tenant`".into(),
                        expected:
                            "`tenant: fixed(\"...\")`, `from_invitation`, or `from_claim(\"...\") allow \"...\"`"
                                .into(),
                    },
                    span: start.merge(end),
                });
            }
        };
        Ok(ProvisioningPolicy {
            first_login,
            tenant,
            default_role,
            span: start.merge(end),
        })
    }

    /// `tenant: fixed("id")` | `from_invitation` | `from_claim("c") allow "a, b"`.
    fn parse_tenant_assignment(&mut self) -> Result<TenantAssignment, ParseError> {
        let (kind, kind_span) = self.expect_ident()?;
        match kind.as_str() {
            "fixed" => {
                let id = self.parse_paren_string_arg("fixed")?;
                Ok(TenantAssignment::Fixed(id))
            }
            "from_invitation" => Ok(TenantAssignment::FromInvitation),
            "from_claim" => {
                let claim = self.parse_paren_string_arg("from_claim")?;
                if !self.peek_ident_is("allow") {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: crate::parser::describe_token(self.peek()),
                            expected: "`allow \"...\"` allowlist after `from_claim(\"...\")`".into(),
                        },
                        span: self.peek_span(),
                    });
                }
                self.bump(); // allow
                let allow = match self.peek().clone() {
                    TokKind::StringLit(s) => {
                        self.bump();
                        s
                    }
                    _ => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: "a non-string allowlist".into(),
                                expected: "a comma-separated allowlist string literal".into(),
                            },
                            span: self.peek_span(),
                        });
                    }
                };
                let allowlist: Vec<String> = allow
                    .split(',')
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
                Ok(TenantAssignment::ClaimMapping { claim, allowlist })
            }
            _ => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("tenant source `{kind}`"),
                    expected: "`fixed(\"...\")`, `from_invitation`, or `from_claim(\"...\") allow \"...\"`"
                        .into(),
                },
                span: kind_span,
            }),
        }
    }

    fn parse_paren_string_arg(&mut self, name: &str) -> Result<String, ParseError> {
        self.expect(TokKind::LParen, "`(` after tenant source")?;
        let value = match self.peek().clone() {
            TokKind::StringLit(s) => {
                self.bump();
                s
            }
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("a non-string `{name}` argument"),
                        expected: "a string literal".into(),
                    },
                    span: self.peek_span(),
                });
            }
        };
        self.expect(TokKind::RParen, "`)` after the tenant source argument")?;
        Ok(value)
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
