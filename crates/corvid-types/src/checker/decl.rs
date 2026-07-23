//! Agent + eval declaration checking.
//!
//! `check_agent` validates an `agent name(params) -> T: body` Ã¢â‚¬â€
//! parameter binding, return-type matching, yield/stream legality.
//! `check_eval` validates an `eval name: body` Ã¢â‚¬â€ including
//! trace-assert (`assert called X before Y`) and statistical
//! confidence modifiers.
//!
//! Extracted from `checker.rs` as part of Phase 20i responsibility
//! decomposition.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind, TypeWarning, TypeWarningKind};
use crate::types::Type;
use corvid_ast::{
    AgentAttribute, AgentDecl, Block, ConnectorAuth, ConnectorDecl, ConnectorMode, EmailMatchPolicy,
    Expr,
    FirstLoginPolicy, HttpMethod, HttpRouteDecl, IdentityDecl, ProviderKind, ProvisioningPolicy,
    SameSite, ServerDecl, Span, Stmt, TenantAssignment,
};
use corvid_resolve::Binding;
use std::collections::HashSet;

impl<'a> Checker<'a> {
    pub(super) fn check_agent(&mut self, a: &AgentDecl) {
        // Bind parameter types.
        self.bind_params(&a.params);

        if a.extern_abi.is_some() {
            self.check_extern_c_signature(a);
        }

        let declared_ret = self.type_ref_to_type(&a.return_ty);
        let prev_ret = std::mem::replace(&mut self.current_return, Some(declared_ret.clone()));
        let prev_in_agent = std::mem::replace(&mut self.in_agent_body, true);
        let prev_saw_yield = std::mem::replace(&mut self.saw_yield, false);
        // Reset the body-wide approve audit log; restore on exit.
        // This is what lets the dangerous-call diagnostic
        // discriminate `approval.token_lexical_only` (an approve
        // exists in this agent's body but is out of lexical scope
        // at the call site) from
        // `approval.dangerous_call_requires_token` (no approve
        // anywhere in the agent).
        let prev_approvals_seen = std::mem::take(&mut self.approvals_seen_in_agent);

        self.check_block(&a.body);

        if matches!(declared_ret, Type::Stream(_)) && !self.saw_yield {
            self.warnings.push(TypeWarning::new(
                TypeWarningKind::StreamReturnWithoutYield {
                    agent: a.name.name.clone(),
                },
                a.span,
            ));
        }

        // Phase 21 slice 21-inv-A: enforce `@replayable`. An agent
        // carrying the attribute must call only functions whose
        // outputs the trace schema can capture; anything in the
        // determinism catalog (clocks, PRNGs, environment reads,
        // etc.) is off-limits.
        //
        // The catalog is empty as of Phase 21 v1 because Corvid
        // source does not yet expose any nondeterministic builtins.
        // The walk runs anyway so the enforcement path is live and
        // ready to fire the moment an entry lands.
        // Durable-job policy annotations (slice 45q).
        for attr in &a.attributes {
            match attr {
                AgentAttribute::Retry {
                    max_attempts, span, ..
                } => {
                    if *max_attempts == 0 {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::AnnotationInvalid {
                                annotation: "retry".into(),
                                message: "max_attempts must be at least 1".into(),
                            },
                            *span,
                        ));
                    }
                }
                AgentAttribute::Idempotency { key, span } => {
                    match a.params.iter().find(|p| p.name.name == key.name) {
                        None => {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::AnnotationInvalid {
                                    annotation: "idempotency".into(),
                                    message: format!(
                                        "`{}` names no parameter of agent `{}`",
                                        key.name, a.name.name
                                    ),
                                },
                                *span,
                            ));
                        }
                        Some(p) => {
                            let ty = self.type_ref_to_type(&p.ty);
                            if !matches!(ty, Type::String | Type::Int | Type::Unknown) {
                                self.errors.push(TypeError::new(
                                    TypeErrorKind::AnnotationInvalid {
                                        annotation: "idempotency".into(),
                                        message: format!(
                                            "key parameter `{}` must be String or Int (a stable, hashable job key), got {}",
                                            key.name,
                                            ty.display_name()
                                        ),
                                    },
                                    *span,
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if AgentAttribute::is_replayable(&a.attributes) {
            self.check_replayable_body(&a.name.name, &a.body);
        }

        // Phase 21 slice 21-inv-F: enforce `@deterministic`. Strictly
        // stronger than `@replayable` Ã¢â‚¬â€ the body must be a pure
        // function of its parameters. No LLM / tool / approve
        // calls, no catalog-registered nondeterminism, and calls
        // to other agents must target agents that are themselves
        // marked `@deterministic`.
        if AgentAttribute::is_deterministic(&a.attributes) {
            self.check_deterministic_body(&a.name.name, &a.body);
        }

        // Provenance Propagation D6 (slice 9): enforce
        // `@grounded_pure`. No `Grounded<T> -> T` laundering
        // anywhere in the body — neither the silent legacy
        // coercion (slice 7a recorded sites) nor the explicit
        // `.unwrap_discarding_sources()` call. Calls to other
        // agents must target agents that are themselves marked
        // `@grounded_pure` so the moat composes through the
        // call graph (R5).
        if AgentAttribute::is_grounded_pure(&a.attributes) {
            self.check_grounded_pure_body(&a.name.name, &a.body);
        }

        self.current_return = prev_ret;
        self.in_agent_body = prev_in_agent;
        self.saw_yield = prev_saw_yield;
        self.approvals_seen_in_agent = prev_approvals_seen;
        // (Locals leak between agents in our single-scope model; harmless
        //  since each agent binds its params fresh at the start.)
    }

    pub(super) fn check_server(&mut self, server: &ServerDecl) {
        let mut seen = HashSet::new();
        for route in &server.routes {
            let key = (route.method, route.path.clone());
            if !seen.insert(key) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::DuplicateServerRoute {
                        server: server.name.name.clone(),
                        method: route.method.as_str().into(),
                        path: route.path.clone(),
                    },
                    route.span,
                ));
            }
            if matches!(route.method, HttpMethod::Get) && route.body_ty.is_some() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::GetRouteBody {
                        server: server.name.name.clone(),
                        path: route.path.clone(),
                    },
                    route.span,
                ));
            }
            self.check_http_route(server, route);
        }
    }

    /// Validate an `identity Name:` block (slice 51g). The safe
    /// defaults are mandatory: at least one provider, well-formed OIDC
    /// discovery URLs, and secure/http_only cookies with a non-`none`
    /// SameSite plus session rotation. Disabling any of those is only
    /// allowed with an explicit `insecure_opt_out: true`, and even then
    /// a warning records the deliberately weakened posture.
    pub(super) fn check_identity(&mut self, decl: &IdentityDecl) {
        let name = decl.name.name.clone();
        let invalid = |message: String, span: Span| {
            TypeError::new(
                TypeErrorKind::IdentityConfigInvalid {
                    identity: name.clone(),
                    message,
                },
                span,
            )
        };

        if decl.providers.is_empty() {
            self.errors.push(invalid(
                "an identity block must declare at least one `provider`".into(),
                decl.span,
            ));
        }

        let mut seen = HashSet::new();
        for provider in &decl.providers {
            let wire = provider.kind.wire_name();
            if !seen.insert(wire.clone()) {
                self.errors.push(invalid(
                    format!("duplicate provider `{wire}`"),
                    provider.span,
                ));
            }
            if let ProviderKind::Oidc { discovery_url, .. } = &provider.kind {
                if !discovery_url.starts_with("https://") {
                    self.errors.push(invalid(
                        format!(
                            "the OIDC discovery URL must be an absolute `https://` URL, got `{discovery_url}`"
                        ),
                        provider.span,
                    ));
                }
            }
        }

        // First-login provisioning policy (slice 52e). A block that
        // declares OAuth providers MUST also declare how an unknown
        // verified subject is provisioned — there is no silent default,
        // because the choice sets the app's registration/tenancy posture
        // (see CLAUDE.md → "No hidden defaults for consequential policy").
        if !decl.providers.is_empty() {
            match &decl.provisioning {
                None => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::FirstLoginPolicyRequired {
                            identity: name.clone(),
                        },
                        decl.span,
                    ));
                }
                Some(provisioning) => {
                    self.check_provisioning(&name, provisioning);
                }
            }
        } else if let Some(provisioning) = &decl.provisioning {
            // A `provisioning:` block with no providers has nothing to
            // provision — the policy can never fire, so flag it rather
            // than let it read as if it were in force.
            self.errors.push(invalid(
                "`provisioning:` requires at least one OAuth `provider`; without a provider no first login can occur".into(),
                provisioning.span,
            ));
        }

        // Role → permission declarations (slice 52f). Roles must be
        // uniquely named and grant at least one permission; a
        // `default_role` names the role an open signup receives, so it
        // too must be declared (granting authority is never a typo).
        let mut declared_roles = HashSet::new();
        for role in &decl.roles {
            if !declared_roles.insert(role.name.clone()) {
                self.errors.push(invalid(
                    format!("duplicate role `{}`", role.name),
                    role.span,
                ));
            }
            if role.permissions.is_empty() {
                self.errors.push(invalid(
                    format!(
                        "role `{}` grants no permissions — list at least one, e.g. `{}: \"user:read\"`",
                        role.name, role.name
                    ),
                    role.span,
                ));
            }
        }
        if let Some(provisioning) = &decl.provisioning {
            if let Some(default_role) = &provisioning.default_role {
                if !declared_roles.contains(default_role) {
                    self.errors.push(invalid(
                        format!(
                            "`default_role: {default_role}` names a role that is not declared in `roles:`"
                        ),
                        provisioning.span,
                    ));
                }
            }
        }

        if let Some(session) = &decl.session {
            let cookie = &session.cookie;
            let mut unsafe_reasons = Vec::new();
            if !cookie.secure {
                unsafe_reasons.push("`secure` is off");
            }
            if !cookie.http_only {
                unsafe_reasons.push("`http_only` is off");
            }
            if matches!(cookie.same_site, SameSite::None) {
                unsafe_reasons.push("`same_site: none`");
            }
            if !session.rotate_on_privilege_change {
                unsafe_reasons.push("`rotate_on_privilege_change` is off");
            }

            if !unsafe_reasons.is_empty() {
                if cookie.insecure_opt_out {
                    // Allowed, but never silent: record the weakened
                    // posture so it shows up in review and audit.
                    self.warnings.push(TypeWarning::new(
                        TypeWarningKind::IdentityInsecureSession {
                            identity: name.clone(),
                            reasons: unsafe_reasons.join(", "),
                        },
                        session.span,
                    ));
                } else {
                    self.errors.push(invalid(
                        format!(
                            "unsafe session configuration ({}) requires an explicit `insecure_opt_out: true`",
                            unsafe_reasons.join(", ")
                        ),
                        session.span,
                    ));
                }
            }

            // `same_site: none` is only meaningful when the cookie is
            // also `secure`; browsers reject the combination otherwise.
            if matches!(cookie.same_site, SameSite::None) && !cookie.secure {
                self.errors.push(invalid(
                    "`same_site: none` requires `secure` cookies (browsers reject SameSite=None without Secure)".into(),
                    session.span,
                ));
            }
        }

        // Account-linking policy (slice 51i). The explicit-confirmation
        // flow is structural (there is no way to turn it off), so the
        // only thing to validate is the email-match policy and its
        // verified domains.
        if let Some(linking) = &decl.linking {
            match linking.email_match {
                EmailMatchPolicy::Never => {
                    if !linking.verified_domains.is_empty() {
                        self.errors.push(invalid(
                            "`verified_domains` only applies to `email_match: verified_domain`; with `never` there is no domain-scoped linking".into(),
                            linking.span,
                        ));
                    }
                }
                EmailMatchPolicy::VerifiedDomain => {
                    if linking.verified_domains.is_empty() {
                        self.errors.push(invalid(
                            "`email_match: verified_domain` requires a non-empty `verified_domains` list — otherwise no email match can ever be trusted".into(),
                            linking.span,
                        ));
                    }
                    for domain in &linking.verified_domains {
                        if !is_plausible_domain(domain) {
                            self.errors.push(invalid(
                                format!("`{domain}` is not a bare domain (expected e.g. `example.com`, no scheme/path/`@`)"),
                                linking.span,
                            ));
                        }
                    }
                }
            }
        }
    }

    /// Validate a first-login `provisioning:` policy (slice 52e). The
    /// policy must name only postures the runtime can execute completely
    /// today, and a tenant must never be derived from a source that does
    /// not match the chosen first-login mode.
    fn check_provisioning(&mut self, identity: &str, provisioning: &ProvisioningPolicy) {
        let invalid = |message: String, span: Span| {
            TypeError::new(
                TypeErrorKind::IdentityConfigInvalid {
                    identity: identity.to_string(),
                    message,
                },
                span,
            )
        };

        // `approval_required` is parsed so the checker can name it, but
        // the runtime cannot execute durable approval until slice 52f. A
        // policy is never silently degraded to a weaker one, so reject it
        // rather than fall back to `open`/`invited`.
        if matches!(provisioning.first_login, FirstLoginPolicy::ApprovalRequired) {
            self.errors.push(invalid(
                "`first_login: approval_required` is not executable yet — durable approval arrives in a later slice. Use `open` or `invited` today.".into(),
                provisioning.span,
            ));
        }

        match &provisioning.tenant {
            // A tenant taken from the invitation only makes sense when a
            // verified invitation actually gates the login.
            TenantAssignment::FromInvitation
                if !matches!(provisioning.first_login, FirstLoginPolicy::Invited) =>
            {
                self.errors.push(invalid(
                    "`tenant: from_invitation` requires `first_login: invited` — there is no invitation to read a tenant from otherwise".into(),
                    provisioning.span,
                ));
            }
            // An allowlisted claim mapping with an empty allowlist would
            // admit every claim value, defeating the point of the gate.
            TenantAssignment::ClaimMapping { allowlist, .. } if allowlist.is_empty() => {
                self.errors.push(invalid(
                    "`tenant: from_claim(...)` requires a non-empty `allow \"...\"` list — an empty allowlist would trust any claim value".into(),
                    provisioning.span,
                ));
            }
            _ => {}
        }
    }

    /// Validate a `connector` declaration (slice 52g): the base URL, the
    /// auth secret references, the reliability values, and each
    /// operation's path placeholders.
    pub(super) fn check_connector(&mut self, decl: &ConnectorDecl) {
        let name = decl.name.name.clone();
        let invalid = |message: String, span: Span| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid {
                    connector: name.clone(),
                    message,
                },
                span,
            )
        };

        // `base_url` must be an absolute `http(s)://` URL.
        if !(decl.base_url.starts_with("https://") || decl.base_url.starts_with("http://")) {
            self.errors.push(invalid(
                format!(
                    "`base_url` must be an absolute `http(s)://` URL, got `{}`",
                    decl.base_url
                ),
                decl.span,
            ));
        }

        // Auth credentials are `secret(...)` references — their names must
        // be non-empty (the parser already forbids literals).
        if let Some(auth) = &decl.auth {
            let mut secret_names: Vec<&str> = Vec::new();
            match auth {
                ConnectorAuth::Bearer(s) => secret_names.push(&s.name),
                ConnectorAuth::Header { name: header, value } => {
                    if header.trim().is_empty() {
                        self.errors.push(invalid(
                            "a `header(...)` auth needs a non-empty header name".into(),
                            decl.span,
                        ));
                    }
                    secret_names.push(&value.name);
                }
                ConnectorAuth::Basic { username, password } => {
                    secret_names.push(&username.name);
                    secret_names.push(&password.name);
                }
            }
            if secret_names.iter().any(|n| n.trim().is_empty()) {
                self.errors.push(invalid(
                    "a `secret(...)` reference must name a non-empty secret".into(),
                    decl.span,
                ));
            }
        }

        // Reliability values, when present, must be non-zero to mean
        // anything.
        if decl.retry == Some(0) {
            self.errors.push(invalid(
                "`retry: 0` is a no-op — omit it, or give a positive count".into(),
                decl.span,
            ));
        }
        if decl.circuit_breaker == Some(0) {
            self.errors.push(invalid(
                "`circuit_breaker: 0` would trip immediately — give a positive threshold".into(),
                decl.span,
            ));
        }
        if let Some(rate) = &decl.rate_limit {
            if rate.limit == 0 || rate.window_secs == 0 {
                self.errors.push(invalid(
                    "`rate_limit` needs a positive limit and window (`<n> per <secs>s`)".into(),
                    decl.span,
                ));
            }
        }

        // A connector MUST declare its allowed execution modes — there
        // is no default. Whether an operation reaches a real external
        // provider is a consequential choice, so silence is a compile
        // error naming the decision and its allowed values, never a
        // default posture the author didn't choose.
        if decl.modes.is_empty() {
            self.errors.push(invalid(
                "a connector must declare its allowed `modes: [...]` — one or more of `mock`, `replay`, `real`. There is no default; the deployment selects one at start (`corvid dev --mode mock`)".into(),
                decl.span,
            ));
        }
        let mut seen_modes = HashSet::new();
        for m in &decl.modes {
            if !seen_modes.insert(m.as_str()) {
                self.errors.push(invalid(
                    format!("mode `{}` is listed more than once in `modes`", m.as_str()),
                    decl.span,
                ));
            }
        }
        let mock_allowed = decl.modes.contains(&ConnectorMode::Mock);

        // Each operation: a unique name, a path that starts with `/`, and
        // `{placeholders}` that name declared parameters.
        let mut seen = HashSet::new();
        for op in &decl.operations {
            if !seen.insert(op.name.name.clone()) {
                self.errors.push(invalid(
                    format!("duplicate operation `{}`", op.name.name),
                    op.span,
                ));
            }
            if !op.path.starts_with('/') {
                self.errors.push(invalid(
                    format!("operation `{}` path must start with `/`, got `{}`", op.name.name, op.path),
                    op.span,
                ));
            }
            let param_names: HashSet<&str> =
                op.params.iter().map(|p| p.name.name.as_str()).collect();
            for placeholder in path_placeholders(&op.path) {
                if !param_names.contains(placeholder.as_str()) {
                    self.errors.push(invalid(
                        format!(
                            "operation `{}` path references `{{{placeholder}}}`, but it declares no parameter named `{placeholder}`",
                            op.name.name
                        ),
                        op.span,
                    ));
                }
            }
            // A `body <param>` / `form <param>` must name a declared
            // parameter.
            if let Some(body) = &op.body {
                if !param_names.contains(body.param.name.as_str()) {
                    self.errors.push(invalid(
                        format!(
                            "operation `{}` body references `{}`, which is not a declared parameter",
                            op.name.name, body.param.name
                        ),
                        op.span,
                    ));
                }
            }

            // The `mock:` payload (52g-3b). If `mock` is one of the
            // allowed modes, every operation must declare one so mock
            // mode is fully serveable without a recorded cassette. A
            // declared mock is type-checked against the operation's
            // return type, with the operation's params in scope (so a
            // mock may reference them, e.g. `mock: Repo(owner)`).
            match &op.mock {
                Some(mock_expr) => {
                    self.bind_params(&op.params);
                    let ret_ty = self.type_ref_to_type(&op.return_ty);
                    let got = self.check_expr_as(mock_expr, Some(&ret_ty));
                    if !got.is_assignable_to(&ret_ty) {
                        self.errors.push(invalid(
                            format!(
                                "operation `{}` mock has type `{}`, but the operation returns `{}`",
                                op.name.name,
                                got.display_name(),
                                ret_ty.display_name()
                            ),
                            mock_expr.span(),
                        ));
                    }
                }
                None if mock_allowed => {
                    self.errors.push(invalid(
                        format!(
                            "operation `{}` needs a `mock:` payload because `mock` is one of the connector's allowed modes — mock mode must be fully serveable",
                            op.name.name
                        ),
                        op.span,
                    ));
                }
                None => {}
            }
        }
    }

    fn check_http_route(&mut self, server: &ServerDecl, route: &HttpRouteDecl) {
        let path_fields = route
            .path_params
            .iter()
            .map(|param| (param.name.name.clone(), self.type_ref_to_type(&param.ty)))
            .collect();
        self.bind_route_local_by_name(&route.body, "path", Type::RouteParams(path_fields));
        if let Some(query_ty) = &route.query_ty {
            let ty = self.type_ref_to_type(query_ty);
            self.bind_route_local_by_name(&route.body, "query", ty);
        }
        if let Some(body_ty) = &route.body_ty {
            let ty = self.type_ref_to_type(body_ty);
            self.bind_route_local_by_name(&route.body, "body", ty);
        }

        // Auth policy (slice 51h): a `requires` clause needs an
        // `identity` block to give it meaning, and binds a typed
        // `actor` in the route body.
        if let Some(policy) = &route.policy {
            if policy.requires_auth() {
                if !self.has_identity {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::RoutePolicyWithoutIdentity {
                            server: server.name.name.clone(),
                            path: route.path.clone(),
                        },
                        policy.span,
                    ));
                }
                // A `requires role/permission` clause must reference a name
                // declared in the identity block's `roles:` — otherwise the
                // requirement can never be satisfied and is almost certainly
                // a typo (slice 52f). Only checked when an identity block
                // exists (the missing-identity error above already fires).
                if self.has_identity {
                    for role in &policy.roles {
                        if !self.identity_roles.contains(role) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::RoutePolicyUndeclaredAuthority {
                                    kind: "role",
                                    name: role.clone(),
                                    path: route.path.clone(),
                                },
                                policy.span,
                            ));
                        }
                    }
                    for permission in &policy.permissions {
                        if !self.identity_permissions.contains(permission) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::RoutePolicyUndeclaredAuthority {
                                    kind: "permission",
                                    name: permission.clone(),
                                    path: route.path.clone(),
                                },
                                policy.span,
                            ));
                        }
                    }
                }
                self.bind_route_local_by_name(&route.body, "actor", actor_type());
            }
        }

        let declared_ret = self.type_ref_to_type(&route.response.ty);
        let prev_ret = std::mem::replace(&mut self.current_return, Some(declared_ret));
        let prev_in_agent = std::mem::replace(&mut self.in_agent_body, true);
        let prev_saw_yield = std::mem::replace(&mut self.saw_yield, false);

        self.check_block(&route.body);

        self.current_return = prev_ret;
        self.in_agent_body = prev_in_agent;
        self.saw_yield = prev_saw_yield;

        let _ = server;
    }

    fn bind_route_local_by_name(&mut self, block: &Block, name: &str, ty: Type) {
        let mut spans = Vec::new();
        collect_ident_spans_by_name_in_block(block, name, &mut spans);
        for span in spans {
            if let Some(Binding::Local(local_id)) = self.bindings.get(&span).cloned() {
                self.local_types.insert(local_id, ty.clone());
            }
        }
    }
}

/// The synthetic struct type bound as `actor` in an authenticated
/// route body (slice 51h). Modeled with the same `RouteParams`
/// machinery `path`/`query` use, so field access is fully typed
/// without a dedicated `Type` variant. Mirrors the runtime's
/// `AuthActor`: identity + tenant + display name + role/permission
/// sets. Provider tokens are deliberately ABSENT — the login actor
/// never carries connector workspace credentials (slice 51j).
fn actor_type() -> Type {
    Type::RouteParams(vec![
        ("id".to_string(), Type::String),
        ("tenant".to_string(), Type::String),
        ("display_name".to_string(), Type::String),
        ("roles".to_string(), Type::List(Box::new(Type::String))),
        ("permissions".to_string(), Type::List(Box::new(Type::String))),
    ])
}

/// A conservative "looks like a bare registrable domain" check for
/// verified-domain linking (slice 51i): at least one dot, no scheme,
/// path, whitespace, or `@`, and non-empty labels.
fn is_plausible_domain(domain: &str) -> bool {
    if domain.is_empty()
        || domain.contains("://")
        || domain.contains('/')
        || domain.contains('@')
        || domain.chars().any(|c| c.is_whitespace())
    {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    labels.len() >= 2 && labels.iter().all(|l| !l.is_empty())
}

/// Extract the `{placeholder}` names from an operation path (slice 52g),
/// e.g. `/repos/{owner}/{repo}` → `["owner", "repo"]`.
fn path_placeholders(path: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        if let Some(close) = rest.find('}') {
            let name = rest[..close].trim();
            if !name.is_empty() {
                names.push(name.to_string());
            }
            rest = &rest[close + 1..];
        } else {
            break;
        }
    }
    names
}

fn collect_ident_spans_by_name_in_block(block: &Block, name: &str, spans: &mut Vec<Span>) {
    for stmt in &block.stmts {
        collect_ident_spans_by_name_in_stmt(stmt, name, spans);
    }
}

fn collect_ident_spans_by_name_in_stmt(stmt: &Stmt, name: &str, spans: &mut Vec<Span>) {
    match stmt {
        Stmt::Let { value, .. } => collect_ident_spans_by_name_in_expr(value, name, spans),
        Stmt::Return { value, .. } => {
            if let Some(expr) = value {
                collect_ident_spans_by_name_in_expr(expr, name, spans);
            }
        }
        Stmt::Yield { value, .. } => collect_ident_spans_by_name_in_expr(value, name, spans),
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            collect_ident_spans_by_name_in_expr(cond, name, spans);
            collect_ident_spans_by_name_in_block(then_block, name, spans);
            if let Some(block) = else_block {
                collect_ident_spans_by_name_in_block(block, name, spans);
            }
        }
        Stmt::For { iter, body, .. } => {
            collect_ident_spans_by_name_in_expr(iter, name, spans);
            collect_ident_spans_by_name_in_block(body, name, spans);
        }
        Stmt::While { cond, body, .. } => {
            collect_ident_spans_by_name_in_expr(cond, name, spans);
            collect_ident_spans_by_name_in_block(body, name, spans);
        }
        Stmt::Destructure { value, .. } => {
            collect_ident_spans_by_name_in_expr(value, name, spans);
        }
        Stmt::Parallel { arms, .. } => {
            for arm in arms {
                collect_ident_spans_by_name_in_expr(&arm.call, name, spans);
            }
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Pass { .. } => {}
        Stmt::Approve { action, .. } => collect_ident_spans_by_name_in_expr(action, name, spans),
        Stmt::Expr { expr, .. } => collect_ident_spans_by_name_in_expr(expr, name, spans),
        Stmt::Assign { target, value, .. } => {
            collect_ident_spans_by_name_in_expr(target, name, spans);
            collect_ident_spans_by_name_in_expr(value, name, spans);
        }
    }
}

fn collect_ident_spans_by_name_in_expr(expr: &Expr, name: &str, spans: &mut Vec<Span>) {
    match expr {
        Expr::Ident { name: ident, .. } if ident.name == name => spans.push(ident.span),
        Expr::Ident { .. } | Expr::Literal { .. } => {}
        Expr::Call { callee, args, .. } => {
            collect_ident_spans_by_name_in_expr(callee, name, spans);
            for arg in args {
                collect_ident_spans_by_name_in_expr(arg, name, spans);
            }
        }
        Expr::FieldAccess { target, .. } | Expr::TryPropagate { inner: target, .. } => {
            collect_ident_spans_by_name_in_expr(target, name, spans);
        }
        Expr::Lambda { body, .. } => {
            collect_ident_spans_by_name_in_expr(body, name, spans);
        }
        Expr::StructLiteral { fields, spread, .. } => {
            for f in fields {
                if let Some(v) = &f.value {
                    collect_ident_spans_by_name_in_expr(v, name, spans);
                }
            }
            if let Some(s) = spread {
                collect_ident_spans_by_name_in_expr(s, name, spans);
            }
        }
        Expr::Index { target, index, .. } => {
            collect_ident_spans_by_name_in_expr(target, name, spans);
            collect_ident_spans_by_name_in_expr(index, name, spans);
        }
        Expr::BinOp { left, right, .. } => {
            collect_ident_spans_by_name_in_expr(left, name, spans);
            collect_ident_spans_by_name_in_expr(right, name, spans);
        }
        Expr::UnOp { operand, .. } => collect_ident_spans_by_name_in_expr(operand, name, spans),
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_ident_spans_by_name_in_expr(scrutinee, name, spans);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_ident_spans_by_name_in_expr(g, name, spans);
                }
                collect_ident_spans_by_name_in_expr(&arm.body, name, spans);
            }
        }
        Expr::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_ident_spans_by_name_in_expr(k, name, spans);
                collect_ident_spans_by_name_in_expr(v, name, spans);
            }
        }
        Expr::List { items, .. } => {
            for item in items {
                collect_ident_spans_by_name_in_expr(item, name, spans);
            }
        }
        Expr::TrustBoundary { inner: body, .. } | Expr::TryRetry { body, .. } => {
            collect_ident_spans_by_name_in_expr(body, name, spans)
        }
        Expr::Replay {
            trace,
            arms,
            else_body,
            ..
        } => {
            collect_ident_spans_by_name_in_expr(trace, name, spans);
            for arm in arms {
                collect_ident_spans_by_name_in_expr(&arm.body, name, spans);
            }
            collect_ident_spans_by_name_in_expr(else_body, name, spans);
        }
    }
}
