//! A child loop over a declared subscription derives its key from the table.
use super::{authoring::call, Parser};
use crate::{ParseError, ParseErrorKind};
use corvid_ast::{Expr, Ident, Literal, Span};

impl Parser<'_> {
    pub(super) fn parse_live_children(
        &mut self,
        row: String,
        source: Expr,
        start: Span,
    ) -> Result<Expr, ParseError> {
        let invalid = |span| ParseError {
            span,
            kind: ParseErrorKind::UnexpectedToken {
                got: "child list source".into(),
                expected: "`by row.field` or ui.live(server.subscription, path arguments)".into(),
            },
        };
        let Expr::Call { callee, args, span } = source else {
            return Err(invalid(source.span()));
        };
        let Expr::FieldAccess {
            target: ui, field, ..
        } = *callee
        else {
            return Err(invalid(span));
        };
        if field.name != "live" {
            return Err(invalid(span));
        }
        let mut args = args.into_iter();
        let Some(Expr::FieldAccess { target, field, .. }) = args.next() else {
            return Err(invalid(span));
        };
        let Expr::Ident { name: server, .. } = *target else {
            return Err(invalid(span));
        };
        let literal = |id: Ident| Expr::Literal {
            value: Literal::String(id.name),
            span: id.span,
        };
        let server = literal(server);
        let live = literal(field);
        let mut params = vec![server.clone(), live.clone()];
        params.extend(args);
        let params = call("__live_arguments", params, span);
        self.row_bindings.push(super::RowBinding {
            name: row,
            live: Some((server.clone(), live.clone())),
        });
        let children = self.parse_child_list();
        self.row_bindings.pop();
        let children = children?;
        let end = children.span();
        Ok(Expr::Call {
            callee: Box::new(Expr::FieldAccess {
                target: ui,
                field: Ident::new("live_rows", start.expanded(1)),
                span: start.expanded(2),
            }),
            args: vec![server, live, params, children],
            span: start.merge(end),
        })
    }
}
