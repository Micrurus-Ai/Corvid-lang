mod drag;

use super::{describe_token, Parser};
use crate::{lex, ParseError, ParseErrorKind, TokKind};
use corvid_ast::{Decl, Span};

/// Browser handlers collected while parsing a component declaration.
pub struct Component {
    pub name: String,
    pub events: String,
    /// Original source span for each line added to the browser event module.
    pub event_line_spans: Vec<Span>,
}

pub fn browser_declarations(
    tokens: &[crate::Token],
) -> Result<(Vec<Component>, Vec<Span>), Vec<ParseError>> {
    let mut parser = Parser::new(tokens);
    parser.parse_file_inner();
    if parser.errors.is_empty() {
        Ok((parser.components, parser.browser_callbacks))
    } else {
        Err(parser.errors)
    }
}

pub fn components(tokens: &[crate::Token]) -> Result<Vec<Component>, Vec<ParseError>> {
    browser_declarations(tokens).map(|(components, _)| components)
}

impl Parser<'_> {
    pub(super) fn parse_browser_callback(&mut self) -> Result<Decl, ParseError> {
        use corvid_ast::{TypeRef, Visibility};
        let start = self.bump().span;
        let named = |ty: &TypeRef, expected: &str| {
            matches!(ty, TypeRef::Named { name, .. } if name.name == expected)
        };
        let visibility = if self.emit_browser_exports {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let (mut implementation, binding_type) = if self.peek_ident_is("lifecycle") {
            let mut hook = self.parse_agent_decl(visibility)?;
            if !hook.params.is_empty() || !named(&hook.return_ty, "Nothing") {
                return Err(ParseError {
                    span: hook.name.span,
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "browser lifecycle signature".into(),
                        expected: "no parameters and a Nothing return type".into(),
                    },
                });
            }
            // Falling through a lifecycle body completes the synchronous hook.
            if !matches!(hook.body.stmts.last(), Some(corvid_ast::Stmt::Return { .. })) {
                hook.body.stmts.push(corvid_ast::Stmt::Return {
                    value: None,
                    span: hook.body.span,
                });
            }
            (Decl::Agent(hook), "ElementCallback")
        } else {
            let update = self.parse_fn_decl(visibility)?;
            let signature = |ty| update.params.len() == 1
                && named(&update.params[0].ty, ty) && named(&update.return_ty, ty);
            let binding_type = if signature("Int") {
                "IntCallback"
            } else if signature("String") {
                "TextCallback"
            } else {
                return Err(ParseError {
                    span: update.name.span,
                    kind: ParseErrorKind::UnexpectedToken {
                        got: "browser callback signature".into(),
                        expected: "an (Int) -> Int or (String) -> String callback".into(),
                    },
                });
            };
            (Decl::Fn(update), binding_type)
        };
        if self.emit_browser_exports {
            return Ok(implementation);
        }
        let ident = match &mut implementation {
            Decl::Agent(hook) => &mut hook.name,
            Decl::Fn(update) => &mut update.name,
            _ => unreachable!(),
        };
        let name = ident.name.clone();
        self.browser_callbacks.push(start);
        ident.name = format!("__browser_callback_{name}");
        self.generated.push(implementation);
        let ui = format!("__callback_ui_{name}");
        let factory = format!("import \"std:ui\" as {ui}\npublic fn {name}() -> {ui}.{binding_type}:\n    return {ui}.{binding_type}(\"{name}\")\n");
        let mut tokens = lex(&factory).expect("valid callback factory");
        for (index, token) in tokens.iter_mut().enumerate() {
            token.span = start.expanded(index);
        }
        let (file, errors) = super::parse_file(&tokens);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        let mut declarations = file.decls.into_iter();
        let import = declarations.next().expect("callback UI import");
        self.generated.extend(declarations);
        Ok(import)
    }

    fn expected(&self, expected: &str) -> ParseError {
        ParseError {
            span: self.peek_span(),
            kind: ParseErrorKind::UnexpectedToken {
                got: describe_token(self.peek()),
                expected: expected.to_owned(),
            },
        }
    }

    fn ui_string(&mut self) -> Result<String, ParseError> {
        self.expect(TokKind::LParen, "(")?;
        let TokKind::StringLit(value) = self.peek().clone() else {
            return Err(self.expected("a quoted label"));
        };
        self.bump();
        self.expect(TokKind::RParen, ")")?;
        Ok(value)
    }

    fn open_block(&mut self) -> Result<(), ParseError> {
        self.expect(TokKind::Colon, ":")?;
        self.expect_newline()?;
        self.expect(TokKind::Indent, "an indented block")?;
        Ok(())
    }

    pub(super) fn parse_component_decl(&mut self) -> Result<Decl, ParseError> {
        let start = self.bump().span;
        let (name, _) = self.expect_ident()?;
        self.open_block()?;
        self.expect_contextual_ident("state")?;
        let (state, _) = self.expect_ident()?;
        if matches!(self.peek(), TokKind::Colon) {
            self.bump();
            self.expect_contextual_ident("Int")?;
        }
        self.expect(TokKind::Assign, "=")?;
        let initial = self.parse_u64_literal("a nonnegative integer initializer")?;
        self.expect_newline()?;
        self.expect_contextual_ident("text")?;
        let display_span = self.peek_span();
        let display = self.ui_string()?;
        let suffix = format!("{{{state}}}");
        let label = display
            .strip_suffix(&suffix)
            .filter(|prefix| !prefix.contains(['{', '}']))
            .ok_or_else(|| ParseError {
                span: display_span,
                kind: ParseErrorKind::UnexpectedToken {
                    got: display.clone(),
                    expected: format!("a label ending in {suffix}"),
                },
            })?;
        self.expect_newline()?;
        let mut captions = vec![label.trim_end().to_owned()];
        let mut events = String::new();
        let mut event_line_spans = Vec::new();
        let mut buttons = Vec::new();
        let ui = format!("__ui_{name}");
        let mut panel_content = None;
        let surface = if self.peek_ident_is("canvas") || self.peek_ident_is("panel") {
            let (view, handler, children) =
                self.parse_drag_component(&name, &state, initial, &ui, &mut event_line_spans)?;
            panel_content = children;
            events = handler;
            Some(view)
        } else {
            None
        };
        while surface.is_none() && self.peek_ident_is("button") {
            let button_span = self.peek_span();
            self.bump();
            captions.push(self.ui_string()?);
            self.expect(TokKind::KwOn, "on click")?;
            self.expect_contextual_ident("click")?;
            self.open_block()?;
            event_line_spans.extend([button_span, self.peek_span()]);
            self.expect_contextual_ident(&state)?;
            self.expect(TokKind::PlusEq, "+=")?;
            let increment = self.parse_u64_literal("a nonnegative increment")?;
            self.expect_newline()?;
            self.expect(TokKind::Dedent, "end of click handler")?;
            let index = buttons.len() + 1;
            let handler = format!("__click_{name}_{index}");
            buttons.push(format!("{ui}.action(\"CAPTION{index}\", \"{handler}\")"));
            events.push_str(&format!(
                "agent {handler}(value: Int) -> Int:\n    return value + {increment}\n"
            ));
        }
        if surface.is_none() && buttons.is_empty() {
            return Err(self.expected("button"));
        }
        self.expect(TokKind::Dedent, "end of component")?;
        let view = surface.unwrap_or_else(|| {
            format!(
                "{ui}.int_state(\"CAPTION0\", {initial}, [{}])",
                buttons.join(", ")
            )
        });
        let source = format!(
            "import \"std:ui\" as {ui}\npublic fn {name}() -> {ui}.View:\n    return {view}\n"
        );
        let mut tokens = lex(&source).expect("valid component lowering");
        for (index, token) in tokens.iter_mut().enumerate() {
            token.span = start.expanded(index);
            if let TokKind::StringLit(id) = &token.kind {
                if let Some(caption) = id
                    .strip_prefix("CAPTION")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| captions.get(n))
                {
                    token.kind = TokKind::StringLit(caption.clone());
                }
            }
        }
        let (file, errors) = super::parse_file(&tokens);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        let mut generated = file.decls.into_iter();
        let import = generated.next().expect("component UI import");
        let mut view_fn = generated.next().expect("component view function");
        if let Some(children) = panel_content {
            let Decl::Fn(view) = &mut view_fn else {
                unreachable!()
            };
            let [corvid_ast::Stmt::Return {
                value: Some(corvid_ast::Expr::Call { args, .. }),
                ..
            }] = view.body.stmts.as_mut_slice()
            else {
                unreachable!()
            };
            *args.last_mut().expect("panel children argument") = children;
        }
        self.generated.push(view_fn);
        self.components.push(Component { name, events, event_line_spans });
        Ok(import)
    }
}

pub(super) fn call(name: &str, args: Vec<corvid_ast::Expr>, span: Span) -> corvid_ast::Expr {
    use corvid_ast::{Expr, Ident};
    Expr::Call {
        callee: Box::new(Expr::Ident {
            name: Ident::new(name, span),
            span,
        }),
        args,
        span,
    }
}

impl Parser<'_> {
    pub(super) fn parse_bounded_ask(&mut self) -> Result<corvid_ast::Expr, ParseError> {
        use corvid_ast::{Backoff, Expr};
        let start = self.bump().span;
        let (prompt, prompt_span) = self.expect_ident()?;
        self.expect(TokKind::LParen, "(")?;
        let input = self.parse_expr()?;
        self.expect(TokKind::RParen, ")")?;
        self.expect_newline()?;
        self.expect(TokKind::Indent, "an indented allow clause")?;
        self.expect_contextual_ident("allow")?;
        let (lookup, lookup_span) = self.expect_ident()?;
        self.expect_contextual_ident("once")?;
        self.expect_newline()?;
        self.expect_contextual_ident("within")?;
        let bound = self.peek_span();
        let seconds = self.parse_u64_literal("a positive deadline in seconds")?;
        let millis = seconds
            .checked_mul(1000)
            .filter(|ms| *ms > 0)
            .ok_or_else(|| ParseError {
                span: bound,
                kind: ParseErrorKind::UnexpectedToken {
                    got: seconds.to_string(),
                    expected: "a positive deadline in millisecond range".to_owned(),
                },
            })?;
        self.expect_contextual_ident("seconds")?;
        self.expect_newline()?;
        self.expect(TokKind::Dedent, "end of ask clauses")?;

        // Both prompt calls and the sole lookup execute inside one timed call.
        let helper = format!("__bounded_ask_{}", start.start);
        let source = format!(
            "agent {helper}(input: String) -> Result<String, String>:
    first = PROMPT(input, \"No lookup yet\")
    return match first.action:
        Suggest(answer) -> Ok(answer)
        Lookup(request) -> {helper}_after(input, request)

agent {helper}_after(input: String, request: Int) -> Result<String, String>:
    context = LOOKUP(request)?
    last = PROMPT(input, context)
    return match last.action:
        Suggest(answer) -> Ok(answer)
        Lookup(_) -> Err(\"The model reached its lookup limit.\")
"
        );
        let mut tokens = lex(&source).expect("valid bounded ask lowering");
        for (index, token) in tokens.iter_mut().enumerate() {
            token.span = prompt_span.expanded(index);
            if let TokKind::Ident(id) = &mut token.kind {
                if id == "PROMPT" {
                    *id = prompt.clone();
                } else if id == "LOOKUP" {
                    *id = lookup.clone();
                    token.span = lookup_span.expanded(index);
                }
            }
        }
        let (file, errors) = super::parse_file(&tokens);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        self.generated.extend(file.decls);
        self.block_expr_terminated = true;
        let span = start.merge(bound);
        Ok(Expr::TryPropagate {
            inner: Box::new(Expr::TryRetry {
                body: Box::new(call(&helper, vec![input], start)),
                attempts: 0,
                backoff: Backoff::Linear(0),
                timeout_ms: Some(millis),
                span,
            }),
            span,
        })
    }
}

// The concise signature names the success type. Explicit Result signatures
// keep ordinary Corvid return rules and can supply their own failure branches.
pub(super) fn ask_return_type(
    ty: corvid_ast::TypeRef,
    body: &mut corvid_ast::Block,
) -> corvid_ast::TypeRef {
    use corvid_ast::{Block, Ident, Stmt, TypeRef};
    if matches!(&ty, TypeRef::Generic { name, .. } if name.name == "Result") {
        return ty;
    }
    fn wrap(block: &mut Block) {
        for stmt in &mut block.stmts {
            match stmt {
                Stmt::Return { value, span } => {
                    if let Some(result) = value.take() {
                        *value = Some(call("Ok", vec![result], *span));
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    wrap(then_block);
                    if let Some(block) = else_block {
                        wrap(block);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => wrap(body),
                _ => {}
            }
        }
    }
    wrap(body);
    let span = ty.span();
    TypeRef::Generic {
        name: Ident::new("Result", span),
        args: vec![
            ty,
            TypeRef::Named {
                name: Ident::new("String", span),
                span,
            },
        ],
        span,
    }
}
