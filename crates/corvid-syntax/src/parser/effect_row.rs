//! Effect-row + prompt-body helpers — `uses` clause, streaming
//! modifiers (min_confidence / max_tokens / with_backpressure), and
//! `@cost`/`@budget` style constraint annotations.
//!
//! Shared by prompt and agent decls; called from parser/prompt.rs
//! (prompt bodies) and parser/decl.rs (agent / extend constraints).
//!
//! Extracted from `parser.rs` as part of Phase 20i responsibility
//! decomposition.

use super::{describe_token, Parser};
use crate::errors::{ParseError, ParseErrorKind};
use crate::token::TokKind;
use corvid_ast::{
    AgentAttribute, Backoff, BackpressurePolicy, EffectConstraint, EffectRef, EffectRow, Ident,
    PromptStreamSettings, Span,
};

impl<'a> Parser<'a> {
    pub(super) fn parse_uses_clause(&mut self) -> Result<EffectRow, ParseError> {
        if !matches!(self.peek(), TokKind::KwUses) {
            return Ok(EffectRow::default());
        }
        let start = self.peek_span();
        self.bump(); // uses
        let mut effects = Vec::new();
        let (first_name, first_span) = self.expect_ident()?;
        effects.push(EffectRef {
            name: Ident::new(first_name, first_span),
            span: first_span,
        });
        while matches!(self.peek(), TokKind::Comma) {
            self.bump();
            let (name, span) = self.expect_ident()?;
            effects.push(EffectRef {
                name: Ident::new(name, span),
                span,
            });
        }
        let end = effects.last().map(|e| e.span).unwrap_or(start);
        Ok(EffectRow {
            effects,
            span: start.merge(end),
        })
    }

    pub(super) fn parse_prompt_stream_settings(&mut self) -> Result<PromptStreamSettings, ParseError> {
        let mut settings = PromptStreamSettings::default();
        while self.peek_ident_is("with") {
            self.bump(); // with
            let (name, span) = self.expect_ident()?;
            match name.as_str() {
                "min_confidence" => {
                    settings.min_confidence = Some(self.parse_confidence_literal()?);
                }
                "max_tokens" => {
                    let (max_tokens, _) =
                        self.expect_positive_int_literal("a positive max token count")?;
                    settings.max_tokens = Some(max_tokens);
                }
                // Sampling overrides (slice 46a). Ranges enforced at
                // parse so the error lands on the literal.
                "temperature" => {
                    let (value, value_span) = self.expect_float_literal("a temperature value")?;
                    if !(0.0..=2.0).contains(&value) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: format!("temperature {value}"),
                                expected: "a temperature between 0.0 and 2.0".into(),
                            },
                            span: value_span,
                        });
                    }
                    settings.temperature = Some(value);
                }
                "top_p" => {
                    let (value, value_span) = self.expect_float_literal("a top_p value")?;
                    if !(0.0..=1.0).contains(&value) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: format!("top_p {value}"),
                                expected: "a top_p between 0.0 and 1.0".into(),
                            },
                            span: value_span,
                        });
                    }
                    settings.top_p = Some(value);
                }
                "repair" => {
                    let (attempts, _) =
                        self.expect_positive_int_literal("a positive repair attempt count")?;
                    settings.repair = Some(attempts);
                }
                "judged" => {
                    let (criteria, _) = self.expect_string_literal("the judge criteria")?;
                    match self.peek() {
                        TokKind::Ident(w) if w == "min" => {
                            self.bump();
                        }
                        other => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: describe_token(other),
                                    expected: "`min <score>` after the judge criteria".into(),
                                },
                                span: self.peek_span(),
                            });
                        }
                    }
                    let (min, min_span) = self.expect_float_literal("a minimum judge score")?;
                    if !(0.0..=1.0).contains(&min) {
                        return Err(ParseError {
                            kind: ParseErrorKind::UnexpectedToken {
                                got: format!("min {min}"),
                                expected: "a judge score between 0.0 and 1.0".into(),
                            },
                            span: min_span,
                        });
                    }
                    settings.judged = Some(corvid_ast::JudgedGuard { criteria, min });
                }
                "backpressure" => {
                    settings.backpressure = Some(self.parse_backpressure_policy()?);
                }
                "escalate_to" => {
                    let (model, model_span) = self.expect_ident()?;
                    settings.escalate_to = Some(Ident::new(model, model_span));
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("identifier `{name}`"),
                            expected: "`min_confidence`, `max_tokens`, `temperature`, `top_p`, `repair`, `judged`, `backpressure`, or `escalate_to`".into(),
                        },
                        span,
                    });
                }
            }
            self.expect_newline()?;
        }
        Ok(settings)
    }

    pub(super) fn parse_backpressure_policy(&mut self) -> Result<BackpressurePolicy, ParseError> {
        let span = self.peek_span();
        match self.peek().clone() {
            TokKind::Ident(name) if name == "unbounded" => {
                self.bump();
                Ok(BackpressurePolicy::Unbounded)
            }
            TokKind::Ident(name) if name == "bounded" => {
                self.bump();
                self.expect(TokKind::LParen, "`(` after `bounded`")?;
                let (capacity, _) =
                    self.expect_positive_int_literal("a positive bounded backpressure size")?;
                self.expect(TokKind::RParen, "`)` after bounded backpressure size")?;
                Ok(BackpressurePolicy::Bounded(capacity))
            }
            TokKind::Ident(name) if name == "pulls_from" => {
                self.bump();
                self.expect(TokKind::LParen, "`(` after `pulls_from`")?;
                let (source, _) = self.expect_ident()?;
                self.expect(TokKind::RParen, "`)` after pulls_from source")?;
                Ok(BackpressurePolicy::PullsFrom(source))
            }
            other => Err(ParseError {
                kind: ParseErrorKind::UnexpectedToken {
                    got: describe_token(&other),
                    expected: "`bounded(N)`, `pulls_from(name)`, or `unbounded`".into(),
                },
                span,
            }),
        }
    }

    /// Parse the pre-agent annotation stream as a mix of compile-
    /// time agent attributes (e.g. `@replayable`) and dimensional
    /// effect constraints (e.g. `@cost(<=$1)`, `@budget($0.10)`).
    ///
    /// Attributes are invariants the type checker enforces but
    /// that do not compose through the call graph; constraints
    /// are dimensional bounds that participate in cost analysis.
    /// Splitting them at parse time keeps the AST honest — the
    /// checker doesn't need to scan `constraints` for pretend-
    /// dimensions named "replayable".
    ///
    /// Attribute names are a fixed catalog (`replayable`,
    /// `deterministic`, `wrapping`, `grounded_pure`). Anything not
    /// in the catalog is treated as an effect constraint.
    pub(super) fn parse_agent_annotations(
        &mut self,
    ) -> Result<(Vec<AgentAttribute>, Vec<EffectConstraint>), ParseError> {
        self.parse_agent_annotations_with_newline(true)
    }

    pub(super) fn parse_inline_agent_annotations(
        &mut self,
    ) -> Result<(Vec<AgentAttribute>, Vec<EffectConstraint>), ParseError> {
        self.parse_agent_annotations_with_newline(false)
    }

    fn parse_agent_annotations_with_newline(
        &mut self,
        expect_newline: bool,
    ) -> Result<(Vec<AgentAttribute>, Vec<EffectConstraint>), ParseError> {
        let mut attributes = Vec::new();
        let mut constraints = Vec::new();
        while matches!(self.peek(), TokKind::At) {
            let start = self.peek_span();
            self.bump(); // @
            // Annotation names may collide with reserved keywords
            // (`@retry` — slice 45q). Accept the keyword token and
            // recover its text instead of demanding a bare ident.
            let (name, name_span) = if matches!(self.peek(), TokKind::KwRetry) {
                let span = self.peek_span();
                self.bump();
                ("retry".to_string(), span)
            } else {
                self.expect_ident()?
            };

            // Named-argument annotations (slice 45q): `@retry(...)`
            // and `@idempotency(...)` take `name: value` pairs.
            if name == "retry" || name == "idempotency" {
                let attribute =
                    self.parse_named_arg_attribute(&name, start, name_span, expect_newline)?;
                attributes.push(attribute);
                continue;
            }

            // Agent attributes: marker-style annotations with no
            // arguments. Optional empty parens are tolerated so
            // `@replayable` and `@replayable()` parse the same.
            if let Some(attribute) =
                self.try_parse_attribute(&name, start, name_span, expect_newline)?
            {
                attributes.push(attribute);
                continue;
            }

            // Dimensional effect constraint — `@cost(<=$1)`,
            // `@budget(...)`, `@trust(autonomous)`, etc.
            if name == "budget" && matches!(self.peek(), TokKind::LParen) {
                constraints.extend(self.parse_budget_constraints(start, name_span)?);
                if expect_newline {
                    self.expect_newline()?;
                }
                continue;
            }
            let value = if matches!(self.peek(), TokKind::LParen) {
                self.bump();
                if matches!(self.peek(), TokKind::RParen) {
                    self.bump();
                    None
                } else {
                    let value = self.parse_dimension_value()?;
                    self.expect(TokKind::RParen, "`)` after constraint value")?;
                    Some(value)
                }
            } else {
                None
            };
            let end = self.prev_span();
            if expect_newline {
                self.expect_newline()?;
            }
            constraints.push(EffectConstraint {
                dimension: Ident::new(name, name_span),
                value,
                span: start.merge(end),
            });
        }
        Ok((attributes, constraints))
    }

    /// `@retry(max_attempts: 3[, backoff: linear|exponential N])`
    /// and `@idempotency(key: param_name)` — named-argument
    /// annotations (slice 45q).
    fn parse_named_arg_attribute(
        &mut self,
        name: &str,
        start: Span,
        _name_span: Span,
        expect_newline: bool,
    ) -> Result<AgentAttribute, ParseError> {
        self.expect(TokKind::LParen, "`(` after the annotation name")?;
        let mut max_attempts: Option<u64> = None;
        let mut backoff: Option<Backoff> = None;
        let mut key: Option<Ident> = None;
        loop {
            // Argument names may also collide with keywords
            // (`backoff` is KwBackoff).
            let (arg, arg_span) = if matches!(self.peek(), TokKind::KwBackoff) {
                let span = self.peek_span();
                self.bump();
                ("backoff".to_string(), span)
            } else {
                self.expect_ident()?
            };
            self.expect(TokKind::Colon, "`:` after the argument name")?;
            match (name, arg.as_str()) {
                ("retry", "max_attempts") => {
                    max_attempts = Some(self.parse_u64_literal("max_attempts")?);
                }
                ("retry", "backoff") => {
                    // Same shape as the `try ... retry` expression:
                    // `linear <ms>` or `exponential <ms>`.
                    backoff = Some(match self.peek() {
                        TokKind::KwLinear => {
                            self.bump();
                            Backoff::Linear(self.parse_u64_literal("linear backoff delay in ms")?)
                        }
                        TokKind::KwExponential => {
                            self.bump();
                            Backoff::Exponential(
                                self.parse_u64_literal("exponential backoff base delay in ms")?,
                            )
                        }
                        other => {
                            return Err(ParseError {
                                kind: ParseErrorKind::UnexpectedToken {
                                    got: describe_token(other),
                                    expected: "`linear <ms>` or `exponential <ms>`".into(),
                                },
                                span: self.peek_span(),
                            });
                        }
                    });
                }
                ("idempotency", "key") => {
                    let (k, k_span) = self.expect_ident()?;
                    key = Some(Ident::new(k, k_span));
                }
                _ => {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: format!("annotation argument `{arg}`"),
                            expected: match name {
                                "retry" => "`max_attempts:` or `backoff:` in `@retry(...)`",
                                _ => "`key:` in `@idempotency(...)`",
                            }
                            .into(),
                        },
                        span: arg_span,
                    });
                }
            }
            if matches!(self.peek(), TokKind::Comma) {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(TokKind::RParen, "`)` closing the annotation arguments")?;
        let end = self.prev_span();
        if expect_newline {
            self.expect_newline()?;
        }
        let span = start.merge(end);
        match name {
            "retry" => {
                let Some(max_attempts) = max_attempts else {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "`@retry` without `max_attempts`".into(),
                            expected: "`@retry(max_attempts: N)` — the attempt count is required"
                                .into(),
                        },
                        span,
                    });
                };
                Ok(AgentAttribute::Retry {
                    max_attempts,
                    backoff,
                    span,
                })
            }
            _ => {
                let Some(key) = key else {
                    return Err(ParseError {
                        kind: ParseErrorKind::UnexpectedToken {
                            got: "`@idempotency` without `key`".into(),
                            expected: "`@idempotency(key: param_name)` — the key is required"
                                .into(),
                        },
                        span,
                    });
                };
                Ok(AgentAttribute::Idempotency { key, span })
            }
        }
    }

    /// Try to parse the current `@name` as a known agent
    /// attribute. Returns `Some(attribute)` on match; `None`
    /// leaves the parser positioned after the name so the
    /// caller can fall through to effect-constraint parsing.
    fn try_parse_attribute(
        &mut self,
        name: &str,
        start: Span,
        _name_span: Span,
        expect_newline: bool,
    ) -> Result<Option<AgentAttribute>, ParseError> {
        let attribute_kind: fn(Span) -> AgentAttribute = match name {
            "replayable" => |span| AgentAttribute::Replayable { span },
            "deterministic" => |span| AgentAttribute::Deterministic { span },
            "wrapping" => |span| AgentAttribute::Wrapping { span },
            "grounded_pure" => |span| AgentAttribute::GroundedPure { span },
            _ => return Ok(None),
        };

        // Tolerate `@name` and `@name()` as synonyms for marker
        // attributes;
        // anything else after the name at the statement level
        // belongs to a following constraint or keyword.
        if matches!(self.peek(), TokKind::LParen) {
            self.bump();
            self.expect(TokKind::RParen, "`)` after attribute name")?;
        }
        let end = self.prev_span();
        if expect_newline {
            self.expect_newline()?;
        }
        Ok(Some(attribute_kind(start.merge(end))))
    }

    fn parse_budget_constraints(
        &mut self,
        start: Span,
        name_span: Span,
    ) -> Result<Vec<EffectConstraint>, ParseError> {
        self.expect(TokKind::LParen, "`(` after `@budget`")?;
        let mut constraints = Vec::new();

        if !matches!(self.peek(), TokKind::RParen) {
            loop {
                if matches!(self.peek(), TokKind::Dollar) {
                    let value = self.parse_dimension_value()?;
                    constraints.push(EffectConstraint {
                        dimension: Ident::new("cost", name_span),
                        value: Some(value),
                        span: start.merge(self.prev_span()),
                    });
                } else {
                    let (dim_name, dim_span) = self.expect_ident()?;
                    self.expect(TokKind::Colon, "`:` after budget dimension name")?;
                    let value = self.parse_dimension_value()?;
                    let canonical_name = match dim_name.as_str() {
                        "latency" => "latency_ms".to_string(),
                        other => other.to_string(),
                    };
                    constraints.push(EffectConstraint {
                        dimension: Ident::new(canonical_name, dim_span),
                        value: Some(value),
                        span: start.merge(self.prev_span()),
                    });
                }

                if !matches!(self.peek(), TokKind::Comma) {
                    break;
                }
                self.bump();
            }
        }

        self.expect(TokKind::RParen, "`)` after budget constraints")?;
        Ok(constraints)
    }

    pub(super) fn consume_optional_duration_suffix(&mut self, value: f64) -> f64 {
        match self.peek() {
            TokKind::Ident(unit) if unit == "ms" => {
                self.bump();
                value
            }
            TokKind::Ident(unit) if unit == "s" => {
                self.bump();
                value * 1000.0
            }
            _ => value,
        }
    }
}
