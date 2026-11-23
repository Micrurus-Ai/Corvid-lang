use super::*;

impl Parser<'_> {
    pub(super) fn parse_drag_component(
        &mut self,
        name: &str,
        state: &str,
        initial: u64,
        ui: &str,
    ) -> Result<(String, String, Option<corvid_ast::Expr>), ParseError> {
        let canvas = self.peek_ident_is("canvas");
        self.bump();
        self.expect(TokKind::LParen, "(")?;
        if !canvas {
            self.expect_contextual_ident(state)?;
            self.expect(TokKind::Comma, ",")?;
        }
        let first = self.parse_u64_literal("a positive pixel size")?;
        self.expect(TokKind::Comma, ",")?;
        let second = self.parse_u64_literal("a positive pixel size")?;
        if !(1..=8192).contains(&first) || !(1..=8192).contains(&second) {
            return Err(self.expected("dimensions between 1 and 8192 pixels"));
        }
        if !canvas && !(first..=second).contains(&initial) {
            return Err(self.expected("panel bounds containing the initial width"));
        }
        let content = if canvas {
            None
        } else {
            self.expect(TokKind::Comma, "a list of panel children")?;
            Some(self.parse_expr()?)
        };
        self.expect(TokKind::RParen, ")")?;
        self.expect_newline()?;
        self.expect(TokKind::KwOn, "on drag")?;
        self.expect_contextual_ident("drag")?;
        self.expect(TokKind::LParen, "(")?;
        let (event, _) = self.expect_ident()?;
        if event == state {
            return Err(self.expected("a stroke parameter distinct from the state name"));
        }
        self.expect(TokKind::Colon, ":")?;
        self.expect_contextual_ident("Stroke")?;
        self.expect(TokKind::RParen, ")")?;
        self.open_block()?;
        let draw = canvas && self.peek_ident_is("line");
        if draw {
            self.expect_contextual_ident("line")?;
            self.expect(TokKind::LParen, "(")?;
            for (field, sep) in [("start", TokKind::Comma), ("end", TokKind::RParen)] {
                self.expect_contextual_ident(&event)?;
                self.expect(TokKind::Dot, ".")?;
                self.expect_contextual_ident(field)?;
                self.expect(sep, if field == "start" { "," } else { ")" })?;
            }
            self.expect_newline()?;
        }
        self.expect_contextual_ident(state)?;
        let increment = matches!(self.peek(), TokKind::PlusEq);
        if increment {
            self.bump();
        } else {
            self.expect(TokKind::Assign, "= or +=")?;
        }
        let expr = drag_integer(&self.parse_expr()?, state, &event)?;
        let update = if increment {
            format!("value + ({expr})")
        } else {
            expr
        };
        self.expect_newline()?;
        self.expect(TokKind::Dedent, "end of drag handler")?;
        let handler = format!("__drag_{name}");
        let events = format!("agent {handler}(value: Int, start_x: Int, start_y: Int, end_x: Int, end_y: Int) -> Int:\n    return {update}\n");
        let surface = if canvas { "drag_canvas" } else { "drag_panel" };
        let extra = if canvas {
            draw.to_string()
        } else {
            "[]".into()
        };
        let view = format!(
            "{ui}.{surface}(\"CAPTION0\", {initial}, {first}, {second}, \"{handler}\", {extra})"
        );
        Ok((view, events, content))
    }
}

// Project the typed stroke onto the browser backend's scalar ABI.
fn drag_integer(expr: &corvid_ast::Expr, state: &str, event: &str) -> Result<String, ParseError> {
    use corvid_ast::{BinaryOp, Expr, Literal};
    match expr {
        Expr::Literal {
            value: Literal::Int(n),
            ..
        } => return Ok(n.to_string()),
        Expr::Ident { name, .. } if name.name == state => return Ok("value".into()),
        Expr::FieldAccess {
            target,
            field: axis,
            ..
        } if matches!(axis.name.as_str(), "x" | "y") => {
            if let Expr::FieldAccess { target, field, .. } = target.as_ref() {
                if matches!(field.name.as_str(), "start" | "end")
                    && matches!(target.as_ref(), Expr::Ident {name, ..} if name.name == event)
                {
                    return Ok(format!("{}_{}", field.name, axis.name));
                }
            }
        }
        Expr::BinOp {
            op, left, right, ..
        } => {
            let symbol = match op {
                BinaryOp::Add => Some("+"),
                BinaryOp::Sub => Some("-"),
                BinaryOp::Mul => Some("*"),
                _ => None,
            };
            if let Some(symbol) = symbol {
                return Ok(format!(
                    "({} {symbol} {})",
                    drag_integer(left, state, event)?,
                    drag_integer(right, state, event)?
                ));
            }
        }
        _ => {}
    }
    Err(ParseError {
        span: expr.span(),
        kind: ParseErrorKind::UnexpectedToken {
            got: "unsupported drag expression".into(),
            expected: format!("integer arithmetic using {state} or {event}.start/end.x/y"),
        },
    })
}
