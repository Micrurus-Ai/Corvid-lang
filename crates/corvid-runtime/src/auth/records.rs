#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthActor {
    pub id: String,
    pub tenant_id: String,
    pub display_name: String,
    pub actor_kind: String,
    pub auth_method: String,
    pub assurance_level: String,
    pub role_fingerprint: String,
    pub permission_fingerprint: String,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub actor_id: String,
    pub tenant_id: String,
    pub token_hash: String,
    pub issued_ms: u64,
    pub expires_ms: u64,
    pub rotation_counter: u64,
    pub csrf_binding_id: String,
    pub revoked_ms: Option<u64>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreate {
    pub id: String,
    pub actor_id: String,
    pub tenant_id: String,
    pub raw_token: String,
    pub issued_ms: u64,
    pub expires_ms: u64,
    pub csrf_binding_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyRecord {
    pub id: String,
    pub service_actor_id: String,
    pub tenant_id: String,
    pub key_hash: String,
    pub scope_fingerprint: String,
    pub expires_ms: u64,
    pub last_used_ms: Option<u64>,
    pub revoked_ms: Option<u64>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyCreate {
    pub id: String,
    pub service_actor_id: String,
    pub tenant_id: String,
    pub raw_key: String,
    pub scope_fingerprint: String,
    pub expires_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthTraceContext {
    pub trace_id: String,
    pub tenant_id: String,
    pub actor_id: String,
    pub auth_method: String,
    pub session_id: String,
    pub api_key_id: String,
    pub permission_fingerprint: String,
    pub replay_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionResolution {
    pub actor: AuthActor,
    pub session: SessionRecord,
    pub trace: AuthTraceContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyResolution {
    pub actor: AuthActor,
    pub api_key: ApiKeyRecord,
    pub trace: AuthTraceContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtVerificationContract {
    pub issuer: String,
    pub audience: String,
    pub jwks_url: String,
    pub algorithm: String,
    pub required_tenant_claim: String,
    pub required_subject_claim: String,
    pub clock_skew_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JwtContractDiagnostic {
    pub valid: bool,
    pub failure_kind: Option<String>,
    pub redacted: bool,
}

/// Why an OAuth flow was started, kept structural so a callback can
/// never confuse a first-time login for an account link (52e).
///
/// - `Login` — a login (first-time or returning): there is NO bound
///   actor at authorize time; the actor is determined at callback from
///   the verified `(issuer, subject)` and, if unknown, the identity's
///   declared first-login provisioning policy.
/// - `Link` — account linking (51i): the state is bound to the
///   ALREADY-authenticated actor, which the callback re-issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthStatePurpose {
    Login,
    Link,
}

impl OAuthStatePurpose {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Link => "link",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "login" => Some(Self::Login),
            "link" => Some(Self::Link),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStateCreate {
    pub id: String,
    pub provider: String,
    pub tenant_id: String,
    /// `None` for a `Login` flow (actor unknown until callback), `Some`
    /// for a `Link` flow (the already-authenticated actor).
    pub actor_id: Option<String>,
    pub purpose: OAuthStatePurpose,
    pub raw_state: String,
    pub pkce_verifier_ref: String,
    pub nonce_fingerprint: String,
    pub expires_ms: u64,
    pub replay_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStateRecord {
    pub id: String,
    pub provider: String,
    pub tenant_id: String,
    /// `None` for a `Login` flow, `Some` for a `Link` flow — see
    /// [`OAuthStatePurpose`].
    pub actor_id: Option<String>,
    pub purpose: OAuthStatePurpose,
    pub state_hash: String,
    pub pkce_verifier_ref: String,
    pub nonce_fingerprint: String,
    pub expires_ms: u64,
    pub replay_key: String,
    pub used_ms: Option<u64>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackResolution {
    /// The bound actor for a `Link` flow; `None` for a `Login` flow,
    /// whose actor the caller resolves from the verified subject.
    pub actor: Option<AuthActor>,
    pub state: OAuthStateRecord,
    pub trace: AuthTraceContext,
}

/// A durable mapping from an external OAuth identity — keyed by the
/// ID token's `(issuer, subject)`, never by email — to a Corvid actor
/// (52e). Written when a login provisions or links an actor; read at
/// callback to recognise a returning subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentityLink {
    pub provider: String,
    pub issuer: String,
    pub subject: String,
    pub actor_id: String,
    pub tenant_id: String,
    pub created_ms: u64,
}

/// An operator-created invitation that gates `first_login: invited`
/// provisioning (52e). Matched at callback against the verified email
/// claim — which authorises provisioning a NEW actor, and is NOT the
/// same as identifying or merging an existing account by email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub id: String,
    pub email: String,
    pub tenant_id: String,
    /// Optional role grant applied to the provisioned actor; empty means
    /// no role.
    pub role_fingerprint: String,
    pub expires_ms: Option<u64>,
    pub consumed_ms: Option<u64>,
    pub created_ms: u64,
}

/// Fields to create an [`Invitation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitationCreate {
    pub id: String,
    pub email: String,
    pub tenant_id: String,
    pub role_fingerprint: String,
    pub expires_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequirement {
    pub tenant_id: String,
    pub permission: String,
    pub permission_fingerprint: String,
    pub surface_kind: String,
    pub surface_id: String,
    pub trace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub actor_id: String,
    pub tenant_id: String,
    pub permission: String,
    pub surface_kind: String,
    pub surface_id: String,
    pub trace_id: String,
    pub reason: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthAuditEvent {
    pub id: String,
    pub event_kind: String,
    pub actor_id: Option<String>,
    pub tenant_id: Option<String>,
    pub session_id: Option<String>,
    pub api_key_id: Option<String>,
    pub trace_id: Option<String>,
    pub status: String,
    pub reason: String,
    pub created_ms: u64,
}
