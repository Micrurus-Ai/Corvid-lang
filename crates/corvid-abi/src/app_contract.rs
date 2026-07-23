//! The Corvid Application Contract (Phase 51 slice 51a).
//!
//! A machine-readable description of a Corvid application's PUBLIC
//! surface — routes, public agents and prompts, the types they
//! exchange, and (uniquely) the AI-native capabilities of each:
//! whether it streams, whether its output is `Grounded`, whether its
//! input carries untrusted `Tainted` content, whether it can raise
//! approvals, its confidence floor, and its worst-case cost/latency.
//!
//! This is the sibling of [`crate::emit::emit_abi`]: the ABI
//! descriptor describes the C/cdylib export surface; the application
//! contract describes the HTTP + agent surface a FRONTEND consumes.
//! Both are emitted after resolve + type/effect checking and share
//! the same [`crate::type_description`] type shapes.
//!
//! Slice 51a ships the structural spine. The OpenAPI 3.1 projection
//! (51b), the richer AI-native event/approval metadata (51c), `@ui`
//! hints (51d), and typed-error presentation (51e) layer on top of
//! this model.

use corvid_ast::{Decl, DimensionValue, EffectDecl, File, Refinement, TypeRef};
use corvid_resolve::Resolved;
use corvid_types::{compute_worst_case_cost, effects::EffectRegistry, Checked};
use serde::{Deserialize, Serialize};

/// The full application contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationContract {
    /// Contract format version (independent of the compiler version).
    pub contract_version: u32,
    pub compiler_version: String,
    pub generated_at: String,
    pub source_path: String,
    /// Public types the surface exchanges, with their field
    /// refinements projected as value constraints.
    pub types: Vec<ContractType>,
    /// Declared HTTP routes.
    pub routes: Vec<ContractRoute>,
    /// Public agents (agent invocations, not ordinary REST).
    pub agents: Vec<ContractCallable>,
    /// Public prompts exposed to callers.
    pub prompts: Vec<ContractCallable>,
    /// Declared identity surfaces (slice 51g) — the providers a client
    /// can sign in with and the login-session configuration. Empty
    /// when the program declares no `identity` block.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identities: Vec<ContractIdentity>,
}

/// Public Corvid guarantee id the contract emitter enforces (slice
/// 51r): `contract.matches_compiled_surface`. The emitted contract
/// describes exactly the checked public surface — private declarations
/// never appear, and every capability derives from the checked
/// signature + composed effect row. Declared as a literal so the
/// `corvid-guarantees` inverse-coverage sentinel finds the enforcement
/// site (`emit_application_contract` filtering on
/// `is_callable_from_outside_file`) wired to the registry row.
#[allow(dead_code)]
pub const GUARANTEE_ID_CONTRACT_FIDELITY: &str = "contract.matches_compiled_surface";

/// An `identity Name:` surface (slice 51g). The SDK and dev console
/// render sign-in buttons from `providers`; `session` documents the
/// login-session posture (all safe-defaults unless a loud opt-out
/// weakened them). This is the login identity — deliberately separate
/// from connector workspace tokens (slice 51j).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractIdentity {
    pub name: String,
    pub providers: Vec<ContractProvider>,
    pub session: ContractSession,
    /// The auth routes the runtime auto-exposes for this identity
    /// (slice 51h): a login + callback per provider, plus logout and
    /// session. Derived — the program does not write them.
    pub auth_routes: Vec<ContractAuthRoute>,
    /// The mandatory OAuth safe-defaults every auth route enforces
    /// (slice 51h). Listed so the contract is explicit about the
    /// guaranteed posture rather than leaving it to documentation.
    pub safeguards: Vec<String>,
    /// Account-linking policy (slice 51i).
    pub linking: ContractLinking,
}

/// The account-linking posture surfaced to clients (slice 51i).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractLinking {
    /// Always true — linking runs the explicit-confirmation flow, and
    /// there is no way to disable it.
    pub confirmation_required: bool,
    /// `never` | `verified_domain` — how a same-email account on a
    /// different provider is treated. Never a silent merge.
    pub email_match: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verified_domains: Vec<String>,
}

/// One auto-exposed auth route (slice 51h).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractAuthRoute {
    pub method: String,
    pub path: String,
    /// `login` | `callback` | `logout` | `session`.
    pub purpose: String,
    /// The provider wire-name, for `login` / `callback` routes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// One configured identity provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractProvider {
    /// `google`/`github`/`microsoft`/`apple`/`discord`/`slack`/`oidc`.
    pub kind: String,
    /// The wire name used in the auth route path (the alias for OIDC).
    pub name: String,
    /// The OIDC discovery URL, for `kind == "oidc"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_url: Option<String>,
}

/// The login-session posture surfaced to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_secs: Option<u64>,
    pub cookie_secure: bool,
    pub cookie_http_only: bool,
    pub same_site: String,
    pub rotate_on_privilege_change: bool,
}

/// The current contract format version. Bumped only on a breaking
/// change to the emitted shape — the same compatibility discipline
/// the ABI descriptor carries.
pub const CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractType {
    pub name: String,
    /// Record fields (empty for a sum type).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ContractField>,
    /// Sum-type variants (empty for a record) — name, payload fields,
    /// and per-variant presentation defaults, so a frontend handles
    /// error/state enums EXHAUSTIVELY.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<ContractVariant>,
}

/// One sum-type variant in the contract (slice 51e).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractVariant {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ContractField>,
    /// `@status(code)` — the HTTP status this variant maps to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u64>,
    /// `@ui(...)` presentation defaults (e.g. a user-facing message).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ui: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractField {
    pub name: String,
    /// JSON-Schema-flavored type name for the field.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Value constraints from a `where` refinement (slice 50j),
    /// projected into the JSON-Schema vocabulary so frontends get
    /// client-side validation for free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    /// Optional `@ui(...)` presentation hints (slice 51d). A SEPARATE
    /// channel from the constraints above: a frontend may ignore
    /// these display suggestions but never the semantic constraints.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub ui: std::collections::BTreeMap<String, serde_json::Value>,
    /// Upload constraints (slice 51f) when this field is typed
    /// `Upload<Format>`: accepted MIME (format default merged with an
    /// explicit `@upload(mime:)`), max size, and retention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<ContractUpload>,
}

/// The upload surface of an `Upload<Format>` field (slice 51f). A
/// frontend renders a file picker constrained to `accepted_mime` and
/// `max_bytes`; the server rejects anything outside them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractUpload {
    /// The format tag (`Pdf`, `Image`, ...) from `Upload<Format>`.
    pub format: String,
    /// Accepted MIME types the boundary allows.
    pub accepted_mime: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRoute {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_params: Vec<ContractParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_type: Option<String>,
    pub response_type: String,
    /// Permissions the route requires (from its effect row's trust /
    /// dangerous surface). Empty = no approval-requiring effect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Pagination surface (slice 51f) when the response is
    /// `Page<Item>` / `Stream<Item>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    /// Auth policy (slice 51h) from a `requires authenticated|role|
    /// permission` clause. Present = the route needs a login session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<ContractRoutePolicy>,
}

/// A route's authentication/authorization policy (slice 51h).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRoutePolicy {
    pub authenticated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractParam {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// A public agent or prompt, with its AI-native capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCallable {
    pub name: String,
    pub inputs: Vec<ContractParam>,
    pub output_type: String,
    pub capabilities: Capabilities,
}

/// What a frontend must know to invoke a callable correctly — the
/// information ordinary API specs cannot express.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Output is a `Stream<T>` — the client consumes SSE events, not
    /// a single response.
    pub streaming: bool,
    /// Output is `Grounded<T>` — carries citation sources.
    pub grounded: bool,
    /// An input is `Tainted<T>` — the callee reads untrusted content
    /// (slice 50i); the frontend should treat inputs accordingly.
    pub tainted_input: bool,
    /// The callee (transitively) reaches an approval-requiring call,
    /// so an `approval_required` event is possible mid-invocation.
    pub approvals_possible: bool,
    /// Confidence floor, when the callee is confidence-gated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_min: Option<f64>,
    /// Worst-case USD cost the checker proved for the callee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Latency class from the composed effect row (`fast`/`medium`/
    /// `slow`), when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<String>,
    /// Pagination surface (slice 51f) when the output is `Page<Item>`
    /// (cursor) or `Stream<Item>` (stream). A generic paginated hook
    /// reads this to drive "load more" / consume-to-end uniformly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}

/// How a callable's output is paginated (slice 51f).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub style: PaginationStyle,
    /// The element type inside the page/stream.
    pub item_type: String,
    /// The query parameter that carries the opaque cursor
    /// (`cursor` for cursor pagination; absent for streams).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_param: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaginationStyle {
    /// `Page<Item>` — opaque forward cursor, one page per request.
    Cursor,
    /// `Stream<Item>` — an SSE/element stream consumed to completion.
    Stream,
}

/// Options threaded from the caller (compiler version, timestamp).
pub struct ContractOptions<'a> {
    pub source_path: &'a str,
    pub compiler_version: &'a str,
    pub generated_at: &'a str,
}

/// Emit the application contract from the checked program.
pub fn emit_application_contract(
    file: &File,
    resolved: &Resolved,
    checked: &Checked,
    registry: &EffectRegistry,
    opts: &ContractOptions<'_>,
) -> ApplicationContract {
    let types = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Type(t) if t.visibility.is_callable_from_outside_file() => {
                Some(contract_type(t))
            }
            _ => None,
        })
        .collect();

    let routes = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Server(server) => Some(server),
            _ => None,
        })
        .flat_map(|server| server.routes.iter().map(contract_route))
        .collect();

    // `compute_worst_case_cost` is available for a tighter
    // per-body bound; the contract advertises the callable's
    // COMPOSED EFFECT-ROW surface (what a frontend should budget for),
    // which composes downstream effects through the call graph.
    let _ = (resolved, checked, compute_worst_case_cost);

    let agents = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Agent(a) if a.visibility.is_callable_from_outside_file() => Some(
                contract_callable(
                    &a.name.name,
                    &a.params,
                    &a.return_ty,
                    &a.effect_row,
                    registry,
                ),
            ),
            _ => None,
        })
        .collect();

    let prompts = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Prompt(p) if p.visibility.is_callable_from_outside_file() => Some(
                contract_callable(
                    &p.name.name,
                    &p.params,
                    &p.return_ty,
                    &p.effect_row,
                    registry,
                ),
            ),
            _ => None,
        })
        .collect();

    let identities = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Identity(i) => Some(contract_identity(i)),
            _ => None,
        })
        .collect();

    ApplicationContract {
        contract_version: CONTRACT_VERSION,
        compiler_version: opts.compiler_version.to_string(),
        generated_at: opts.generated_at.to_string(),
        source_path: opts.source_path.to_string(),
        types,
        routes,
        agents,
        prompts,
        identities,
    }
}

fn contract_identity(decl: &corvid_ast::IdentityDecl) -> ContractIdentity {
    use corvid_ast::ProviderKind;
    let providers = decl
        .providers
        .iter()
        .map(|p| match &p.kind {
            ProviderKind::Oidc { discovery_url, alias } => ContractProvider {
                kind: "oidc".into(),
                name: alias.name.clone(),
                discovery_url: Some(discovery_url.clone()),
            },
            other => ContractProvider {
                kind: other.wire_name(),
                name: other.wire_name(),
                discovery_url: None,
            },
        })
        .collect();
    let session = decl.session.clone().unwrap_or_default();

    // Auto-exposed auth routes (slice 51h): login + callback per
    // provider, plus a shared logout and session endpoint.
    let mut auth_routes = Vec::new();
    for p in &decl.providers {
        let provider = p.kind.wire_name();
        auth_routes.push(ContractAuthRoute {
            method: "GET".into(),
            path: format!("/auth/{provider}/login"),
            purpose: "login".into(),
            provider: Some(provider.clone()),
        });
        auth_routes.push(ContractAuthRoute {
            method: "GET".into(),
            path: format!("/auth/{provider}/callback"),
            purpose: "callback".into(),
            provider: Some(provider),
        });
    }
    auth_routes.push(ContractAuthRoute {
        method: "POST".into(),
        path: "/auth/logout".into(),
        purpose: "logout".into(),
        provider: None,
    });
    auth_routes.push(ContractAuthRoute {
        method: "GET".into(),
        path: "/auth/session".into(),
        purpose: "session".into(),
        provider: None,
    });

    // Account-linking routes (slice 51i): start a link to another
    // provider, then confirm ownership. Derived from the providers.
    for p in &decl.providers {
        let provider = p.kind.wire_name();
        auth_routes.push(ContractAuthRoute {
            method: "POST".into(),
            path: format!("/auth/link/{provider}/start"),
            purpose: "link_start".into(),
            provider: Some(provider),
        });
    }
    auth_routes.push(ContractAuthRoute {
        method: "POST".into(),
        path: "/auth/link/confirm".into(),
        purpose: "link_confirm".into(),
        provider: None,
    });

    let linking = decl.linking.clone().unwrap_or_default();

    ContractIdentity {
        name: decl.name.name.clone(),
        providers,
        session: ContractSession {
            lifetime_secs: session.lifetime_secs,
            cookie_secure: session.cookie.secure,
            cookie_http_only: session.cookie.http_only,
            same_site: session.cookie.same_site.wire_name().to_string(),
            rotate_on_privilege_change: session.rotate_on_privilege_change,
        },
        auth_routes,
        safeguards: mandatory_auth_safeguards(),
        linking: ContractLinking {
            confirmation_required: true,
            email_match: linking.email_match.wire_name().to_string(),
            verified_domains: linking.verified_domains.clone(),
        },
    }
}

/// The OAuth safe-defaults every auto-exposed auth route enforces
/// (slice 51h). Named in the contract so the guaranteed posture is
/// explicit and machine-readable, not just documented.
fn mandatory_auth_safeguards() -> Vec<String> {
    [
        "authorization_code_with_pkce",
        "signed_expiring_state",
        "oidc_nonce",
        "exact_redirect_uri_allowlist",
        "jwks_signature_verification",
        "iss_aud_exp_nbf_validation",
        "secure_http_only_cookies",
        "session_rotation_on_privilege_change",
        "csrf_double_submit",
        "refresh_token_rotation",
        "encrypted_provider_tokens",
        "token_revocation",
        "redacted_auth_logs",
        "minimal_scopes",
        // Account-linking guarantees (slice 51i).
        "explicit_link_confirmation",
        "no_silent_email_merge",
        "link_ownership_proof",
        "link_audit_trail",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn contract_type(t: &corvid_ast::TypeDecl) -> ContractType {
    ContractType {
        name: t.name.name.clone(),
        fields: t.fields.iter().map(contract_field).collect(),
        variants: t.variants.iter().map(contract_variant).collect(),
    }
}

fn contract_variant(v: &corvid_ast::SumVariant) -> ContractVariant {
    ContractVariant {
        name: v.name.name.clone(),
        fields: v.fields.iter().map(contract_field).collect(),
        status: v.status,
        ui: ui_hints_map(&v.ui),
    }
}

/// Project a slice of `@ui` hints into a name → JSON-value map.
fn ui_hints_map(hints: &[corvid_ast::UiHint]) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut map = std::collections::BTreeMap::new();
    for hint in hints {
        let value = match &hint.value {
            corvid_ast::UiHintValue::Str(s) => serde_json::Value::String(s.clone()),
            corvid_ast::UiHintValue::Bool(b) => serde_json::Value::Bool(*b),
            corvid_ast::UiHintValue::Int(n) => serde_json::Value::from(*n),
        };
        map.insert(hint.key.name.clone(), value);
    }
    map
}

fn contract_field(f: &corvid_ast::Field) -> ContractField {
    let mut field = ContractField {
        name: f.name.name.clone(),
        type_name: type_ref_name(&f.ty),
        minimum: None,
        maximum: None,
        min_length: None,
        max_length: None,
        ui: ui_hints_map(&f.ui),
        upload: contract_upload(&f.ty, f.upload.as_ref()),
    };
    match f.refinement {
        Some(Refinement::Between { min, max }) => {
            field.minimum = Some(min);
            field.maximum = Some(max);
        }
        Some(Refinement::LenBetween { min, max }) => {
            field.min_length = Some(min);
            field.max_length = Some(max);
        }
        None => {}
    }
    field
}

/// The upload surface of a field, when its type is `Upload<Format>`.
/// The format tag supplies default accepted MIME, which an explicit
/// `@upload(mime:)` overrides; size/retention come from `@upload`.
fn contract_upload(
    ty: &TypeRef,
    spec: Option<&corvid_ast::UploadSpec>,
) -> Option<ContractUpload> {
    let format = upload_format_tag(ty)?;
    let accepted_mime = match spec {
        Some(s) if !s.mime.is_empty() => s.mime.clone(),
        _ => default_mime_for_format(&format),
    };
    Some(ContractUpload {
        format,
        accepted_mime,
        max_bytes: spec.and_then(|s| s.max_bytes),
        retention_days: spec.and_then(|s| s.retention_days),
    })
}

/// The `Format` in `Upload<Format>`, or `None` if the type is not an
/// upload.
fn upload_format_tag(ty: &TypeRef) -> Option<String> {
    match ty {
        TypeRef::Generic { name, args, .. } if name.name == "Upload" && args.len() == 1 => {
            Some(type_ref_name(&args[0]))
        }
        _ => None,
    }
}

/// Default accepted MIME types for a well-known upload format tag. An
/// unknown tag falls back to `application/octet-stream` so the surface
/// stays usable without special-casing every format.
///
/// This is the SINGLE SOURCE OF TRUTH for the format→MIME mapping: the
/// Application Contract advertises it (frontend pickers constrain to
/// it) AND `corvid serve` enforces it on the multipart upload boundary
/// (slice 52c-2), so the runtime can never accept a media type the
/// contract did not promise.
pub fn default_mime_for_format(format: &str) -> Vec<String> {
    let mimes: &[&str] = match format {
        "Pdf" => &["application/pdf"],
        "Image" => &["image/png", "image/jpeg", "image/gif", "image/webp"],
        "Csv" => &["text/csv"],
        "Json" => &["application/json"],
        "Text" => &["text/plain"],
        "Audio" => &["audio/mpeg", "audio/wav", "audio/ogg"],
        "Video" => &["video/mp4", "video/webm"],
        _ => &["application/octet-stream"],
    };
    mimes.iter().map(|m| m.to_string()).collect()
}

/// Pagination surface for a callable/route output type: `Page<Item>`
/// is cursor pagination; `Stream<Item>` is stream pagination.
fn pagination_for(ty: &TypeRef) -> Option<Pagination> {
    match ty {
        TypeRef::Generic { name, args, .. } if name.name == "Page" && args.len() == 1 => {
            Some(Pagination {
                style: PaginationStyle::Cursor,
                item_type: type_ref_name(&args[0]),
                cursor_param: Some("cursor".to_string()),
            })
        }
        TypeRef::Generic { name, args, .. } if name.name == "Stream" && args.len() == 1 => {
            Some(Pagination {
                style: PaginationStyle::Stream,
                item_type: type_ref_name(&args[0]),
                cursor_param: None,
            })
        }
        _ => None,
    }
}

fn contract_route(r: &corvid_ast::HttpRouteDecl) -> ContractRoute {
    ContractRoute {
        method: r.method.as_str().to_string(),
        path: r.path.clone(),
        path_params: r
            .path_params
            .iter()
            .map(|p| ContractParam {
                name: p.name.name.clone(),
                type_name: type_ref_name(&p.ty),
            })
            .collect(),
        query_type: r.query_ty.as_ref().map(type_ref_name),
        body_type: r.body_ty.as_ref().map(type_ref_name),
        response_type: type_ref_name(&r.response.ty),
        // A route inherits the permission surface of its effect row;
        // 51h wires named permissions. For 51a we surface the effect
        // names that require approval.
        requires: r
            .effect_row
            .effects
            .iter()
            .map(|e| e.name.name.clone())
            .collect(),
        pagination: pagination_for(&r.response.ty),
        policy: r.policy.as_ref().filter(|p| p.requires_auth()).map(|p| {
            ContractRoutePolicy {
                authenticated: true,
                roles: p.roles.clone(),
                permissions: p.permissions.clone(),
            }
        }),
    }
}

/// Collect the file's effect declarations to build the registry the
/// contract analysis composes against.
pub fn effect_decls_of(file: &File) -> Vec<EffectDecl> {
    file.decls
        .iter()
        .filter_map(|decl| match decl {
            Decl::Effect(e) => Some(e.clone()),
            _ => None,
        })
        .collect()
}

fn contract_callable(
    name: &str,
    params: &[corvid_ast::Param],
    return_ty: &TypeRef,
    effect_row: &corvid_ast::EffectRow,
    registry: &EffectRegistry,
) -> ContractCallable {
    let inputs = params
        .iter()
        .map(|p| ContractParam {
            name: p.name.name.clone(),
            type_name: type_ref_name(&p.ty),
        })
        .collect::<Vec<_>>();

    let tainted_input = params.iter().any(|p| type_ref_is_tainted(&p.ty));

    let composed = registry.compose(
        &effect_row
            .effects
            .iter()
            .map(|e| e.name.name.as_str())
            .collect::<Vec<_>>(),
    );
    let latency_class = match composed.dimensions.get("latency") {
        Some(DimensionValue::Name(class)) => Some(class.clone()),
        _ => None,
    };
    // Only advertise a confidence floor when there is a REAL gate;
    // 1.0 is the "fully confident / no floor declared" default and
    // would mislead a frontend into showing a strict gate.
    let confidence_min = match composed.dimensions.get("confidence") {
        Some(DimensionValue::Number(v)) if *v < 1.0 => Some(*v),
        _ => None,
    };
    let max_cost_usd = match composed.dimensions.get("cost") {
        Some(DimensionValue::Cost(v)) if *v > 0.0 => Some(*v),
        Some(DimensionValue::Number(v)) if *v > 0.0 => Some(*v),
        _ => None,
    };
    let approvals_possible = corvid_types::effects::effect_row_trust_requires_approval(
        effect_row, registry,
    )
    .is_some();

    ContractCallable {
        name: name.to_string(),
        inputs,
        output_type: type_ref_name(return_ty),
        capabilities: Capabilities {
            streaming: type_ref_head_is(return_ty, "Stream"),
            grounded: type_ref_head_is(return_ty, "Grounded"),
            tainted_input,
            approvals_possible,
            confidence_min,
            max_cost_usd,
            latency_class,
            pagination: pagination_for(return_ty),
        },
    }
}

/// A frontend-facing name for a `TypeRef`. Named types keep their
/// name (referencing the `types` section); generics render their
/// head + inner recursively so `Stream<Answer>` and `Grounded<Doc>`
/// read the way a client expects.
fn type_ref_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named { name, .. } => name.name.clone(),
        TypeRef::Generic { name, args, .. } => {
            let inner = args.iter().map(type_ref_name).collect::<Vec<_>>().join(", ");
            format!("{}<{}>", name.name, inner)
        }
        TypeRef::Qualified { name, .. } => name.name.clone(),
        TypeRef::Weak { inner, .. } => format!("Weak<{}>", type_ref_name(inner)),
        // Function types don't appear on the public exchange surface.
        other => format!("{other:?}"),
    }
}

fn type_ref_head_is(ty: &TypeRef, head: &str) -> bool {
    matches!(ty, TypeRef::Generic { name, .. } if name.name == head)
}

fn type_ref_is_tainted(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Generic { name, .. } if name.name == "Tainted" => true,
        TypeRef::Generic { args, .. } => args.iter().any(type_ref_is_tainted),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_for(src: &str) -> ApplicationContract {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
        let registry = EffectRegistry::from_decls(&effect_decls_of(&file));
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "check: {:?}", checked.errors);
        emit_application_contract(
            &file,
            &resolved,
            &checked,
            &registry,
            &ContractOptions {
                source_path: "app.cor",
                compiler_version: "test",
                generated_at: "now",
            },
        )
    }

    #[test]
    fn public_agent_capabilities_reflect_return_type_and_effects() {
        let contract = contract_for(
            "\
effect ask:
    cost: $0.05
    latency: fast

public type Answer:
    text: String
    score: Int where between(0, 100)

public agent support(question: String) -> Answer uses ask:
    return Answer(\"hi\", 90)
",
        );
        assert_eq!(contract.contract_version, CONTRACT_VERSION);
        let ty = contract.types.iter().find(|t| t.name == "Answer").unwrap();
        let score = ty.fields.iter().find(|f| f.name == "score").unwrap();
        assert_eq!(score.minimum, Some(0));
        assert_eq!(score.maximum, Some(100));

        let agent = contract.agents.iter().find(|a| a.name == "support").unwrap();
        assert!(!agent.capabilities.streaming);
        assert_eq!(agent.capabilities.latency_class.as_deref(), Some("fast"));
        assert_eq!(agent.capabilities.max_cost_usd, Some(0.05));
    }

    #[test]
    fn streaming_and_grounded_capabilities_are_detected() {
        let contract = contract_for(
            "\
effect retrieval:
    data: grounded

public agent chat(message: String) -> Stream<String>:
    return stream_answer(message)

tool stream_answer(m: String) -> Stream<String>

public agent lookup(q: String) -> Grounded<String> uses retrieval:
    return fetch(q)

tool fetch(q: String) -> Grounded<String> uses retrieval
",
        );
        let chat = contract.agents.iter().find(|a| a.name == "chat").unwrap();
        assert!(chat.capabilities.streaming);
        let lookup = contract.agents.iter().find(|a| a.name == "lookup").unwrap();
        assert!(lookup.capabilities.grounded);
    }

    #[test]
    fn confidence_floor_appears_only_when_gated() {
        let contract = contract_for(
            "effect judged:
    confidence: 0.85

public agent classify(text: String) -> String uses judged:
    return text

public agent plain(text: String) -> String:
    return text
",
        );
        let classify = contract.agents.iter().find(|a| a.name == "classify").unwrap();
        assert_eq!(classify.capabilities.confidence_min, Some(0.85));
        let plain = contract.agents.iter().find(|a| a.name == "plain").unwrap();
        assert_eq!(plain.capabilities.confidence_min, None);
    }

    #[test]
    fn error_enum_variants_carry_status_ui_and_payload() {
        let contract = contract_for(
            "public type RefundError:
    @status(404)
    @ui(message: \"Not found.\")
    | PaymentNotFound
    | ApprovalDenied(reason: String)

public agent submit(x: String) -> RefundError:
    return PaymentNotFound
",
        );
        let e = contract.types.iter().find(|t| t.name == "RefundError").unwrap();
        assert_eq!(e.variants.len(), 2);
        let not_found = &e.variants[0];
        assert_eq!(not_found.name, "PaymentNotFound");
        assert_eq!(not_found.status, Some(404));
        assert_eq!(not_found.ui.get("message").unwrap(), "Not found.");
        let denied = &e.variants[1];
        assert_eq!(denied.fields.len(), 1);
        assert_eq!(denied.fields[0].name, "reason");
    }

    #[test]
    fn upload_field_surfaces_mime_size_and_retention() {
        let contract = contract_for(
            "public type DocSubmission:
    @upload(max_mb: 10, retention_days: 7)
    file: Upload<Pdf>
    note: String

public agent ingest(doc: DocSubmission) -> String:
    return doc.note
",
        );
        let ty = contract.types.iter().find(|t| t.name == "DocSubmission").unwrap();
        let file = ty.fields.iter().find(|f| f.name == "file").unwrap();
        let upload = file.upload.as_ref().expect("upload surface present");
        assert_eq!(upload.format, "Pdf");
        assert_eq!(upload.accepted_mime, vec!["application/pdf".to_string()]);
        assert_eq!(upload.max_bytes, Some(10 * 1024 * 1024));
        assert_eq!(upload.retention_days, Some(7));
        // A non-upload field carries no upload surface.
        let note = ty.fields.iter().find(|f| f.name == "note").unwrap();
        assert!(note.upload.is_none());
    }

    #[test]
    fn explicit_upload_mime_overrides_format_default() {
        let contract = contract_for(
            "public type Avatar:
    @upload(mime: \"image/png, image/jpeg\")
    picture: Upload<Image>

public agent set_avatar(a: Avatar) -> String:
    return \"ok\"
",
        );
        let ty = contract.types.iter().find(|t| t.name == "Avatar").unwrap();
        let pic = ty.fields.iter().find(|f| f.name == "picture").unwrap();
        let upload = pic.upload.as_ref().unwrap();
        assert_eq!(
            upload.accepted_mime,
            vec!["image/png".to_string(), "image/jpeg".to_string()]
        );
        assert!(upload.max_bytes.is_none());
    }

    #[test]
    fn page_return_advertises_cursor_pagination() {
        let contract = contract_for(
            "public type Item:
    id: String

tool fetch_page(cursor: String) -> Page<Item>

public agent list_items(cursor: String) -> Page<Item>:
    return fetch_page(cursor)
",
        );
        let agent = contract.agents.iter().find(|a| a.name == "list_items").unwrap();
        let pg = agent.capabilities.pagination.as_ref().expect("pagination present");
        assert_eq!(pg.style, PaginationStyle::Cursor);
        assert_eq!(pg.item_type, "Item");
        assert_eq!(pg.cursor_param.as_deref(), Some("cursor"));
    }

    #[test]
    fn stream_return_advertises_stream_pagination() {
        let contract = contract_for(
            "public agent chat(message: String) -> Stream<String>:
    return stream_answer(message)

tool stream_answer(m: String) -> Stream<String>
",
        );
        let agent = contract.agents.iter().find(|a| a.name == "chat").unwrap();
        let pg = agent.capabilities.pagination.as_ref().expect("pagination present");
        assert_eq!(pg.style, PaginationStyle::Stream);
        assert_eq!(pg.item_type, "String");
        assert!(pg.cursor_param.is_none());
        assert!(agent.capabilities.streaming);
    }

    #[test]
    fn identity_surface_lists_providers_and_session_posture() {
        let contract = contract_for(
            "identity app_users:
    provider google
    provider github
    provider oidc \"https://issuer.example.com/.well-known/openid-configuration\" as corp
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    session:
        lifetime: 24h
        same_site: strict
",
        );
        assert_eq!(contract.identities.len(), 1);
        let id = &contract.identities[0];
        assert_eq!(id.name, "app_users");
        assert_eq!(id.providers.len(), 3);
        assert_eq!(id.providers[0].kind, "google");
        let oidc = id.providers.iter().find(|p| p.kind == "oidc").unwrap();
        assert_eq!(oidc.name, "corp");
        assert!(oidc.discovery_url.as_deref().unwrap().starts_with("https://"));
        // Safe defaults hold, and lifetime parsed from `24h`.
        assert_eq!(id.session.lifetime_secs, Some(24 * 3600));
        assert_eq!(id.session.same_site, "strict");
        assert!(id.session.cookie_secure);
        assert!(id.session.cookie_http_only);
        assert!(id.session.rotate_on_privilege_change);
    }

    #[test]
    fn identity_unsafe_cookie_without_opt_out_is_rejected() {
        let src = "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    session:
        secure: false
";
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(
            checked.errors.iter().any(|e| matches!(
                e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { .. }
            )),
            "expected an IdentityConfigInvalid error, got {:?}",
            checked.errors
        );
    }

    #[test]
    fn identity_unsafe_cookie_with_opt_out_warns_but_compiles() {
        let src = "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    session:
        secure: false
        insecure_opt_out: true
";
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(checked.errors.is_empty(), "unexpected errors: {:?}", checked.errors);
        assert!(
            checked.warnings.iter().any(|w| matches!(
                w.kind,
                corvid_types::TypeWarningKind::IdentityInsecureSession { .. }
            )),
            "expected an insecure-session warning"
        );
    }

    #[test]
    fn identity_auto_exposes_auth_routes_with_safeguards() {
        let contract = contract_for(
            "identity app_users:
    provider google
    provider github
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
",
        );
        let id = &contract.identities[0];
        // login + callback per provider (2×2), plus logout + session,
        // plus a link-start per provider (2) and one link-confirm.
        assert_eq!(id.auth_routes.len(), 2 * 2 + 2 + 2 + 1);
        assert!(id
            .auth_routes
            .iter()
            .any(|r| r.path == "/auth/google/login" && r.purpose == "login"));
        assert!(id
            .auth_routes
            .iter()
            .any(|r| r.path == "/auth/github/callback" && r.purpose == "callback"));
        assert!(id.auth_routes.iter().any(|r| r.path == "/auth/logout"));
        assert!(id.auth_routes.iter().any(|r| r.path == "/auth/session"));
        // The mandatory safe-defaults are named in the contract.
        for required in [
            "authorization_code_with_pkce",
            "jwks_signature_verification",
            "secure_http_only_cookies",
            "refresh_token_rotation",
        ] {
            assert!(id.safeguards.iter().any(|s| s == required), "missing {required}");
        }
    }

    #[test]
    fn route_requires_policy_surfaces_and_binds_typed_actor() {
        let contract = contract_for(
            "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    roles:
        admin: \"user:read, user:write\"

server admin_api:
    route GET \"/admin\" -> json String requires role(\"admin\"):
        return actor.id
",
        );
        let route = contract.routes.iter().find(|r| r.path == "/admin").unwrap();
        let policy = route.policy.as_ref().expect("policy present");
        assert!(policy.authenticated);
        assert_eq!(policy.roles, vec!["admin".to_string()]);
    }

    #[test]
    fn linking_defaults_to_never_and_exposes_link_routes() {
        let contract = contract_for(
            "identity app_users:
    provider google
    provider github
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
",
        );
        let id = &contract.identities[0];
        // Defaults: confirmation always required, email-match never.
        assert!(id.linking.confirmation_required);
        assert_eq!(id.linking.email_match, "never");
        // A link-start route per provider, plus one shared confirm.
        assert!(id
            .auth_routes
            .iter()
            .any(|r| r.path == "/auth/link/google/start" && r.purpose == "link_start"));
        assert!(id
            .auth_routes
            .iter()
            .any(|r| r.path == "/auth/link/confirm" && r.purpose == "link_confirm"));
        // The never-silent-merge guarantees are named in the contract.
        for g in ["explicit_link_confirmation", "no_silent_email_merge", "link_audit_trail"] {
            assert!(id.safeguards.iter().any(|s| s == g), "missing {g}");
        }
    }

    #[test]
    fn linking_verified_domain_surfaces_domains() {
        let contract = contract_for(
            "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    linking:
        email_match: verified_domain
        verified_domains: \"example.com, corp.example.com\"
",
        );
        let linking = &contract.identities[0].linking;
        assert_eq!(linking.email_match, "verified_domain");
        assert_eq!(
            linking.verified_domains,
            vec!["example.com".to_string(), "corp.example.com".to_string()]
        );
    }

    #[test]
    fn linking_verified_domain_without_domains_is_rejected() {
        let src = "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    linking:
        email_match: verified_domain
";
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(
            checked.errors.iter().any(|e| matches!(
                e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { .. }
            )),
            "expected IdentityConfigInvalid, got {:?}",
            checked.errors
        );
    }

    #[test]
    fn linking_malformed_verified_domain_is_rejected() {
        let src = "identity app_users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed(\"public\")
    linking:
        email_match: verified_domain
        verified_domains: \"https://example.com/path\"
";
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(
            checked.errors.iter().any(|e| matches!(
                e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { .. }
            )),
            "expected IdentityConfigInvalid for malformed domain, got {:?}",
            checked.errors
        );
    }

    #[test]
    fn route_requires_without_identity_is_rejected() {
        let src = "server admin_api:
    route GET \"/admin\" -> json String requires authenticated:
        return \"ok\"
";
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        let checked = corvid_types::typecheck(&file, &resolved);
        assert!(
            checked.errors.iter().any(|e| matches!(
                e.kind,
                corvid_types::TypeErrorKind::RoutePolicyWithoutIdentity { .. }
            )),
            "expected RoutePolicyWithoutIdentity, got {:?}",
            checked.errors
        );
    }

    /// Run the front-half of the pipeline and return the checker errors,
    /// asserting the source lexes/parses/resolves cleanly first.
    fn check_errors_of(src: &str) -> Vec<corvid_types::TypeError> {
        let tokens = corvid_syntax::lex(src).expect("lex");
        let (file, perr) = corvid_syntax::parse_file(&tokens);
        assert!(perr.is_empty(), "parse: {perr:?}");
        let resolved = corvid_resolve::resolve(&file);
        corvid_types::typecheck(&file, &resolved).errors
    }

    #[test]
    fn identity_with_oauth_provider_but_no_provisioning_is_rejected() {
        // The whole point of slice 52e: an OAuth block must state its
        // first-login posture — omission is a compile error, never a
        // silent open-registration default.
        let errors = check_errors_of("identity users:\n    provider google\n");
        assert!(
            errors.iter().any(|e| matches!(
                e.kind,
                corvid_types::TypeErrorKind::FirstLoginPolicyRequired { .. }
            )),
            "expected FirstLoginPolicyRequired, got {errors:?}"
        );
    }

    #[test]
    fn identity_provisioning_open_with_fixed_tenant_compiles() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n",
        );
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn identity_provisioning_invited_from_invitation_compiles() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: invited\n        tenant: from_invitation\n",
        );
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn identity_provisioning_approval_required_is_rejected_until_supported() {
        // `approval_required` parses so the checker can name it, but the
        // runtime cannot execute durable approval yet — reject rather
        // than silently degrade to a weaker posture.
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: approval_required\n        tenant: fixed(\"public\")\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("approval_required")
            )),
            "expected an approval_required rejection, got {errors:?}"
        );
    }

    #[test]
    fn identity_provisioning_from_invitation_requires_invited_mode() {
        // A tenant read from an invitation makes no sense without an
        // invitation gate.
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: from_invitation\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("from_invitation")
            )),
            "expected a from_invitation/invited mismatch, got {errors:?}"
        );
    }

    #[test]
    fn identity_provisioning_claim_mapping_empty_allowlist_is_rejected() {
        // An empty allowlist would trust any claim value.
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: from_claim(\"org_id\") allow \"\"\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("allow")
            )),
            "expected an empty-allowlist rejection, got {errors:?}"
        );
    }

    #[test]
    fn provisioning_without_a_provider_is_rejected() {
        // A provisioning policy with nothing to provision can never
        // fire; flag it rather than let it read as if in force. (Also
        // trips the no-providers error — both are IdentityConfigInvalid.)
        let errors = check_errors_of(
            "identity users:\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("provisioning")
            )),
            "expected a provisioning-without-provider rejection, got {errors:?}"
        );
    }

    #[test]
    fn a_declared_role_and_permission_referenced_by_a_route_compiles() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n    roles:\n        admin: \"refund:write, user:read\"\n\nserver api:\n    route GET \"/admin\" -> json String requires role(\"admin\") and permission(\"refund:write\"):\n        return actor.id\n",
        );
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn requires_an_undeclared_role_is_rejected() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n    roles:\n        admin: \"user:read\"\n\nserver api:\n    route GET \"/x\" -> json String requires role(\"superadmin\"):\n        return actor.id\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::RoutePolicyUndeclaredAuthority { kind, name, .. }
                    if *kind == "role" && name == "superadmin"
            )),
            "expected an undeclared-role rejection, got {errors:?}"
        );
    }

    #[test]
    fn requires_an_undeclared_permission_is_rejected() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n    roles:\n        admin: \"user:read\"\n\nserver api:\n    route GET \"/x\" -> json String requires permission(\"refund:write\"):\n        return actor.id\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::RoutePolicyUndeclaredAuthority { kind, name, .. }
                    if *kind == "permission" && name == "refund:write"
            )),
            "expected an undeclared-permission rejection, got {errors:?}"
        );
    }

    #[test]
    fn default_role_must_reference_a_declared_role() {
        let errors = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n        default_role: member\n    roles:\n        admin: \"user:read\"\n",
        );
        assert!(
            errors.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("default_role")
            )),
            "expected a default_role rejection, got {errors:?}"
        );
    }

    #[test]
    fn a_duplicate_role_and_an_empty_role_are_rejected() {
        let dup = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n    roles:\n        admin: \"user:read\"\n        admin: \"user:write\"\n",
        );
        assert!(
            dup.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("duplicate role")
            )),
            "expected a duplicate-role rejection, got {dup:?}"
        );
        let empty = check_errors_of(
            "identity users:\n    provider google\n    provisioning:\n        first_login: open\n        tenant: fixed(\"public\")\n    roles:\n        ghost: \"\"\n",
        );
        assert!(
            empty.iter().any(|e| matches!(
                &e.kind,
                corvid_types::TypeErrorKind::IdentityConfigInvalid { message, .. }
                    if message.contains("grants no permissions")
            )),
            "expected an empty-role rejection, got {empty:?}"
        );
    }

    #[test]
    fn private_agents_are_not_in_the_contract() {
        let contract = contract_for(
            "\
agent internal_helper(x: String) -> String:
    return x
",
        );
        assert!(contract.agents.is_empty(), "private agents must not leak");
    }
}
