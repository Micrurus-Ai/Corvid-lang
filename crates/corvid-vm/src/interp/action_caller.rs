//! Code calling an action with a caller (B10): the action's `requires`
//! holds for the `Actor` passed in, as it holds for a request.
//!
//! A request reaches an action only after `corvid serve` checks its
//! session. A Corvid caller (a route, a worker, another agent) passes an
//! `Actor` it got from a verified session, and this check stops a route
//! that requires less from lending its caller to an action that requires
//! more: `requires authenticated` cannot call through to `requires
//! permission("board.edit")` for a caller without that permission.

use super::Interpreter;
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::Value;
use corvid_ast::Span;
use corvid_ir::IrAgent;
use corvid_types::action_input::is_caller;

impl<'ir> Interpreter<'ir> {
    pub(super) fn check_action_caller(&self, agent: &IrAgent, args: &[Value], span: Span) -> Result<(), InterpError> {
        let Some(action) = self.ir.actions.iter().find(|action| action.agent == agent.id) else {
            return Ok(());
        };
        for (param, arg) in agent.params.iter().zip(args) {
            if !is_caller(&param.ty) {
                continue;
            }
            let Value::Struct(caller) = arg else { continue };
            let held = |field: &str| -> Vec<String> {
                match caller.get_field(field) {
                    Some(Value::List(items)) => items
                        .iter_cloned()
                        .into_iter()
                        .filter_map(|item| match item {
                            Value::String(text) => Some(text.to_string()),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                }
            };
            let roles = held("roles");
            let permissions = held("permissions");
            let missing = action
                .policy
                .roles
                .iter()
                .find(|role| !roles.contains(role))
                .map(|role| format!("role `{role}`"))
                .or_else(|| {
                    action
                        .policy
                        .permissions
                        .iter()
                        .find(|permission| !permissions.contains(permission))
                        .map(|permission| format!("permission `{permission}`"))
                });
            if let Some(missing) = missing {
                let caller = match caller.get_field("id") {
                    Some(Value::String(id)) => id.to_string(),
                    _ => String::new(),
                };
                return Err(InterpError::new(
                    InterpErrorKind::CallerLacksAuthority { action: action.name.clone(), caller, missing },
                    span,
                ));
            }
        }
        Ok(())
    }
}
