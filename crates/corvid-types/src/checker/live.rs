//! Resolve browser live bindings against the same typed route as the backend.
use super::Checker;
use crate::{
    errors::{TypeError, TypeErrorKind},
    Type,
};
use corvid_ast::{Decl, Expr, HttpRouteDecl, Ident, Literal, Span};

impl<'a> Checker<'a> {
    fn live_error(&mut self, span: Span, message: impl Into<String>) {
        self.errors.push(TypeError::new(
            TypeErrorKind::InvalidLiveBinding {
                message: message.into(),
            },
            span,
        ));
    }

    fn live_route(&mut self, name: &Ident, args: &[Expr]) -> Option<&'a HttpRouteDecl> {
        let [Expr::Literal {
            value: Literal::String(server),
            ..
        }, Expr::Literal {
            value: Literal::String(live),
            ..
        }, ..] = args
        else {
            self.live_error(name.span, "expected a declared server.subscription");
            return None;
        };
        let route = self.file.decls.iter().find_map(|decl| match decl {
            Decl::Server(s) if s.name.name == *server => s
                .routes
                .iter()
                .find(|r| r.live_name() == Some(live.as_str())),
            _ => None,
        });
        if route.is_none() {
            self.live_error(
                args[1].span(),
                format!("no live declaration named `{server}.{live}`"),
            );
        }
        route
    }

    pub(super) fn check_live_arguments(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if let Some(route) = self.live_route(name, args) {
            let values = &args[2..];
            if values.len() != route.path_params.len() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::ArityMismatch {
                        callee: format!("ui.live path {}", route.path),
                        expected: route.path_params.len(),
                        got: values.len(),
                    },
                    name.span,
                ));
            }
            for (param, value) in route.path_params.iter().zip(values) {
                let expected = self.type_ref_to_type(&param.ty);
                let got = self.check_expr_as(value, Some(&expected));
                if expected != got {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: expected.display_name(),
                            got: got.display_name(),
                            context: format!("live path parameter `{}`", param.name.name),
                        },
                        value.span(),
                    ));
                }
            }
        }
        Type::List(Box::new(Type::String))
    }

    pub(super) fn check_live_field(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if let Some(route) = self.live_route(name, args) {
            if let [_, _, Expr::Literal {
                value: Literal::String(key),
                span,
            }] = args
            {
                let row = self.type_ref_to_type(route.response.live_row().expect("live route"));
                return self.check_field_path(name, row, key, *span, &[]);
            }
            self.live_error(name.span, "expected a field on the live row");
        }
        Type::String
    }
}
