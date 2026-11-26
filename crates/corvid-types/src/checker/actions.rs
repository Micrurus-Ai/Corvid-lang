//! `action` declarations (B10): who may call an action from outside the
//! program is checked against the identity block the same way a route's
//! `requires` is, through one rule.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::action_input::ACTIONS_PATH;
use corvid_ast::{ActionSpec, AgentDecl, Decl, File, RoutePolicy};

impl<'a> Checker<'a> {
    /// An action's caller policy names authorities the identity block
    /// declares. `requires anyone` names none, and needs no identity.
    pub(super) fn check_action_policy(&mut self, agent: &AgentDecl, spec: &ActionSpec) {
        let policy = &spec.policy;
        if !policy.requires_auth() {
            return;
        }
        if !self.has_identity {
            self.errors.push(TypeError::new(
                TypeErrorKind::ActionPolicyWithoutIdentity { action: agent.name.name.clone() },
                policy.span,
            ));
            return;
        }
        let action = agent.name.name.clone();
        self.check_policy_authorities(policy, &|kind, name| TypeErrorKind::ActionPolicyUndeclaredAuthority {
            kind,
            name: name.to_string(),
            action: action.clone(),
        });
    }

    /// No route is declared where an action is served, `/actions/<name>`:
    /// the two would answer one URL.
    pub(super) fn check_action_paths(&mut self, file: &File) {
        let actions: Vec<&str> = file
            .decls
            .iter()
            .filter_map(|decl| match decl {
                Decl::Agent(agent) if agent.action.is_some() => Some(agent.name.name.as_str()),
                _ => None,
            })
            .collect();
        for decl in &file.decls {
            let Decl::Server(server) = decl else { continue };
            for route in &server.routes {
                let taken = route.path.strip_prefix(ACTIONS_PATH).filter(|name| actions.contains(name));
                if let Some(action) = taken {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ActionPathTaken { action: action.to_string(), path: route.path.clone() },
                        route.span,
                    ));
                }
            }
        }
    }

    /// Every role and permission a `requires` clause names is declared in
    /// the identity block's `roles:`; one that is not could never be
    /// satisfied and is almost certainly a typo (slice 52f). Routes and
    /// actions both use this, each naming itself in the error. Only
    /// checked when an identity block exists.
    pub(super) fn check_policy_authorities(
        &mut self,
        policy: &RoutePolicy,
        undeclared: &dyn Fn(&'static str, &str) -> TypeErrorKind,
    ) {
        if !self.has_identity {
            return;
        }
        for role in &policy.roles {
            if !self.identity_roles.contains(role) {
                self.errors.push(TypeError::new(undeclared("role", role), policy.span));
            }
        }
        for permission in &policy.permissions {
            if !self.identity_permissions.contains(permission) {
                self.errors.push(TypeError::new(undeclared("permission", permission), policy.span));
            }
        }
    }
}
