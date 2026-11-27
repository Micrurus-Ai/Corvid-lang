//! `action` declarations (B10): who may call an action from outside the
//! program is checked against the identity block the same way a route's
//! `requires` is, through one rule.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use crate::action_input::{is_caller, ACTIONS_PATH};
use corvid_ast::{ActionSpec, AgentDecl, Decl, File, RoutePolicy, TypeRef};

impl<'a> Checker<'a> {
    /// An action's caller policy names authorities the identity block
    /// declares. `requires anyone` names none, and needs no identity.
    pub(super) fn check_action_policy(&mut self, agent: &AgentDecl, spec: &ActionSpec) {
        // An assistant acts for a signed-in person, who confirms its calls,
        // so offering one needs a way to sign in.
        if let Some(line) = &spec.assistants {
            if !self.has_identity {
                self.errors.push(TypeError::new(
                    TypeErrorKind::AssistantsWithoutIdentity { action: agent.name.name.clone() },
                    line.span,
                ));
            }
        }
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

    /// A `Caller` comes only from a verified session. An action's
    /// parameter of type `Caller` is its caller, which needs a signed-in
    /// policy; a `Caller` anywhere a request, a client or a model supplies
    /// the value would let the sender name any caller, so it is refused:
    /// inside an action's input, in a route's path, query or body, and in
    /// a prompt's result.
    pub(super) fn check_actor_sources(&mut self, file: &File) {
        for decl in &file.decls {
            match decl {
                Decl::Agent(agent) => {
                    let Some(spec) = &agent.action else { continue };
                    for param in &agent.params {
                        let Some(ty) = self.type_refs.get(&param.ty.span()).cloned() else { continue };
                        if is_caller(&ty) {
                            if !spec.policy.requires_auth() {
                                self.errors.push(TypeError::new(
                                    TypeErrorKind::ActionCallerWithoutSignIn {
                                        action: agent.name.name.clone(),
                                        param: param.name.name.clone(),
                                    },
                                    param.span,
                                ));
                            }
                        } else {
                            self.refuse_actor_at(&param.ty, "an action's input");
                        }
                    }
                }
                Decl::Server(server) => {
                    for route in &server.routes {
                        let inputs: Vec<&TypeRef> = route
                            .path_params
                            .iter()
                            .map(|param| &param.ty)
                            .chain(route.query_ty.iter())
                            .chain(route.body_ty.iter())
                            .collect();
                        for ty in inputs {
                            self.refuse_actor_at(ty, "a route's path, query or body");
                        }
                    }
                }
                Decl::Prompt(prompt) => self.refuse_actor_at(&prompt.return_ty, "a prompt's result"),
                _ => {}
            }
        }
    }

    fn refuse_actor_at(&mut self, ty: &TypeRef, place: &str) {
        let Some(resolved) = self.type_refs.get(&ty.span()).cloned() else { return };
        if let Some(at) = self.type_reached(&resolved, is_caller) {
            self.errors.push(TypeError::new(
                TypeErrorKind::ActorFromInput { place: place.to_string(), at },
                ty.span(),
            ));
        }
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
