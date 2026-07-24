//! Contract Closure (slice 52b) — the Phase 52 invariant made
//! mechanical: *the running backend proves it implements its own
//! contract, or it refuses to start.*
//!
//! Before `corvid serve` / `corvid dev` bind a listener, they walk the
//! public HTTP surface (the `server` routes the Application Contract
//! advertises) and assert a runtime execution path exists for every
//! element. A route the contract describes but the runtime cannot yet
//! execute — a `Stream<T>` response with no SSE, an `Upload<Format>`
//! body with no multipart parser, a `Page<Item>` response with no
//! cursor envelope, a `requires`-policy route with no authorization
//! enforcement — is a **startup error (E5204)**, naming the offending
//! element and the capability it needs. It is never a silent runtime
//! `501`: the developer's source is the forcing function. Writing
//! `route ... -> json Stream<Event>` forces the runtime to have SSE or
//! refuse to start.
//!
//! The check reads a [`RuntimeCapabilities`] snapshot describing what
//! the interpreter tier can execute as of the current slice. Each
//! Phase 52 slice that lands a capability flips one field to `true`,
//! so the closure surface grows in lockstep with the runtime and can
//! never advertise more than it delivers.

use corvid_ir::IrFile;
use corvid_types::Type;

/// The guarantee-registry id anchored at this enforcement site
/// (RuntimeChecked). The startup closure check below is the runtime
/// path that proves it; the row lives in
/// `crates/corvid-guarantees/src/registry.rs`.
pub const GUARANTEE_ID_CONTRACT_RUNTIME_CLOSURE: &str = "contract.runtime_closure";

/// One public contract element whose runtime execution path does not
/// yet exist. Named precisely enough that the developer knows which
/// route to look at and which capability is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureGap {
    /// The offending element, e.g. `route GET /orders/{id}`.
    pub element: String,
    /// The runtime capability it needs but the runtime does not yet
    /// provide, e.g. `streaming responses (Server-Sent Events)`.
    pub missing_capability: String,
    /// The slice that lands the capability, e.g. `52c`.
    pub provided_by: &'static str,
}

impl ClosureGap {
    /// The single-line `E5204` startup-refusal message for this gap.
    pub fn message(&self) -> String {
        format!(
            "E5204 Contract not executable: {} needs {} — a runtime path for it \
             does not exist yet (arrives in slice {}). The backend refuses to \
             start rather than advertise a surface it cannot serve.",
            self.element, self.missing_capability, self.provided_by
        )
    }
}

/// What the interpreter tier can execute as of the current slice. Each
/// Phase 52 slice that lands a capability flips its field to `true`.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeCapabilities {
    /// Route bodies execute — path params, query structs, typed JSON
    /// bodies, scalar/struct returns (slice 52a).
    pub route_execution: bool,
    /// SSE streaming for `Stream<T>` responses (slice 52c).
    pub streaming: bool,
    /// Multipart parsing for `Upload<Format>` request bodies (52c).
    pub uploads: bool,
    /// Cursor-paginated `{items, next_cursor, has_more}` envelopes for
    /// `Page<Item>` responses (slice 52c).
    pub pagination: bool,
    /// Authenticated-actor derivation and `requires`-policy
    /// enforcement (slices 52g/52h). Until present, a policy route
    /// executes with a stub actor and does NOT enforce its
    /// requirement, so the contract's auth promise is unmet.
    pub auth_enforcement: bool,
}

impl RuntimeCapabilities {
    /// The interpreter tier as it stands after slice 52a: route bodies
    /// execute; streaming, uploads, pagination, and auth enforcement
    /// are not yet wired.
    pub fn interpreter_tier() -> Self {
        Self {
            route_execution: true,
            // SSE streaming for `Stream<T>` responses executes end-to-end
            // (slice 52c): serve consumes the interpreter's stream channel
            // and flushes each chunk as a `data:` event with an `event:
            // done` terminator.
            streaming: true,
            // `Upload<Format>` request bodies are parsed from multipart
            // (accepted-MIME + max-size enforced) and read via
            // `body.text()`/`bytes()`/… methods (slice 52c-2).
            uploads: true,
            // `Page<Item>` responses build a `{items, next_cursor,
            // has_more}` envelope via the `Page(items, next_cursor)`
            // constructor (slice 52c-2).
            pagination: true,
            // A `requires authenticated|role|permission` route is enforced
            // before its handler runs (slice 52f): the session is resolved
            // to a verified typed `actor`, tenant + role + permission are
            // checked, and cookie-authenticated mutations require CSRF
            // double-submit. This was the last Contract Closure gap.
            auth_enforcement: true,
        }
    }
}

/// Walk the public route surface and return every element whose
/// runtime execution path does not exist under `caps`. An empty vec
/// means the contract is closed: every advertised element is
/// executable, and the backend may start.
pub fn check_contract_closure(ir: &IrFile, caps: RuntimeCapabilities) -> Vec<ClosureGap> {
    let mut gaps = Vec::new();
    for server in &ir.servers {
        for route in &server.routes {
            let element = format!("route {} {}", route.method.as_str(), route.path);

            // Response boundary types.
            if !caps.streaming && type_mentions_stream(&route.response_ty) {
                gaps.push(ClosureGap {
                    element: element.clone(),
                    missing_capability: "streaming responses (Server-Sent Events)".to_string(),
                    provided_by: "52c-1",
                });
            }
            if !caps.pagination && matches!(route.response_ty, Type::Page(_)) {
                gaps.push(ClosureGap {
                    element: element.clone(),
                    missing_capability: "cursor-paginated responses (Page<Item> envelope)"
                        .to_string(),
                    provided_by: "52c-2",
                });
            }

            // Request body boundary type.
            if !caps.uploads
                && route
                    .body_ty
                    .as_ref()
                    .is_some_and(|t| matches!(t, Type::Upload(_)))
            {
                gaps.push(ClosureGap {
                    element: element.clone(),
                    missing_capability: "file uploads (multipart Upload<Format> parsing)"
                        .to_string(),
                    provided_by: "52c-2",
                });
            }

            // Auth policy: a route requires authentication when its
            // synthetic handler agent (slice 52a) binds an `actor`
            // parameter. Executing it without enforcing the policy
            // would let the contract advertise an auth requirement the
            // runtime never checks.
            if !caps.auth_enforcement && route_requires_auth(ir, &route.handler_agent) {
                gaps.push(ClosureGap {
                    element: element.clone(),
                    missing_capability:
                        "authorization enforcement (authenticated actor + policy check)"
                            .to_string(),
                    provided_by: "52f",
                });
            }
        }
    }
    gaps
}

/// A route requires authentication iff its synthetic handler agent has
/// an `actor` parameter — the resolver binds one only for routes that
/// carry an auth policy (slice 52a `build_route_handler_agent`).
fn route_requires_auth(ir: &IrFile, handler_agent: &str) -> bool {
    ir.agents
        .iter()
        .find(|a| a.name == handler_agent)
        .is_some_and(|a| a.params.iter().any(|p| p.name == "actor"))
}

/// `Stream<T>` in the response position, at the top level or nested
/// (e.g. a wrapper carrying a stream field is still not serveable
/// without SSE). Conservative: any structural mention counts.
fn type_mentions_stream(ty: &Type) -> bool {
    match ty {
        Type::Stream(_) => true,
        Type::Page(inner) | Type::Upload(inner) | Type::List(inner) | Type::Option(inner) => {
            type_mentions_stream(inner)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::{HttpMethod, RouteResponseKind, Span};
    use corvid_ir::{IrAgent, IrBlock, IrParam, IrRoute, IrServer};
    use corvid_resolve::{DefId, LocalId};

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn empty_block() -> IrBlock {
        IrBlock {
            stmts: vec![],
            span: span(),
        }
    }

    fn param(name: &str, local: u32) -> IrParam {
        IrParam {
            name: name.to_string(),
            local_id: LocalId(local),
            ty: Type::RouteParams(vec![]),
            span: span(),
        }
    }

    /// A minimal handler agent for a route. `with_actor` mirrors the
    /// 52a synthetic agent shape for a policy route.
    fn handler_agent(name: &str, with_actor: bool) -> IrAgent {
        let mut params = vec![param("path", 0)];
        if with_actor {
            params.push(param("actor", 1));
        }
        IrAgent {
            id: DefId(1000),
            name: name.to_string(),
            extern_abi: None,
            params,
            return_ty: Type::String,
            cost_budget: None,
            wrapping_arithmetic: false,
            is_replayable: false,
            pure_fn: false,
            retry_max_attempts: None,
            retry_backoff_ms: None,
            idempotency_key_param: None,
            body: empty_block(),
            span: span(),
            borrow_sig: None,
        }
    }

    fn route(response_ty: Type, body_ty: Option<Type>, handler: &str) -> IrRoute {
        IrRoute {
            method: HttpMethod::Get,
            path: "/x".to_string(),
            path_params: vec![],
            query_ty: None,
            body_ty,
            response_kind: RouteResponseKind::Json,
            response_ty,
            effect_names: vec![],
            body: empty_block(),
            handler_agent: handler.to_string(),
            upload_policy: None,
            approval_policy: None,
            upload_format: None,
            policy: None,
            span: span(),
        }
    }

    fn ir_with(route: IrRoute, agents: Vec<IrAgent>) -> IrFile {
        IrFile {
            imports: vec![],
            types: vec![],
            tools: vec![],
            prompts: vec![],
            agents,
            evals: vec![],
            tests: vec![],
            fixtures: vec![],
            mocks: vec![],
            servers: vec![IrServer {
                id: DefId(1),
                name: "api".to_string(),
                routes: vec![route],
                span: span(),
            }],
            models: vec![],
            connectors: vec![],
        }
    }

    /// Positive: a plain route (scalar/struct response, typed body, no
    /// policy) is fully executable under the 52a interpreter tier, so
    /// the contract is closed and the backend may start.
    #[test]
    fn reference_shape_has_no_closure_gaps() {
        let r = route(Type::String, Some(Type::Struct(DefId(7))), "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let gaps = check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier());
        assert!(gaps.is_empty(), "plain route should have no gaps: {gaps:?}");
    }

    /// The streaming detection path: with the `streaming` capability
    /// OFF, a `Stream<T>` response route is a gap (E5204). Slice 52c-1
    /// shipped the SSE runtime and turned the capability on in
    /// `interpreter_tier()`, so this asserts the detection code against
    /// an explicit streaming-off snapshot — the code path that guarded
    /// stream routes before 52c-1 and still guards native tiers that
    /// lack SSE.
    #[test]
    fn stream_response_route_is_a_gap_when_streaming_is_off() {
        let r = route(Type::Stream(Box::new(Type::String)), None, "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let caps = RuntimeCapabilities {
            streaming: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].missing_capability.contains("streaming"));
        assert!(gaps[0].message().contains("E5204"));
    }

    /// Positive: with `interpreter_tier()` (streaming ON as of 52c-1), a
    /// `Stream<T>` response route is NOT a gap — it starts and serves.
    #[test]
    fn stream_response_route_starts_under_interpreter_tier() {
        let r = route(Type::Stream(Box::new(Type::String)), None, "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let gaps = check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier());
        assert!(gaps.is_empty(), "streaming route should serve: {gaps:?}");
    }

    /// The upload detection path: with the `uploads` capability OFF, an
    /// `Upload<Format>` body route is a gap. Slice 52c-2 shipped the
    /// multipart runtime and turned the capability on in
    /// `interpreter_tier()`, so this asserts detection against an
    /// explicit uploads-off snapshot (the code path that guards native
    /// tiers lacking multipart).
    #[test]
    fn upload_body_route_is_a_closure_gap() {
        let r = route(Type::String, Some(Type::Upload(Box::new(Type::String))), "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let caps = RuntimeCapabilities {
            uploads: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].missing_capability.contains("upload"));
    }

    /// The pagination detection path: with `pagination` OFF, a
    /// `Page<Item>` response route is a gap. Slice 52c-2 turned the
    /// capability on in `interpreter_tier()`.
    #[test]
    fn page_response_route_is_a_closure_gap() {
        let r = route(Type::Page(Box::new(Type::Struct(DefId(7)))), None, "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let caps = RuntimeCapabilities {
            pagination: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].missing_capability.contains("paginated"));
    }

    /// Positive: with `interpreter_tier()` (uploads + pagination ON as
    /// of 52c-2), `Upload<Format>` and `Page<Item>` routes are NOT gaps
    /// — they start and serve.
    #[test]
    fn upload_and_page_routes_start_under_interpreter_tier() {
        let upload = route(Type::String, Some(Type::Upload(Box::new(Type::String))), "h");
        let ir = ir_with(upload, vec![handler_agent("h", false)]);
        assert!(check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier()).is_empty());

        let page = route(Type::Page(Box::new(Type::Struct(DefId(7)))), None, "h2");
        let ir = ir_with(page, vec![handler_agent("h2", false)]);
        assert!(check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier()).is_empty());
    }

    /// Adversarial: a `requires`-policy route (handler binds `actor`)
    /// refuses to start until authorization is enforced — advertising
    /// an auth requirement the runtime doesn't check is exactly the
    /// contract/runtime gap the invariant forbids.
    #[test]
    fn policy_route_without_auth_enforcement_is_a_closure_gap() {
        let r = route(Type::String, None, "h");
        let ir = ir_with(r, vec![handler_agent("h", true)]);
        // With authorization enforcement OFF, a `requires`-policy route is a
        // gap (the historical pre-52f state).
        let caps = RuntimeCapabilities {
            auth_enforcement: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].missing_capability.contains("authorization"));
        assert_eq!(gaps[0].provided_by, "52f");
        // As of 52f the interpreter tier enforces it, so the gap closes.
        assert!(check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier()).is_empty());
    }

    /// Once a capability lands (here: streaming), the same route is no
    /// longer a gap — the closure surface grows with the runtime.
    #[test]
    fn capability_present_closes_the_gap() {
        let r = route(Type::Stream(Box::new(Type::String)), None, "h");
        let ir = ir_with(r, vec![handler_agent("h", false)]);
        let caps = RuntimeCapabilities {
            streaming: true,
            ..RuntimeCapabilities::interpreter_tier()
        };
        assert!(check_contract_closure(&ir, caps).is_empty());
    }

    /// End-to-end guard: an `Upload<Format>` body route, compiled through
    /// the REAL pipeline (source → resolve → check → lower), is detected
    /// as an upload gap when the capability is OFF. This pins the
    /// `type_ref_to_type` lowering of `Upload`/`Page` — without it the
    /// IR's `body_ty` is `Type::Unknown` and closure silently passes a
    /// route the runtime can't serve (the gap the hand-built unit tests
    /// could not catch because they construct `Type::Upload` directly,
    /// bypassing lowering).
    #[test]
    fn compiled_upload_route_is_detected_as_a_closure_gap() {
        let source = r#"type R:
    ok: Bool

agent take(body: Upload<Csv>) -> R:
    return R(true)

server a:
    @upload(max_mb: 1)
    route POST "/i" body Upload<Csv> -> json R:
        return take(body)
"#;
        let ir = crate::compile_to_ir(source).expect("source compiles");
        let caps = RuntimeCapabilities {
            uploads: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1, "expected one upload gap: {gaps:?}");
        assert!(gaps[0].missing_capability.contains("upload"));
        // And under the full interpreter tier (52c-2), it serves.
        assert!(
            check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier()).is_empty(),
            "upload route should serve under 52c-2"
        );
    }

    /// End-to-end companion: a `Page<Item>` response route compiled
    /// through the real pipeline is detected as a pagination gap when
    /// the capability is off, and serves under the full tier.
    #[test]
    fn compiled_page_route_is_detected_as_a_closure_gap() {
        let source = r#"type Item:
    id: String

agent list_items() -> Page<Item>:
    return list_items()

server a:
    route GET "/items" -> json Page<Item>:
        return list_items()
"#;
        let ir = crate::compile_to_ir(source).expect("source compiles");
        let caps = RuntimeCapabilities {
            pagination: false,
            ..RuntimeCapabilities::interpreter_tier()
        };
        let gaps = check_contract_closure(&ir, caps);
        assert_eq!(gaps.len(), 1, "expected one pagination gap: {gaps:?}");
        assert!(gaps[0].missing_capability.contains("paginated"));
        assert!(
            check_contract_closure(&ir, RuntimeCapabilities::interpreter_tier()).is_empty(),
            "page route should serve under 52c-2"
        );
    }
}
