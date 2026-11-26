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
    Effect, Expr,
    FirstLoginPolicy, HttpMethod, HttpRouteDecl, IdentityDecl, ProviderKind, ProvisioningPolicy,
    ProtocolPollInterval, ProviderProtocolDecl, SameSite, ServerDecl, Span, Stmt, TenantAssignment,
};
use corvid_resolve::{Binding, DefId};
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
        let prev_saw_stream_output = std::mem::replace(&mut self.saw_stream_output, false);
        // Reset the body-wide approve audit log; restore on exit.
        // This is what lets the dangerous-call diagnostic
        // discriminate `approval.token_lexical_only` (an approve
        // exists in this agent's body but is out of lexical scope
        // at the call site) from
        // `approval.dangerous_call_requires_token` (no approve
        // anywhere in the agent).
        let prev_approvals_seen = std::mem::take(&mut self.approvals_seen_in_agent);

        self.check_block(&a.body);

        if matches!(declared_ret, Type::Stream(_)) && !self.saw_stream_output {
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
        self.saw_stream_output = prev_saw_stream_output;
        self.approvals_seen_in_agent = prev_approvals_seen;
        // (Locals leak between agents in our single-scope model; harmless
        //  since each agent binds its params fresh at the start.)
    }

    pub(super) fn check_server(&mut self, server: &ServerDecl) {
        let mut seen = HashSet::new();
        let mut live_names = HashSet::new();
        for route in &server.routes {
            if route.is_live() {
                self.check_http_route(server, route);
                self.check_live(route, &mut live_names);
                continue;
            }
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
            if matches!(route.method, HttpMethod::Head | HttpMethod::Options) {
                self.errors.push(TypeError::new(
                    TypeErrorKind::RouteMethodAnsweredByServer {
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

    /// A `live` declaration (B14): its body is one `return` of a table
    /// query on the row type it delivers, without `take`, and its name,
    /// the path's last fixed segment, is unique in the server. The body
    /// is otherwise checked as a GET route's is, so `path`, `actor` and
    /// `requires` mean what they mean on a route.
    fn check_live(&mut self, route: &corvid_ast::HttpRouteDecl, names: &mut HashSet<String>) {
        let invalid = |checker: &mut Self, problem: String| {
            checker.errors.push(TypeError::new(
                TypeErrorKind::LiveDeclarationInvalid { path: route.path.clone(), problem },
                route.span,
            ));
        };
        match route.live_name() {
            None => invalid(self, "has no fixed path segment to name it by; end the path with one, as in `/boards/{board}/cards`".into()),
            Some(name) if !names.insert(name.to_string()) => {
                invalid(self, format!("is named `{name}`, the same as another `live` in this server; give one a different last path segment"))
            }
            Some(_) => {}
        }
        let row_name = match route.response.live_row() {
            Some(corvid_ast::TypeRef::Named { name, .. }) => name.name.clone(),
            _ => {
                invalid(self, "must deliver a table's rows, as in `-> Card`".into());
                return;
            }
        };
        let is_table = self.file.decls.iter().any(|decl| matches!(decl, corvid_ast::Decl::Table(t) if t.name().name == row_name));
        if !is_table {
            invalid(self, format!("delivers `{row_name}`, which is not a table; a `live` delivers the rows of a `table` declaration"));
            return;
        }
        let returned = match route.body.stmts.as_slice() {
            [corvid_ast::Stmt::Return { value: Some(value), .. }] => value,
            _ => {
                invalid(self, "must be one `return` of a table query".into());
                return;
            }
        };
        let query = match returned {
            corvid_ast::Expr::Call { callee, args, .. } => {
                crate::table_ops::table_query_call(callee, args, self.file, self.symbols, self.bindings)
            }
            _ => None,
        };
        let Some(query) = query else {
            invalid(self, "must return a table query, such as `Card.where(fn (c) -> ...)` or `Card.all()`".into());
            return;
        };
        if query.table.name().name != row_name {
            invalid(self, format!("delivers `{row_name}` but queries `{}`", query.table.name().name));
        }
        if query.take.is_some() {
            invalid(self, "uses `take`; a live query keeps every matching row current, so a moving top-N is not supported yet".into());
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
        if let Some(base_url) = &decl.base_url {
            if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
                self.errors.push(invalid(
                    format!("`base_url` must be an absolute `http(s)://` URL, got `{base_url}`"),
                    decl.span,
                ));
            }
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

        // How long a request may take and where a redirect may send it
        // are consequential choices with more than one reasonable answer,
        // so the source declares both; silence is a compile error naming
        // the decision and its values, never a default.
        match decl.timeout_secs {
            None => self.errors.push(invalid(
                "a connector must declare `timeout: <n>s`, the longest one request may take from connecting to the last byte of the response (for example `timeout: 10s`). There is no default timeout; an operation may override it with its own `timeout:`".into(),
                decl.span,
            )),
            Some(secs) => {
                if let Some(message) = connector_timeout_problem("the connector", secs) {
                    self.errors.push(invalid(message, decl.span));
                }
            }
        }
        if decl.redirects.is_none() {
            self.errors.push(invalid(
                "a connector must declare `redirects:` — `never` (a 3xx is the answer), `same_host` (follow only to the same scheme, host and port), or `allowed_hosts` (follow to any host in `[http] allow`). There is no default: where a request may be sent next is a security decision".into(),
                decl.span,
            ));
        }
        if decl.published.is_none() {
            self.errors.push(invalid(
                format!(
                    "connector `{}` must declare whether the published contract lists it: add `published: no` (backend only) or `published: yes` (its name, URL, paths and protocol are listed at /.well-known/corvid). There is no default: which outside services an app calls, and where, is a security decision",
                    decl.name.name
                ),
                decl.span,
            ));
        }

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
            self.check_operation_target(decl, op);
            if op.url_param.is_none() && !op.path.starts_with('/') {
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

            self.check_operation_query_and_headers(decl, op, &param_names);
            self.check_operation_response_shape(decl, op);
            self.check_operation_on_failure(decl, op);
            self.check_operation_stream(decl, op);
            self.check_operation_body(decl, op);

            if let Some(secs) = op.timeout_secs {
                let subject = format!("operation `{}`", op.name.name);
                if let Some(message) = connector_timeout_problem(&subject, secs) {
                    self.errors.push(invalid(message, op.span));
                }
            }

            if let Some(protocol) = &op.protocol {
                for (message, span) in validate_provider_protocol(op, protocol) {
                    self.errors.push(invalid(message, span));
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
                    // A `Response<Body>` operation's mock is the body: mock
                    // mode answers with status 200 and no headers.
                    let ret_ty = mock_payload_type(self.type_ref_to_type(&op.return_ty));
                    let got = self.check_expr_as(mock_expr, Some(&ret_ty));
                    if !got.is_assignable_to(&ret_ty) {
                        self.errors.push(invalid(
                            format!(
                                "operation `{}` mock has type `{}`, but the operation's mock is `{}`",
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

            // `on status <code> -> Variant` coherence (slice 52g-3c-5,
            // closing the 52g-2 deferral). An operation that maps statuses
            // to typed errors must return `Result<Success, ErrorEnum>`,
            // and every mapped variant must be a fieldless variant of that
            // error enum — a status carries no payload to fill fields.
            if !op.error_map.is_empty() {
                let ret = self.type_ref_to_type(&op.return_ty);
                match &ret {
                    Type::Result(_ok, err) => match err.as_ref() {
                        Type::Struct(enum_id) => {
                            let enum_decl = self.types_by_id.get(enum_id).copied();
                            match enum_decl {
                                Some(decl) if !decl.variants.is_empty() => {
                                    for mapping in &op.error_map {
                                        match decl
                                            .variants
                                            .iter()
                                            .find(|v| v.name.name == mapping.variant.name)
                                        {
                                            None => self.errors.push(invalid(
                                                format!(
                                                    "operation `{}` maps status {} to `{}`, which is not a variant of the error type `{}`",
                                                    op.name.name, mapping.status, mapping.variant.name, decl.name.name
                                                ),
                                                mapping.span,
                                            )),
                                            Some(v) if !v.fields.is_empty() => self.errors.push(invalid(
                                                format!(
                                                    "operation `{}` maps status {} to `{}`, but that variant has fields — a status can only map to a fieldless variant",
                                                    op.name.name, mapping.status, mapping.variant.name
                                                ),
                                                mapping.span,
                                            )),
                                            Some(_) => {}
                                        }
                                    }
                                }
                                _ => self.errors.push(invalid(
                                    format!(
                                        "operation `{}` maps statuses to error variants, so its error type must be an enum",
                                        op.name.name
                                    ),
                                    op.span,
                                )),
                            }
                        }
                        other => self.errors.push(invalid(
                            format!(
                                "operation `{}` maps statuses to error variants, so its error type must be an enum, got `{}`",
                                op.name.name,
                                other.display_name()
                            ),
                            op.span,
                        )),
                    },
                    other => self.errors.push(invalid(
                        format!(
                            "operation `{}` declares `on status` error mappings, so it must return `Result<Success, ErrorEnum>` — got `{}`",
                            op.name.name,
                            other.display_name()
                        ),
                        op.span,
                    )),
                }
            }
        }
    }

    fn check_http_route(&mut self, server: &ServerDecl, route: &HttpRouteDecl) {
        let body_is_upload = route
            .body_ty
            .as_ref()
            .is_some_and(is_direct_upload_type);
        match (&route.upload, body_is_upload) {
            (None, true) => self.errors.push(TypeError::new(
                TypeErrorKind::RouteUploadPolicyInvalid {
                    path: route.path.clone(),
                    message: "`body Upload<Format>` requires an explicit `@upload(max_mb: N)` policy; there is no runtime default".into(),
                },
                route.span,
            )),
            (Some(spec), true) if spec.max_bytes.is_none() => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::RouteUploadPolicyInvalid {
                        path: route.path.clone(),
                        message: "`@upload` on a route must declare `max_mb` or `max_bytes`"
                            .into(),
                    },
                    spec.span,
                ));
            }
            (Some(spec), false) => self.errors.push(TypeError::new(
                TypeErrorKind::RouteUploadPolicyInvalid {
                    path: route.path.clone(),
                    message: "`@upload(...)` is only valid on a route with `body Upload<Format>`"
                        .into(),
                },
                spec.span,
            )),
            _ => {}
        }

        let path_fields = route
            .path_params
            .iter()
            .map(|param| {
                let ty = self.type_ref_to_type(&param.ty);
                // A path segment is text the server reads before the
                // handler runs, through the shape the server reads too.
                let plain = |ty: &Type| self.has_plain_variants(ty);
                if ty != Type::Unknown && crate::text_parameters::text_shape(&ty, &plain).is_none() {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::RoutePathParamType {
                            path: route.path.clone(),
                            param: param.name.name.clone(),
                            ty: ty.display_name(),
                        },
                        param.ty.span(),
                    ));
                }
                (param.name.name.clone(), ty)
            })
            .collect();
        self.bind_route_local_by_name(&route.body, "path", Type::RouteParams(path_fields));
        if let Some(query_ty) = &route.query_ty {
            let ty = self.type_ref_to_type(query_ty);
            self.check_route_query_fields(route, &ty, query_ty.span());
            self.bind_route_local_by_name(&route.body, "query", ty);
        }
        if let Some(body_ty) = &route.body_ty {
            let ty = self.type_ref_to_type(body_ty);
            if route.form_encoded {
                self.check_form_record(&ty, body_ty.span());
            }
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
        let prev_saw_stream_output = std::mem::replace(&mut self.saw_stream_output, false);
        self.check_block(&route.body);
        let reaches_approval = block_reaches_approval(
            &route.body,
            self.bindings,
            &self.agents_by_id,
            self.methods,
            &self.types,
            &mut HashSet::new(),
        );

        self.current_return = prev_ret;
        self.in_agent_body = prev_in_agent;
        self.saw_stream_output = prev_saw_stream_output;

        if reaches_approval
            && !route
                .policy
                .as_ref()
                .is_some_and(corvid_ast::RoutePolicy::requires_auth)
        {
            self.errors.push(TypeError::new(
                TypeErrorKind::RouteApprovalRequiresAuthentication {
                    path: route.path.clone(),
                },
                route.span,
            ));
        }
        match (reaches_approval, route.approval.as_ref()) {
            (true, None) => self.errors.push(TypeError::with_guarantee(
                TypeErrorKind::RouteApprovalPolicyInvalid {
                    path: route.path.clone(),
                    message: "an approval-capable route must declare a complete `@approval(...)` policy; serve never invents reviewer authority or expiry".into(),
                },
                route.span,
                "approval.policy_clause_static_check",
            )),
            (false, Some(spec)) => self.errors.push(TypeError::with_guarantee(
                TypeErrorKind::RouteApprovalPolicyInvalid {
                    path: route.path.clone(),
                    message: "`@approval(...)` is only valid when the route can reach an `approve` boundary".into(),
                },
                spec.span,
                "approval.policy_clause_static_check",
            )),
            (true, Some(spec)) => {
                if !self.identity_roles.contains(&spec.role) {
                    self.errors.push(TypeError::with_guarantee(
                        TypeErrorKind::RouteApprovalPolicyInvalid {
                            path: route.path.clone(),
                            message: format!(
                                "reviewer role `{}` is not declared in the identity block",
                                spec.role
                            ),
                        },
                        spec.span,
                        "approval.policy_clause_static_check",
                    ));
                }
                if !self.identity_permissions.contains("approvals.decide") {
                    self.errors.push(TypeError::with_guarantee(
                        TypeErrorKind::RouteApprovalPolicyInvalid {
                            path: route.path.clone(),
                            message: "the identity roles grant no `approvals.decide` permission, so this approval could never be decided".into(),
                        },
                        spec.span,
                        "approval.policy_clause_static_check",
                    ));
                }
            }
            (false, None) => {}
        }

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

fn block_reaches_approval(
    block: &Block,
    bindings: &std::collections::HashMap<Span, Binding>,
    agents: &std::collections::HashMap<DefId, &AgentDecl>,
    methods: &std::collections::HashMap<
        DefId,
        std::collections::HashMap<String, corvid_resolve::resolver::MethodEntry>,
    >,
    types: &std::collections::HashMap<Span, Type>,
    visited_agents: &mut HashSet<DefId>,
) -> bool {
    block.stmts.iter().any(|stmt| {
        stmt_reaches_approval(stmt, bindings, agents, methods, types, visited_agents)
    })
}

fn stmt_reaches_approval(
    stmt: &Stmt,
    bindings: &std::collections::HashMap<Span, Binding>,
    agents: &std::collections::HashMap<DefId, &AgentDecl>,
    methods: &std::collections::HashMap<
        DefId,
        std::collections::HashMap<String, corvid_resolve::resolver::MethodEntry>,
    >,
    types: &std::collections::HashMap<Span, Type>,
    visited_agents: &mut HashSet<DefId>,
) -> bool {
    match stmt {
        Stmt::Approve { .. } => true,
        Stmt::Let { value, .. }
        | Stmt::Yield { value, .. }
        | Stmt::Destructure { value, .. } => {
            expr_reaches_approval(value, bindings, agents, methods, types, visited_agents)
        }
        Stmt::Return { value, .. } => value.as_ref().is_some_and(|value| {
            expr_reaches_approval(value, bindings, agents, methods, types, visited_agents)
        }),
        Stmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            expr_reaches_approval(cond, bindings, agents, methods, types, visited_agents)
                || block_reaches_approval(
                    then_block,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
                || else_block.as_ref().is_some_and(|block| {
                    block_reaches_approval(
                        block,
                        bindings,
                        agents,
                        methods,
                        types,
                        visited_agents,
                    )
                })
        }
        Stmt::For { iter, body, .. } => {
            expr_reaches_approval(iter, bindings, agents, methods, types, visited_agents)
                || block_reaches_approval(
                    body,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }
        Stmt::Atomic { body, .. } => block_reaches_approval(body, bindings, agents, methods, types, visited_agents),
        Stmt::While { cond, body, .. } => {
            expr_reaches_approval(cond, bindings, agents, methods, types, visited_agents)
                || block_reaches_approval(
                    body,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }
        Stmt::Parallel { arms, .. } => arms.iter().any(|arm| {
            expr_reaches_approval(
                &arm.call,
                bindings,
                agents,
                methods,
                types,
                visited_agents,
            )
        }),
        Stmt::Expr { expr, .. } => {
            expr_reaches_approval(expr, bindings, agents, methods, types, visited_agents)
        }
        Stmt::Assign { target, value, .. } => {
            expr_reaches_approval(target, bindings, agents, methods, types, visited_agents)
                || expr_reaches_approval(
                    value,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }
        Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Pass { .. } => false,
    }
}

fn expr_reaches_approval(
    expr: &Expr,
    bindings: &std::collections::HashMap<Span, Binding>,
    agents: &std::collections::HashMap<DefId, &AgentDecl>,
    methods: &std::collections::HashMap<
        DefId,
        std::collections::HashMap<String, corvid_resolve::resolver::MethodEntry>,
    >,
    types: &std::collections::HashMap<Span, Type>,
    visited_agents: &mut HashSet<DefId>,
) -> bool {
    match expr {
        Expr::Call { callee, args, .. } => {
            let called_agent_reaches_approval = match callee.as_ref() {
                Expr::Ident { name, .. } => match bindings.get(&name.span) {
                    Some(Binding::Decl(def_id)) if visited_agents.insert(*def_id) => agents
                        .get(def_id)
                        .is_some_and(|agent| {
                            block_reaches_approval(
                                &agent.body,
                                bindings,
                                agents,
                                methods,
                                types,
                                visited_agents,
                            )
                        }),
                    _ => false,
                },
                Expr::FieldAccess { target, field, .. } => match types.get(&target.span()) {
                    Some(Type::Struct(type_id)) => methods
                        .get(type_id)
                        .and_then(|entries| entries.get(&field.name))
                        .filter(|entry| {
                            entry.kind == corvid_resolve::resolver::MethodKind::Agent
                        })
                        .filter(|entry| visited_agents.insert(entry.def_id))
                        .and_then(|entry| agents.get(&entry.def_id))
                        .is_some_and(|agent| {
                            block_reaches_approval(
                                &agent.body,
                                bindings,
                                agents,
                                methods,
                                types,
                                visited_agents,
                            )
                        }),
                    _ => false,
                },
                _ => false,
            };

            called_agent_reaches_approval
                || expr_reaches_approval(
                    callee,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
                || args.iter().any(|arg| {
                    expr_reaches_approval(
                        arg,
                        bindings,
                        agents,
                        methods,
                        types,
                        visited_agents,
                    )
                })
        }
        Expr::FieldAccess { target, .. }
        | Expr::UnOp {
            operand: target, ..
        }
        | Expr::Lambda { body: target, .. }
        | Expr::TryPropagate { inner: target, .. }
        | Expr::TrustBoundary { inner: target, .. }
        | Expr::TryRetry { body: target, .. } => {
            expr_reaches_approval(target, bindings, agents, methods, types, visited_agents)
        }
        Expr::Index { target, index, .. } | Expr::BinOp {
            left: target,
            right: index,
            ..
        } => {
            expr_reaches_approval(target, bindings, agents, methods, types, visited_agents)
                || expr_reaches_approval(
                    index,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }
        Expr::List { items, .. } => items.iter().any(|item| {
            expr_reaches_approval(item, bindings, agents, methods, types, visited_agents)
        }),
        Expr::Match {
            scrutinee, arms, ..
        } => {
            expr_reaches_approval(scrutinee, bindings, agents, methods, types, visited_agents)
                || arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(|guard| {
                        expr_reaches_approval(
                            guard,
                            bindings,
                            agents,
                            methods,
                            types,
                            visited_agents,
                        )
                    }) || expr_reaches_approval(
                        &arm.body,
                        bindings,
                        agents,
                        methods,
                        types,
                        visited_agents,
                    )
                })
        }
        Expr::StructLiteral { fields, spread, .. } => {
            fields.iter().any(|field| {
                field.value.as_ref().is_some_and(|value| {
                    expr_reaches_approval(
                        value,
                        bindings,
                        agents,
                        methods,
                        types,
                        visited_agents,
                    )
                })
            }) || spread.as_ref().is_some_and(|spread| {
                expr_reaches_approval(
                    spread,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
            })
        }
        Expr::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            expr_reaches_approval(key, bindings, agents, methods, types, visited_agents)
                || expr_reaches_approval(
                    value,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }),
        Expr::Replay {
            trace,
            arms,
            else_body,
            ..
        } => {
            expr_reaches_approval(trace, bindings, agents, methods, types, visited_agents)
                || arms.iter().any(|arm| {
                    expr_reaches_approval(
                        &arm.body,
                        bindings,
                        agents,
                        methods,
                        types,
                        visited_agents,
                    )
                })
                || expr_reaches_approval(
                    else_body,
                    bindings,
                    agents,
                    methods,
                    types,
                    visited_agents,
                )
        }
        Expr::Literal { .. } | Expr::Ident { .. } => false,
    }
}

fn is_direct_upload_type(ty: &corvid_ast::TypeRef) -> bool {
    matches!(
        ty,
        corvid_ast::TypeRef::Generic { name, args, .. }
            if name.name == "Upload" && args.len() == 1
    )
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

fn validate_provider_protocol(
    operation: &corvid_ast::OperationDecl,
    protocol: &ProviderProtocolDecl,
) -> Vec<(String, Span)> {
    let mut errors = Vec::new();
    let prefix = || format!("operation `{}` async protocol", operation.name.name);

    let mut statuses = HashSet::new();
    if protocol.statuses.is_empty() {
        errors.push((format!("{} must declare a non-empty `statuses` universe", prefix()), protocol.span));
    }
    for status in &protocol.statuses {
        if !statuses.insert(status.name.clone()) {
            errors.push((format!("{} declares provider status `{}` more than once", prefix(), status.name), status.span));
        }
    }

    let mut terminal = HashSet::new();
    if protocol.terminal.is_empty() {
        errors.push((format!("{} must declare at least one terminal state", prefix()), protocol.span));
    }
    for state in &protocol.terminal {
        if !terminal.insert(state.name.clone()) {
            errors.push((format!("{} declares terminal state `{}` more than once", prefix(), state.name), state.span));
        }
    }

    let mut nonterminal = HashSet::new();
    for state in &protocol.states {
        if terminal.contains(&state.name.name) {
            errors.push((format!("{} declares terminal state `{}` with outgoing transitions", prefix(), state.name.name), state.span));
        }
        if !nonterminal.insert(state.name.name.clone()) {
            errors.push((format!("{} declares state `{}` more than once", prefix(), state.name.name), state.span));
        }
    }
    let declared: HashSet<String> = nonterminal.union(&terminal).cloned().collect();
    if !declared.contains(&protocol.initial.name) {
        errors.push((format!("{} initial state `{}` is not declared", prefix(), protocol.initial.name), protocol.initial.span));
    }
    if protocol.deadline_secs == 0 {
        errors.push((format!("{} `deadline` must be greater than zero", prefix()), protocol.span));
    }
    if !terminal.contains(&protocol.deadline_target.name) {
        errors.push((format!("{} `deadline_target` `{}` must name a terminal state", prefix(), protocol.deadline_target.name), protocol.deadline_target.span));
    }
    if protocol.poll.method != HttpMethod::Get {
        errors.push((format!("{} polling must use `GET` so observation cannot mutate provider state", prefix()), protocol.poll.span));
    }
    if !protocol.poll.path.starts_with('/') {
        errors.push((format!("{} poll path must start with `/`, got `{}`", prefix(), protocol.poll.path), protocol.poll.span));
    }
    if matches!(protocol.interval, ProtocolPollInterval::FixedSeconds(0)) {
        errors.push((format!("{} fixed poll interval must be greater than zero", prefix()), protocol.span));
    }
    if operation.method != HttpMethod::Get && operation.effect != Effect::Dangerous {
        errors.push((
            format!("{} submits with `{}` and must be declared `dangerous` so approval is enforced before provider I/O", prefix(), operation.method.as_str()),
            operation.span,
        ));
    }

    for state in &protocol.states {
        let mut handled = HashSet::new();
        for transition in &state.transitions {
            if !statuses.contains(&transition.status.name) {
                errors.push((format!("{} state `{}` handles undeclared provider status `{}`", prefix(), state.name.name, transition.status.name), transition.status.span));
            }
            if !handled.insert(transition.status.name.clone()) {
                errors.push((format!("{} state `{}` handles status `{}` more than once", prefix(), state.name.name, transition.status.name), transition.span));
            }
            if !declared.contains(&transition.target.name) {
                errors.push((format!("{} state `{}` transitions to undeclared state `{}`", prefix(), state.name.name, transition.target.name), transition.target.span));
            }
        }
        let missing: Vec<_> = protocol.statuses.iter()
            .filter(|status| !handled.contains(&status.name))
            .map(|status| status.name.as_str()).collect();
        if !missing.is_empty() {
            errors.push((format!("{} state `{}` is not exhaustive; missing status{} {}", prefix(), state.name.name, if missing.len() == 1 { "" } else { "es" }, missing.join(", ")), state.span));
        }
    }

    if declared.contains(&protocol.initial.name) {
        let transitions: std::collections::HashMap<&str, Vec<&str>> = protocol.states.iter()
            .map(|state| (state.name.name.as_str(), state.transitions.iter().map(|t| t.target.name.as_str()).collect()))
            .collect();
        let mut reachable = HashSet::new();
        let mut queue = std::collections::VecDeque::from([protocol.initial.name.as_str()]);
        while let Some(state) = queue.pop_front() {
            if !reachable.insert(state) { continue; }
            if let Some(targets) = transitions.get(state) {
                queue.extend(targets.iter().copied());
            }
        }
        for state in &declared {
            if !reachable.contains(state.as_str()) {
                errors.push((format!("{} state `{state}` is unreachable from initial state `{}`", prefix(), protocol.initial.name), protocol.span));
            }
        }
        if terminal.iter().all(|state| !reachable.contains(state.as_str())) {
            errors.push((format!("{} has no terminal state reachable from `{}`", prefix(), protocol.initial.name), protocol.span));
        }
    }
    errors
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
        Stmt::Atomic { body, .. } => {
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

/// The longest timeout a connector or operation may declare: one hour.
/// A longer wait is durable work (a job or a provider protocol), not one
/// request.
pub(crate) const MAX_CONNECTOR_TIMEOUT_SECS: u64 = 3600;

/// Why a declared connector or operation timeout is unusable, if it is.
fn connector_timeout_problem(subject: &str, secs: u64) -> Option<String> {
    if secs == 0 {
        return Some(format!(
            "{subject} declares `timeout: 0s`, which would fail every request; give a positive number of seconds"
        ));
    }
    if secs > MAX_CONNECTOR_TIMEOUT_SECS {
        return Some(format!(
            "{subject} declares `timeout: {secs}s`, longer than the {MAX_CONNECTOR_TIMEOUT_SECS}s limit for one request; work that takes longer belongs in a durable job or an `async:` provider protocol"
        ));
    }
    None
}

/// Headers an operation cannot set with `header`: the credential headers
/// (`auth:` owns `Authorization`, and a cookie is a credential), and the
/// ones the transport writes from the request itself.
const CONNECTOR_MANAGED_HEADERS: [(&str, &str); 7] = [
    ("authorization", "the connector's `auth:` sets it"),
    ("proxy-authorization", "it carries a credential"),
    ("cookie", "it carries a credential"),
    ("host", "the request's URL sets it"),
    ("content-type", "the operation's `body` or `form` sets it"),
    ("content-length", "the transport sets it from the body"),
    ("transfer-encoding", "the transport sets it"),
];

/// A multipart text part's value: one scalar written as text. A query
/// field is checked through `text_parameters` instead; a multipart body
/// is its own form, where `Bytes` is a file part, not text.
fn is_multipart_scalar(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Int
            | Type::Float
            | Type::Bool
            | Type::String
            | Type::Decimal
            | Type::Duration
            | Type::Instant
            | Type::ZonedTime
            | Type::TimeZone
            | Type::Currency
            | Type::Locale
    )
}

/// Whether `name` is an HTTP header token (RFC 9110 `tchar`s).
fn is_header_token(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

impl<'a> Checker<'a> {
    /// `query <param>` names a record of scalar, optional or list fields,
    /// and each `header` line names a header the operation may set, once,
    /// with a `String` parameter or a fixed value.
    fn check_operation_query_and_headers(
        &mut self,
        connector: &ConnectorDecl,
        op: &corvid_ast::OperationDecl,
        param_names: &HashSet<&str>,
    ) {
        let invalid = |message: String, span| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid {
                    connector: connector.name.name.clone(),
                    message,
                },
                span,
            )
        };
        let operation = &op.name.name;
        let param_type = |checker: &mut Self, name: &str| {
            op.params
                .iter()
                .find(|p| p.name.name == name)
                .map(|p| checker.type_ref_to_type(&p.ty))
        };

        if let Some(query) = &op.query {
            let used_elsewhere = op.body.as_ref().is_some_and(|b| b.param.name == query.name)
                || path_placeholders(&op.path).iter().any(|p| p == &query.name);
            if !param_names.contains(query.name.as_str()) {
                self.errors.push(invalid(
                    format!("operation `{operation}` query references `{}`, which is not a declared parameter", query.name),
                    query.span,
                ));
            } else if used_elsewhere {
                self.errors.push(invalid(
                    format!("operation `{operation}` sends `{}` both as its query and in its path or body; a parameter goes in one place", query.name),
                    query.span,
                ));
            } else {
                match param_type(self, &query.name) {
                    Some(Type::Struct(id)) if self.types_by_id.get(&id).is_some_and(|d| d.variants.is_empty()) => {
                        let fields: Vec<(String, corvid_ast::TypeRef)> = self.types_by_id[&id]
                            .fields
                            .iter()
                            .map(|f| (f.name.name.clone(), f.ty.clone()))
                            .collect();
                        for (field, field_ty) in fields {
                            let ty = self.type_ref_to_type(&field_ty);
                            // The shape a route reads its query through, so
                            // a value goes out as a route would read it in.
                            let plain = |ty: &Type| self.has_plain_variants(ty);
                            let ok = crate::text_parameters::query_field_shape(&ty, &plain).is_some();
                            if !ok {
                                self.errors.push(invalid(
                                    format!(
                                        "operation `{operation}` query field `{field}` is `{}`; a query field is one of {}, an `Option` of one (omitted when `None`) or a `List` of one (the name repeated)",
                                        self.json_type_name(&ty),
                                        crate::text_parameters::TEXT_TYPES
                                    ),
                                    field_ty.span(),
                                ));
                            }
                        }
                    }
                    _ => self.errors.push(invalid(
                        format!("operation `{operation}` query `{}` must be a record whose fields become the query parameters", query.name),
                        query.span,
                    )),
                }
            }
        }

        let auth_header = match &connector.auth {
            Some(ConnectorAuth::Header { name, .. }) => Some(name.to_ascii_lowercase()),
            _ => None,
        };
        let idempotency_header = op.protocol.as_ref().map(|p| match &p.idempotency.transport {
            corvid_ast::ProtocolIdempotencyTransport::Header(name) => name.to_ascii_lowercase(),
        });
        let mut seen = HashSet::new();
        for header in &op.headers {
            let lower = header.name.to_ascii_lowercase();
            if !is_header_token(&header.name) {
                self.errors.push(invalid(
                    format!("operation `{operation}` header name `{}` is not a valid HTTP header name (letters, digits and `-`, no spaces or `:`)", header.name),
                    header.span,
                ));
                continue;
            }
            if let Some((_, why)) = CONNECTOR_MANAGED_HEADERS.iter().find(|(name, _)| *name == lower) {
                self.errors.push(invalid(
                    format!("operation `{operation}` cannot set `{}` with `header`: {why}", header.name),
                    header.span,
                ));
                continue;
            }
            if auth_header.as_deref() == Some(lower.as_str()) {
                self.errors.push(invalid(
                    format!("operation `{operation}` cannot set `{}` with `header`: the connector's `auth:` sends its credential in it", header.name),
                    header.span,
                ));
                continue;
            }
            if idempotency_header.as_deref() == Some(lower.as_str()) {
                self.errors.push(invalid(
                    format!("operation `{operation}` cannot set `{}` with `header`: its `async:` protocol sends the intent key in it", header.name),
                    header.span,
                ));
                continue;
            }
            if !seen.insert(lower) {
                self.errors.push(invalid(
                    format!("operation `{operation}` sets header `{}` twice", header.name),
                    header.span,
                ));
                continue;
            }
            match &header.value {
                corvid_ast::OperationHeaderValue::Literal(text) => {
                    if text.is_empty() || text.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
                        self.errors.push(invalid(
                            format!("operation `{operation}` header `{}` needs a non-empty value on one line", header.name),
                            header.span,
                        ));
                    }
                }
                corvid_ast::OperationHeaderValue::Param(param) => {
                    match param_type(self, &param.name) {
                        None => self.errors.push(invalid(
                            format!("operation `{operation}` header `{}` references `{}`, which is not a declared parameter", header.name, param.name),
                            param.span,
                        )),
                        Some(Type::String) => {}
                        Some(other) => self.errors.push(invalid(
                            format!(
                                "operation `{operation}` header `{}` takes `{}`, which is `{}`; a header's value is a `String` parameter or a fixed string",
                                header.name, param.name, self.json_type_name(&other)
                            ),
                            param.span,
                        )),
                    }
                }
            }
        }
    }
}

/// What an operation's `mock:` must produce: its return type, with a
/// `Response<Body>` replaced by its body (inside a `Result` too) and a
/// `Stream<T>` by the `List<T>` of events mock mode streams.
fn mock_payload_type(ret: Type) -> Type {
    match ret {
        Type::Response(body) => *body,
        // A streamed operation's mock is its events, in order.
        Type::Stream(event) => Type::List(event),
        Type::Result(ok, err) => match *ok {
            Type::Response(body) => Type::Result(body, err),
            ok => Type::Result(Box::new(ok), err),
        },
        other => other,
    }
}

impl<'a> Checker<'a> {
    /// A `Response` is an operation's whole result or its `Result`'s
    /// success type, and its body holds no `Response`. A `HEAD` answer
    /// has no body, so a `HEAD` operation returns `Response<Nothing>`.
    fn check_operation_response_shape(&mut self, connector: &ConnectorDecl, op: &corvid_ast::OperationDecl) {
        let invalid = |message: String| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid { connector: connector.name.name.clone(), message },
                op.return_ty.span(),
            )
        };
        let operation = &op.name.name;
        let ret = self.type_ref_to_type(&op.return_ty);
        let (response_body, rest_mentions) = match &ret {
            Type::Response(body) => (Some(body.as_ref()), body.mentions_response()),
            Type::Result(ok, err) => match ok.as_ref() {
                Type::Response(body) => (Some(body.as_ref()), body.mentions_response() || err.mentions_response()),
                ok => (None, ok.mentions_response() || err.mentions_response()),
            },
            other => (None, other.mentions_response()),
        };
        if rest_mentions {
            self.errors.push(invalid(format!(
                "operation `{operation}` returns `{}`; a `Response` is the operation's whole result or its `Result`'s success type, never inside another type",
                ret.display_name()
            )));
        }
        if matches!(op.method, corvid_ast::HttpMethod::Head) && !matches!(response_body, Some(Type::Nothing)) {
            self.errors.push(invalid(format!(
                "operation `{operation}` is a HEAD request, whose answer has no body, so it returns `Response<Nothing>` (or a `Result` of one), not `{}`",
                ret.display_name()
            )));
        }
    }
}

impl<'a> Checker<'a> {
    /// A `base_url` connector's operations name paths; a `hosts:
    /// allowed_hosts` connector's operations name a `String` URL
    /// parameter, carry no `async:` protocol (its poll path is relative to
    /// one base URL), and declare `response: text` only for a `String`
    /// body.
    fn check_operation_target(&mut self, connector: &ConnectorDecl, op: &corvid_ast::OperationDecl) {
        let invalid = |message: String, span| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid { connector: connector.name.name.clone(), message },
                span,
            )
        };
        let operation = &op.name.name;
        match (&connector.base_url, &op.url_param) {
            (Some(_), Some(url)) => self.errors.push(invalid(
                format!(
                    "operation `{operation}` names its URL with `{}`, but the connector has a `base_url`; give a path such as `\"/v1/items\"`, or declare `hosts: allowed_hosts` instead of `base_url`",
                    url.name
                ),
                url.span,
            )),
            (None, None) => self.errors.push(invalid(
                format!(
                    "operation `{operation}` gives a path, but the connector declares `hosts: allowed_hosts`, so its operations take the whole URL as a parameter: `GET url`"
                ),
                op.span,
            )),
            (None, Some(url)) => {
                match op.params.iter().find(|p| p.name.name == url.name) {
                    None => self.errors.push(invalid(
                        format!("operation `{operation}` takes its URL from `{}`, which is not a declared parameter", url.name),
                        url.span,
                    )),
                    Some(param) => {
                        let ty = self.type_ref_to_type(&param.ty);
                        if ty != Type::String {
                            self.errors.push(invalid(
                                format!(
                                    "operation `{operation}` takes its URL from `{}`, which is `{}`; a URL is a `String`",
                                    url.name,
                                    self.json_type_name(&ty)
                                ),
                                url.span,
                            ));
                        }
                    }
                }
                if op.protocol.is_some() {
                    self.errors.push(invalid(
                        format!(
                            "operation `{operation}` declares an `async:` protocol, whose poll path is relative to one base URL; a `hosts: allowed_hosts` connector has none"
                        ),
                        op.span,
                    ));
                }
            }
            (Some(_), None) => {}
        }
        if op.response_text {
            let body = match self.type_ref_to_type(&op.return_ty) {
                Type::Response(body) => *body,
                Type::Result(ok, _) => match *ok {
                    Type::Response(body) => *body,
                    ok => ok,
                },
                other => other,
            };
            if body != Type::String {
                self.errors.push(invalid(
                    format!(
                        "operation `{operation}` reads its response as text, so its body is a `String`, not `{}`",
                        self.json_type_name(&body)
                    ),
                    op.return_ty.span(),
                ));
            }
        }
    }
}

impl<'a> Checker<'a> {
    /// `on failure -> Variant` turns every failure into a typed error, so
    /// the operation returns a `Result` whose error type declares the
    /// variant, with no fields or one `String` field for the reason.
    fn check_operation_on_failure(&mut self, connector: &ConnectorDecl, op: &corvid_ast::OperationDecl) {
        let Some(variant) = &op.on_failure else { return };
        let invalid = |message: String| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid { connector: connector.name.name.clone(), message },
                variant.span,
            )
        };
        let operation = &op.name.name;
        // A protocol's submit and polls fail through its own lifecycle
        // (deadline target, circuit breaker), not a single request, so
        // `on failure` would map only some of its failures. Refused
        // rather than applied partly.
        if op.protocol.is_some() {
            self.errors.push(invalid(format!(
                "operation `{operation}` has an `async:` protocol, which handles its failures through its deadline target and circuit breaker; `on failure` maps one request's failures and cannot apply to it"
            )));
            return;
        }
        // A streamed operation maps failures into its events: each event
        // is `Ok`, and an outage is one final `Err` (decision 7).
        let returned = self.type_ref_to_type(&op.return_ty);
        let (streamed, mapped) = match (&op.streams, returned) {
            (Some(_), Type::Stream(item)) => (true, *item),
            (_, returned) => (false, returned),
        };
        let Type::Result(_, err) = mapped else {
            self.errors.push(invalid(if streamed {
                format!(
                    "operation `{operation}` streams and maps failures to `{}`, so each event is a `Result` whose error type declares that variant: `Stream<Result<Event, Error>>`",
                    variant.name
                )
            } else {
                format!(
                    "operation `{operation}` maps failures to `{}`, so it returns a `Result` whose error type declares that variant",
                    variant.name
                )
            }));
            return;
        };
        let decl = match err.as_ref() {
            Type::Struct(id) => self.types_by_id.get(id).copied(),
            _ => None,
        };
        let Some(found) = decl.and_then(|d| d.variants.iter().find(|v| v.name.name == variant.name)) else {
            self.errors.push(invalid(format!(
                "operation `{operation}` maps failures to `{}`, which is not a variant of its error type `{}`",
                variant.name,
                self.json_type_name(&err)
            )));
            return;
        };
        let fields: Vec<Type> = found.fields.iter().map(|f| f.ty.clone()).collect::<Vec<_>>()
            .iter().map(|ty| self.type_ref_to_type(ty)).collect();
        if !(fields.is_empty() || fields == [Type::String]) {
            self.errors.push(invalid(format!(
                "`{}` receives failures, so it has no fields or one `String` field for the reason",
                variant.name
            )));
        }
    }
}

impl<'a> Checker<'a> {
    /// A streamed operation returns `Stream<T>` as its whole result and
    /// declares how events are framed; nothing else declares `streams:`.
    /// Its failures end the stream with an error, or with a final `Err`
    /// event when it declares `on failure` over `Stream<Result<T, E>>`;
    /// it cannot also read text or run an `async:` protocol.
    fn check_operation_stream(&mut self, connector: &ConnectorDecl, op: &corvid_ast::OperationDecl) {
        let invalid = |message: String| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid { connector: connector.name.name.clone(), message },
                op.span,
            )
        };
        let operation = &op.name.name;
        let ret = self.type_ref_to_type(&op.return_ty);
        let streams_whole = matches!(ret, Type::Stream(_));
        let nested = match &ret {
            Type::Stream(inner) => mentions_stream(inner),
            other => mentions_stream(other),
        };
        if nested {
            self.errors.push(invalid(format!(
                "operation `{operation}` returns `{}`; a `Stream` is an operation's whole result, never inside another type",
                self.json_type_name(&ret)
            )));
        }
        match (streams_whole, op.streams) {
            (true, None) => self.errors.push(invalid(format!(
                "operation `{operation}` returns a `Stream`, so it declares how events arrive: `streams: server_sent_events` or `streams: json_lines`"
            ))),
            (false, Some(_)) => self.errors.push(invalid(format!(
                "operation `{operation}` declares `streams:` but returns `{}`, not a `Stream`",
                self.json_type_name(&ret)
            ))),
            _ => {}
        }
        if op.streams.is_some() {
            if op.stream_until.as_deref() == Some("") {
                self.errors.push(invalid(format!("operation `{operation}` ends its stream `until` an empty event, which every keep-alive would match")));
            }
            if op.stream_until.is_some() && op.streams == Some(corvid_ast::StreamFormat::JsonLines) {
                self.errors.push(invalid(format!(
                    "operation `{operation}` declares `until` on `json_lines`; an end marker belongs to `server_sent_events`, whose events can carry one"
                )));
            }
            let conflicts = [
                (op.response_text, "`response: text`"),
                (op.protocol.is_some(), "an `async:` protocol"),
            ];
            for (present, what) in conflicts {
                if present {
                    self.errors.push(invalid(format!("operation `{operation}` streams its response, so it cannot also declare {what}")));
                }
            }
        }
    }
}

fn mentions_stream(ty: &Type) -> bool {
    match ty {
        Type::Stream(_) => true,
        Type::List(inner) | Type::Option(inner) | Type::Response(inner) => mentions_stream(inner),
        Type::Result(a, b) | Type::Map(a, b) => mentions_stream(a) || mentions_stream(b),
        _ => false,
    }
}

/// A multipart field's value: a scalar part, or an `Upload` or `Bytes`
/// file part.
fn is_multipart_value(ty: &Type) -> bool {
    is_multipart_scalar(ty) || matches!(ty, Type::Upload(_) | Type::Bytes)
}

impl<'a> Checker<'a> {
    /// What an operation sends and reads has a wire form: a `body` is a
    /// `Bytes` or `Upload` sent as itself, or a value with a JSON form;
    /// a `form` carries no files; a `multipart` body is a record of flat
    /// fields, files included; a response is read through JSON unless it
    /// is `response: text`.
    fn check_operation_body(&mut self, connector: &ConnectorDecl, op: &corvid_ast::OperationDecl) {
        let invalid = |message: String| {
            TypeError::new(
                TypeErrorKind::ConnectorConfigInvalid { connector: connector.name.name.clone(), message },
                op.span,
            )
        };
        let operation = &op.name.name;
        if let Some(body) = &op.body {
            let Some(param) = op.params.iter().find(|p| p.name.name == body.param.name) else { return };
            let ty = self.type_ref_to_type(&param.ty);
            let file = matches!(ty, Type::Upload(_) | Type::Bytes);
            match body.encoding {
                corvid_ast::BodyEncoding::Json if file => {}
                corvid_ast::BodyEncoding::Json => {
                    if let Some(problem) = self.json_wire_problem(&ty, true) {
                        self.errors.push(invalid(format!(
                            "operation `{operation}` sends `{}` as its JSON body, but {problem}",
                            body.param.name
                        )));
                    }
                }
                corvid_ast::BodyEncoding::Form if file => self.errors.push(invalid(format!(
                    "operation `{operation}` sends `{}` as a form, which carries text fields only; send a file as `body {}` or inside a `multipart` record",
                    body.param.name, body.param.name
                ))),
                corvid_ast::BodyEncoding::Form => {}
                corvid_ast::BodyEncoding::Multipart => match ty {
                    Type::Struct(id) if self.types_by_id.get(&id).is_some_and(|d| d.variants.is_empty()) => {
                        let fields: Vec<(String, corvid_ast::TypeRef)> = self.types_by_id[&id]
                            .fields
                            .iter()
                            .map(|f| (f.name.name.clone(), f.ty.clone()))
                            .collect();
                        for (field, field_ty) in fields {
                            let ty = self.type_ref_to_type(&field_ty);
                            let ok = match &ty {
                                Type::Option(inner) | Type::List(inner) => is_multipart_value(inner),
                                other => is_multipart_value(other),
                            };
                            if !ok {
                                self.errors.push(invalid(format!(
                                    "operation `{operation}` multipart field `{field}` is `{}`; a part is a number, text, `Bool`, a decimal or time value, an `Upload` or `Bytes`, or an `Option` or `List` of one",
                                    self.json_type_name(&ty)
                                )));
                            }
                        }
                    }
                    other => self.errors.push(invalid(format!(
                        "operation `{operation}` sends `{}` as multipart, so it is a record whose fields become the parts, not `{}`",
                        body.param.name,
                        self.json_type_name(&other)
                    ))),
                },
            }
        }
        if !op.response_text {
            let read = match self.type_ref_to_type(&op.return_ty) {
                Type::Result(ok, _) => *ok,
                other => other,
            };
            let read = match read {
                Type::Response(body) | Type::Stream(body) => *body,
                other => other,
            };
            if let Some(problem) = self.json_wire_problem(&read, false) {
                self.errors.push(invalid(format!(
                    "operation `{operation}` reads its response as `{}`, but {problem}",
                    self.json_type_name(&read)
                )));
            }
        }
    }
}
