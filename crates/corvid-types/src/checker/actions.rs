//! `action` declarations (B10): who may call an action from outside the
//! program is checked against the identity block the same way a route's
//! `requires` is, through one rule.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{ActionSpec, AgentDecl, RoutePolicy};

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
