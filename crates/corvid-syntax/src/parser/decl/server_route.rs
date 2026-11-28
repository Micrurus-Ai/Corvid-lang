//! `schedule`, `server`, and HTTP-route declaration parsing —
//! cron schedules calling agents/tools, server blocks holding
//! `route GET "/path"` definitions, and the per-route grammar.

use crate::errors::{ParseError, ParseErrorKind};
use crate::parser::{describe_token, Parser};
use crate::token::TokKind;
use corvid_ast::{
    HttpMethod, HttpRouteDecl, Ident, RoutePathParam, RouteResponse, RouteResponseKind,
    ScheduleDecl, ServerDecl, Span, TypeRef,
};

impl<'a> Parser<'a> {
    pub(super) fn parse_schedule_decl(&mut self) -> Result<ScheduleDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokKind::KwSchedule, "`schedule` declaration")?;
        let cron_span = self.peek_span();
        let cron = match self.peek().clone() {
            TokKind::StringLit(cron) => {
                self.bump();
                cron
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(&other),
                        expected: "a cron expression string literal".into(),
                    },
                    span: cron_span,
                });
            }
        };
        self.expect(TokKind::KwZone, "`zone` after schedule cron expression")?;
        let zone_span = self.peek_span();
        let zone = match self.peek().clone() {
            TokKind::StringLit(zone) => {
                self.bump();
                zone
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(&other),
                        expected: "an IANA time zone string literal".into(),
                    },
                    span: zone_span,
                });
            }
        };
        self.expect(TokKind::Arrow, "`->` before schedule target")?;
        let (target, target_span) = self.expect_ident()?;
        self.expect(TokKind::LParen, "`(` after schedule target")?;
        let mut args = Vec::new();
        if !matches!(self.peek(), TokKind::RParen) {
            args.push(self.parse_expr()?);
            while matches!(self.peek(), TokKind::Comma) {
                self.bump();
                if matches!(self.peek(), TokKind::RParen) {
                    break;
                }
                args.push(self.parse_expr()?);
            }
        }
        self.expect(TokKind::RParen, "`)` after schedule target arguments")?;
        let effect_row = self.parse_uses_clause()?;
        let end = self.peek_span();
        self.expect_newline()?;
        Ok(ScheduleDecl {
            cron,
            zone,
            target: Ident::new(target, target_span),
            args,
            effect_row,
            span: start.merge(end),
        })
    }

    pub(super) fn parse_server_decl(&mut self) -> Result<ServerDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // server

        let (name, name_span) = self.expect_ident()?;
        self.expect(TokKind::Colon, "`:` after server name")?;
        self.expect_newline()?;

        if !matches!(self.peek(), TokKind::Indent) {
            return Err(ParseError {
                kind: ParseErrorKind::ExpectedBlock,
                span: self.peek_span(),
            });
        }
        self.bump(); // Indent

        let mut routes = Vec::new();
        while !matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
            self.skip_newlines();
            if matches!(self.peek(), TokKind::Dedent | TokKind::Eof) {
                break;
            }
            let mut upload = None;
            let mut approval = None;
            while matches!(self.peek(), TokKind::At) {
                match self.peek_ahead(1) {
                    TokKind::Ident(name) if name == "upload" => {
                        if upload.is_some() {
                            self.errors.push(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: "duplicate `@upload` route policy".into(),
                                    expected: "at most one `@upload(...)`".into(),
                                },
                                span: self.peek_span(),
                            });
                            self.sync_to_statement_boundary();
                            continue;
                        }
                        match self.parse_upload_spec() {
                            Ok(spec) => upload = Some(spec),
                            Err(error) => {
                                self.errors.push(error);
                                self.sync_to_statement_boundary();
                                continue;
                            }
                        }
                    }
                    TokKind::Ident(name) if name == "approval" => {
                        if approval.is_some() {
                            self.errors.push(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: "duplicate `@approval` route policy".into(),
                                    expected: "at most one `@approval(...)`".into(),
                                },
                                span: self.peek_span(),
                            });
                            self.sync_to_statement_boundary();
                            continue;
                        }
                        match self.parse_approval_spec() {
                            Ok(spec) => approval = Some(spec),
                            Err(error) => {
                                self.errors.push(error);
                                self.sync_to_statement_boundary();
                                continue;
                            }
                        }
                    }
                    other => {
                        self.errors.push(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: format!("route annotation `@{}`", describe_token(other)),
                                expected: "`@upload(...)` or `@approval(...)`".into(),
                            },
                            span: self.peek_span(),
                        });
                        self.sync_to_statement_boundary();
                    }
                }
            }
            let parsed = if self.peek_ident_is("live") {
                self.parse_live_decl()
            } else {
                self.parse_http_route_decl(upload, approval)
            };
            match parsed {
                Ok(route) => routes.push(route),
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

        Ok(ServerDecl {
            name: Ident::new(name, name_span),
            routes,
            span: start.merge(end),
        })
    }

    /// `live "/boards/{board}/cards" -> Card requires permission("board.view"):`
    /// (B14). `live` is contextual, so it stays an ordinary name
    /// elsewhere. It is held as a GET route whose response is the
    /// snapshot, `List<Card>`, so path parameters, `actor` and the
    /// `requires` policy work as a route's do.
    fn parse_live_decl(&mut self) -> Result<HttpRouteDecl, ParseError> {
        let start = self.peek_span();
        self.bump(); // live
        let path_span = self.peek_span();
        let path = match self.peek().clone() {
            TokKind::StringLit(path) => {
                self.bump();
                path
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(&other),
                        expected: "a live path string literal".into(),
                    },
                    span: path_span,
                });
            }
        };
        let (path, path_params, placeholder_errors) = parse_route_path_params(&path, path_span);
        self.errors.extend(placeholder_errors);
        self.expect(TokKind::Arrow, "`->` before the row type a `live` declaration delivers")?;
        let row = self.parse_type_ref()?;
        let row_span = row.span();
        let snapshot = TypeRef::Generic { name: Ident::new("List", row_span), args: vec![row], span: row_span };
        let policy = self.parse_route_policy()?;
        self.expect(TokKind::Colon, "`:` after a `live` declaration")?;
        self.expect_newline()?;
        let body = self.parse_indented_block()?;
        let end = body.span;
        Ok(HttpRouteDecl {
            method: HttpMethod::Get,
            path,
            path_params,
            query_ty: None,
            body_ty: None,
            form_encoded: false,
            upload: None,
            approval: None,
            response: RouteResponse { kind: RouteResponseKind::Live, span: row_span, ty: snapshot },
            effect_row: Default::default(),
            policy,
            verified: None,
            body,
            span: start.merge(end),
        })
    }

    fn parse_http_route_decl(
        &mut self,
        upload: Option<corvid_ast::UploadSpec>,
        approval: Option<corvid_ast::ApprovalSpec>,
    ) -> Result<HttpRouteDecl, ParseError> {
        let start = self.peek_span();
        self.expect(TokKind::KwRoute, "`route` inside a server block")?;
        let method = self.parse_http_method()?;
        let path_span = self.peek_span();
        let path = match self.peek().clone() {
            TokKind::StringLit(path) => {
                self.bump();
                path
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(&other),
                        expected: "a route path string literal".into(),
                    },
                    span: path_span,
                });
            }
        };

        let (path, path_params, placeholder_errors) = parse_route_path_params(&path, path_span);
        self.errors.extend(placeholder_errors);
        let mut query_ty = None;
        let mut body_ty = None;
        let mut form_encoded = false;
        let mut verified = None;
        loop {
            if self.peek_ident_is("query") {
                self.bump();
                if query_ty.is_some() {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "duplicate `query` route clause".into(),
                            expected: "at most one `query Type` clause".into(),
                        },
                        span: self.peek_span(),
                    });
                }
                query_ty = Some(self.parse_type_ref()?);
                continue;
            }
            if self.peek_ident_is("body") || self.peek_ident_is("form") {
                form_encoded = self.peek_ident_is("form");
                self.bump();
                if body_ty.is_some() {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "duplicate `body` route clause".into(),
                            expected: "one request payload, `body Type` or `form Type`".into(),
                        },
                        span: self.peek_span(),
                    });
                }
                body_ty = Some(self.parse_type_ref()?);
                continue;
            }
            if self.peek_ident_is("verified") {
                if verified.is_some() {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "duplicate `verified` route clause".into(),
                            expected: "at most one `verified <scheme>(...)` clause".into(),
                        },
                        span: self.peek_span(),
                    });
                }
                verified = Some(self.parse_webhook_verification()?);
                continue;
            }
            break;
        }

        self.expect(TokKind::Arrow, "`->` before route response")?;
        let (format, response_start) = self.expect_ident()?;
        let (kind, response_ty) = match format.as_str() {
            "json" => (RouteResponseKind::Json, self.parse_type_ref()?),
            "html" => (
                RouteResponseKind::Html,
                TypeRef::Named {
                    name: Ident::new("String", response_start),
                    span: response_start,
                },
            ),
            _ => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format,
                        expected: "`json Type` or `html` after `->`".into(),
                    },
                    span: response_start,
                });
            }
        };
        let response = RouteResponse {
            kind,
            span: response_start.merge(response_ty.span()),
            ty: response_ty,
        };
        let effect_row = self.parse_uses_clause()?;
        let policy = self.parse_route_policy()?;
        self.expect(TokKind::Colon, "`:` after route signature")?;
        self.expect_newline()?;
        let body = self.parse_indented_block()?;
        let end = body.span;

        Ok(HttpRouteDecl {
            method,
            path,
            path_params,
            query_ty,
            body_ty,
            form_encoded,
            upload,
            approval,
            response,
            effect_row,
            policy,
            verified,
            body,
            span: start.merge(end),
        })
    }

    /// `verified <scheme>(secret: "<NAME>", within: <duration>)`, or
    /// `replays: accepted` in place of `within` for a scheme that signs
    /// no time (verified webhooks, B16). Each key at most once; the
    /// checker decides which window a scheme needs.
    fn parse_webhook_verification(&mut self) -> Result<corvid_ast::WebhookVerification, ParseError> {
        let start = self.peek_span();
        self.bump(); // verified
        let (scheme, scheme_span) = self.expect_ident()?;
        self.expect(TokKind::LParen, "`(` after the `verified` scheme")?;
        let mut secret = None;
        let mut replay = None;
        while !matches!(self.peek(), TokKind::RParen | TokKind::Eof) {
            let (key, key_span) = self.expect_ident()?;
            let duplicate = match key.as_str() {
                "secret" => secret.is_some(),
                "within" | "replays" => replay.is_some(),
                _ => false,
            };
            if duplicate {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("a second `{key}` in `verified`"),
                        expected: "`secret` once, and one of `within` or `replays`".into(),
                    },
                    span: key_span,
                });
            }
            self.expect(TokKind::Colon, "`:` after a `verified` key")?;
            match key.as_str() {
                "secret" => match self.peek().clone() {
                    TokKind::StringLit(name) => {
                        self.bump();
                        secret = Some(name);
                    }
                    other => {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: describe_token(&other),
                                expected: "a string naming the signing secret, as in `secret: \"STRIPE_WEBHOOK_SECRET\"`".into(),
                            },
                            span: self.peek_span(),
                        });
                    }
                },
                "within" => {
                    let window_start = self.peek_span();
                    let expected = |got: &TokKind, span| ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: describe_token(got),
                            expected: "a duration, as in `within: 5 minutes`".into(),
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
                    replay = Some(corvid_ast::WebhookReplay::Within { count, unit, span: window_start.merge(self.prev_span()) });
                }
                "replays" => {
                    if !self.peek_ident_is("accepted") {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: describe_token(self.peek()),
                                expected: "`accepted`, as in `replays: accepted`".into(),
                            },
                            span: self.peek_span(),
                        });
                    }
                    self.bump();
                    replay = Some(corvid_ast::WebhookReplay::Accepted { span: key_span.merge(self.prev_span()) });
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("`verified` key `{key}`"),
                            expected: "`secret`, `within`, or `replays`".into(),
                        },
                        span: key_span,
                    });
                }
            }
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokKind::RParen, "`)` after `verified`")?;
        let span = start.merge(self.prev_span());
        let Some(secret) = secret else {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: "`verified` without a `secret`".into(),
                    expected: "`secret: \"<NAME>\"` naming the signing secret".into(),
                },
                span,
            });
        };
        Ok(corvid_ast::WebhookVerification { scheme: Ident::new(scheme, scheme_span), secret, replay, span })
    }

    /// A complete approval policy. Every key is mandatory because
    /// reviewer authority and expiry are consequential deployment
    /// policy and must never come from runtime defaults.
    pub(super) fn parse_approval_spec(&mut self) -> Result<corvid_ast::ApprovalSpec, ParseError> {
        let start = self.peek_span();
        self.bump(); // @
        self.bump(); // approval
        self.expect(TokKind::LParen, "`(` after `@approval`")?;
        let mut role = None;
        let mut risk = None;
        let mut data = None;
        let mut expires_ms = None;
        let mut max_cost_usd = None;
        let mut irreversible = None;
        let mut seen = std::collections::HashSet::new();
        while !matches!(self.peek(), TokKind::RParen | TokKind::Eof) {
            let (key, key_span) = self.expect_ident()?;
            if !seen.insert(key.clone()) {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: format!("duplicate `@approval` key `{key}`"),
                        expected: "each approval-policy key exactly once".into(),
                    },
                    span: key_span,
                });
            }
            self.expect(TokKind::Colon, "`:` after an `@approval` key")?;
            match key.as_str() {
                "role" => role = Some(self.parse_approval_string("role")?),
                "risk" => risk = Some(self.parse_approval_string("risk")?),
                "data" => data = Some(self.parse_approval_string("data")?),
                "expires_ms" => {
                    expires_ms = Some(self.expect_positive_int_literal("a positive expiry")?.0)
                }
                "max_cost_usd" => max_cost_usd = Some(self.parse_cost_literal()?),
                "irreversible" => {
                    irreversible = Some(match self.peek() {
                        TokKind::KwTrue => {
                            self.bump();
                            true
                        }
                        TokKind::KwFalse => {
                            self.bump();
                            false
                        }
                        other => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: describe_token(other),
                                    expected: "`true` or `false`".into(),
                                },
                                span: self.peek_span(),
                            });
                        }
                    })
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("`@approval` key `{key}`"),
                            expected: "`role`, `risk`, `data`, `expires_ms`, `max_cost_usd`, or `irreversible`".into(),
                        },
                        span: key_span,
                    });
                }
            }
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(TokKind::RParen, "`)` after `@approval`")?;
        self.skip_newlines();
        let missing = [
            ("role", role.is_none()),
            ("risk", risk.is_none()),
            ("data", data.is_none()),
            ("expires_ms", expires_ms.is_none()),
            ("max_cost_usd", max_cost_usd.is_none()),
            ("irreversible", irreversible.is_none()),
        ]
        .into_iter()
        .filter_map(|(name, absent)| absent.then_some(name))
        .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!(
                        "incomplete `@approval` policy (missing {})",
                        missing.join(", ")
                    ),
                    expected: "all six approval-policy keys".into(),
                },
                span: start.merge(self.prev_span()),
            });
        }
        Ok(corvid_ast::ApprovalSpec {
            role: role.unwrap(),
            risk: risk.unwrap(),
            data: data.unwrap(),
            expires_ms: expires_ms.unwrap(),
            max_cost_usd: max_cost_usd.unwrap(),
            irreversible: irreversible.unwrap(),
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_approval_string(&mut self, key: &str) -> Result<String, ParseError> {
        match self.peek().clone() {
            TokKind::StringLit(value) if !value.trim().is_empty() => {
                self.bump();
                Ok(value)
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: format!("a non-empty string for `{key}`"),
                },
                span: self.peek_span(),
            }),
        }
    }

    /// `requires <item> [and <item>]*` where each item is
    /// `authenticated`, `role("name")`, or `permission("name")`
    /// (slice 51h). Returns `None` when there is no `requires` clause.
    pub(super) fn parse_route_policy(&mut self) -> Result<Option<corvid_ast::RoutePolicy>, ParseError> {
        if !matches!(self.peek(), TokKind::KwRequires) {
            return Ok(None);
        }
        let start = self.peek_span();
        self.bump(); // requires
        let mut policy = corvid_ast::RoutePolicy::default();
        // `requires anyone`: written, and alone, so an open caller policy
        // is a decision in the source, never an omission.
        if matches!(self.peek(), TokKind::Ident(word) if word == "anyone") {
            self.bump();
            if matches!(self.peek(), TokKind::KwAnd) {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "`requires anyone and ...`".into(),
                        expected: "`requires anyone` on its own, or the requirements without `anyone`".into(),
                    },
                    span: self.peek_span(),
                });
            }
            policy.span = start.merge(self.prev_span());
            return Ok(Some(policy));
        }
        loop {
            let (item, item_span) = self.expect_ident()?;
            match item.as_str() {
                "authenticated" => policy.authenticated = true,
                "role" => {
                    policy.roles.push(self.parse_policy_string_arg("role")?);
                }
                "permission" => {
                    policy
                        .permissions
                        .push(self.parse_policy_string_arg("permission")?);
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("route requirement `{item}`"),
                            expected: "`anyone`, `authenticated`, `role(\"...\")`, or `permission(\"...\")`"
                                .into(),
                        },
                        span: item_span,
                    });
                }
            }
            // `and` chains multiple requirements; anything else ends it.
            if matches!(self.peek(), TokKind::KwAnd) {
                self.bump();
                continue;
            }
            break;
        }
        policy.span = start.merge(self.prev_span());
        Ok(Some(policy))
    }

    fn parse_policy_string_arg(&mut self, clause: &str) -> Result<String, ParseError> {
        self.expect(TokKind::LParen, "`(` after the route requirement")?;
        let value = match self.peek().clone() {
            TokKind::StringLit(s) => {
                self.bump();
                s
            }
            other => {
                return Err(ParseError {
                    kind: ParseErrorKind::UnexpectedToken {
                        got: describe_token(&other),
                        expected: format!("a string literal naming the {clause}"),
                    },
                    span: self.peek_span(),
                });
            }
        };
        self.expect(TokKind::RParen, "`)` after the route requirement")?;
        Ok(value)
    }

    pub(super) fn parse_http_method(&mut self) -> Result<HttpMethod, ParseError> {
        let span = self.peek_span();
        let (method, method_span) = self.expect_ident()?;
        match method.as_str() {
            "GET" => Ok(HttpMethod::Get),
            "POST" => Ok(HttpMethod::Post),
            "PUT" => Ok(HttpMethod::Put),
            "PATCH" => Ok(HttpMethod::Patch),
            "DELETE" => Ok(HttpMethod::Delete),
            "HEAD" => Ok(HttpMethod::Head),
            "OPTIONS" => Ok(HttpMethod::Options),
            _ => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("HTTP method `{method}`"),
                    expected: "`GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`, or `OPTIONS`".into(),
                },
                span: span.merge(method_span),
            }),
        }
    }
}

/// A route or live path's placeholders, and the path as the router and
/// the contract carry it: `"/boards/{board: Int}/cards"` declares
/// `board` an `Int` and is served as `"/boards/{board}/cards"`. A
/// placeholder without a type, `{board}`, is a `String`. Anything else
/// between braces is an error naming the two forms, rather than text the
/// router would treat as a parameter of a name no handler reads.
fn parse_route_path_params(
    path: &str,
    path_span: Span,
) -> (String, Vec<RoutePathParam>, Vec<ParseError>) {
    // The path text starts one past the literal's opening quote.
    let text_start = path_span.start + 1;
    let mut canonical = String::with_capacity(path.len());
    let mut params = Vec::new();
    let mut errors = Vec::new();
    let mut offset = 0usize;
    while let Some(open_rel) = path[offset..].find('{') {
        let open = offset + open_rel;
        canonical.push_str(&path[offset..open]);
        let Some(close_rel) = path[open + 1..].find('}') else {
            errors.push(placeholder_error(
                &path[open..],
                Span::new(text_start + open, text_start + path.len()),
            ));
            canonical.push_str(&path[open..]);
            offset = path.len();
            break;
        };
        let close = open + 1 + close_rel;
        let inside = &path[open + 1..close];
        let span = Span::new(text_start + open, text_start + close + 1);
        let (name, ty) = match inside.split_once(':') {
            Some((name, ty)) => (name.trim(), Some(ty.trim())),
            None => (inside.trim(), None),
        };
        let name_ok = is_route_param_name(name);
        let ty_ok = ty.map_or(true, is_route_param_name);
        if name_ok && ty == Some("String") {
            // One way to say it: an untyped placeholder is a String.
            errors.push(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: format!("path placeholder `{{{name}: String}}`"),
                    expected: format!("`{{{name}}}`: a placeholder without a type is already a `String`"),
                },
                span,
            });
            canonical.push('{');
            canonical.push_str(name);
            canonical.push('}');
        } else if name_ok && ty_ok {
            let name_at = text_start + open + 1 + inside.find(name).unwrap_or(0);
            let name_span = Span::new(name_at, name_at + name.len());
            let ty = match ty {
                Some(ty) => {
                    let ty_at = text_start + open + 1 + inside.rfind(ty).unwrap_or(0);
                    let ty_span = Span::new(ty_at, ty_at + ty.len());
                    TypeRef::Named { name: Ident::new(ty.to_string(), ty_span), span: ty_span }
                }
                None => TypeRef::Named { name: Ident::new("String", name_span), span: name_span },
            };
            params.push(RoutePathParam { name: Ident::new(name.to_string(), name_span), ty, span });
            canonical.push('{');
            canonical.push_str(name);
            canonical.push('}');
        } else {
            errors.push(placeholder_error(&path[open..=close], span));
            canonical.push_str(&path[open..=close]);
        }
        offset = close + 1;
    }
    canonical.push_str(&path[offset..]);
    (canonical, params, errors)
}

fn placeholder_error(found: &str, span: Span) -> ParseError {
    ParseError {
        kind: ParseErrorKind::UnexpectedToken {
            got: format!("path placeholder `{found}`"),
            expected: "`{name}` or `{name: Type}`, such as `{board}` or `{board: Int}`".into(),
        },
        span,
    }
}

/// A placeholder's name or type: letters, digits and `_`, not starting
/// with a digit.
fn is_route_param_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}
