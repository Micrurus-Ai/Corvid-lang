//! Resolve page subscription paths against the compiled live declarations.
use crate::serve_cmd::{bind_path, corvid_route_to_axum_path};
use anyhow::Context;
use corvid_ast::RouteResponseKind;
use corvid_driver::Value;
use corvid_ir::{IrFile, IrRoute};
use percent_encoding::percent_decode_str;

pub(in crate::serve_cmd) struct Routes(Option<matchit::Router<(usize, usize)>>);

impl Routes {
    pub(in crate::serve_cmd) fn new(ir: &IrFile) -> anyhow::Result<Self> {
        let mut routes = None;
        for (server_index, server) in ir.servers.iter().enumerate() {
            for (route_index, route) in server.routes.iter().enumerate() {
                if route.response_kind == RouteResponseKind::Live {
                    routes
                        .get_or_insert_with(matchit::Router::new)
                        .insert(
                            corvid_route_to_axum_path(&route.path),
                            (server_index, route_index),
                        )
                        .with_context(|| format!("register live path {}", route.path))?;
                }
            }
        }
        Ok(Self(routes))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    pub(super) fn resolve<'a>(&self, ir: &'a IrFile, path: &str) -> Option<(&'a IrRoute, Value)> {
        let matched = self.0.as_ref()?.at(path).ok()?;
        let (server, index) = *matched.value;
        let route = &ir.servers[server].routes[index];
        let types_by_id = ir.types.iter().map(|ty| (ty.id, ty)).collect();
        let argument = bind_path(&route.path_params, &types_by_id, |name| {
            percent_decode_str(matched.params.get(name)?)
                .decode_utf8()
                .ok()
        })
        .ok()?;
        Some((route, argument))
    }
}
