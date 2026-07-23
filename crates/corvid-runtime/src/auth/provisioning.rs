//! First-login provisioning executor (52e) — the security decision a
//! login callback runs AFTER it has a verified subject.
//!
//! Given the verified ID-token identity `(issuer, subject)` and the
//! identity block's declared provisioning policy, this decides which
//! actor a login becomes:
//!
//! 1. **Recognise** — if `(issuer, subject)` already maps to an actor,
//!    that actor is returned. No policy runs; a returning user always
//!    lands on the same actor.
//! 2. **Provision** — otherwise the declared `first_login` policy runs:
//!    - `open` auto-provisions a new actor.
//!    - `invited` provisions only if the verified email matches an
//!      operator-created invitation, which it then consumes.
//!    The actor's tenant comes from the declared `tenant` source — fixed
//!    config, the matched invitation, or an allowlisted issuer claim —
//!    NEVER a bare, caller-controlled claim.
//!
//! The new actor is bound to `(issuer, subject)` via
//! `link_external_identity`, so the next login recognises it at step 1.
//! A session is issued by the caller only after this returns Ok.

use sha2::{Digest, Sha256};

use super::{AuthActor, Invitation, SessionAuthRuntime};
use crate::errors::RuntimeError;

/// The executable first-login policies. `approval_required` is a compile
/// error until the durable-approval runtime exists (the checker rejects
/// it), so it is deliberately not representable here — the runtime only
/// ever offers what it can execute completely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstLoginPolicy {
    Open,
    Invited,
}

/// Where a newly provisioned actor's tenant comes from. Never a bare,
/// caller-controlled claim — only fixed config, a verified invitation,
/// or an issuer claim constrained to an allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TenantSource {
    Fixed(String),
    FromInvitation,
    Claim {
        claim: String,
        allowlist: Vec<String>,
    },
}

/// One login's provisioning inputs. The caller (serve) is responsible
/// for populating `verified_email` / `tenant_claim_value` ONLY from
/// claims it has verified on the ID token.
#[derive(Debug, Clone)]
pub struct ProvisioningRequest<'a> {
    pub provider: &'a str,
    pub issuer: &'a str,
    pub subject: &'a str,
    /// The verified `email` claim, if present and verified. Required for
    /// `invited`; used only to match an invitation, never to identify an
    /// existing account.
    pub verified_email: Option<&'a str>,
    /// The verified value of the configured tenant claim, if the tenant
    /// source is `Claim`.
    pub tenant_claim_value: Option<&'a str>,
    /// A display name from the verified profile, if any.
    pub display_name: Option<&'a str>,
    pub first_login: FirstLoginPolicy,
    pub tenant: TenantSource,
    pub trace_id: &'a str,
    pub at_ms: u64,
}

/// The outcome of provisioning a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisioningOutcome {
    pub actor: AuthActor,
    /// `true` if a new actor was created, `false` if an existing external
    /// identity was recognised.
    pub provisioned: bool,
}

/// A deterministic actor id derived from the external identity, so a
/// racing double-callback upserts the same actor rather than creating
/// two.
fn derive_actor_id(issuer: &str, subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"corvid-auth-actor-v1:");
    hasher.update(issuer.as_bytes());
    hasher.update([0u8]);
    hasher.update(subject.as_bytes());
    format!("actor:{:x}", hasher.finalize())
}

impl SessionAuthRuntime {
    /// Recognise or provision the actor for a verified login. See the
    /// module docs for the decision order.
    pub fn provision_login(
        &self,
        req: ProvisioningRequest<'_>,
    ) -> Result<ProvisioningOutcome, RuntimeError> {
        if req.issuer.trim().is_empty() || req.subject.trim().is_empty() {
            return Err(RuntimeError::Other(
                "provisioning requires a non-empty verified (issuer, subject)".to_string(),
            ));
        }

        // 1. Recognise a returning subject — no policy runs.
        if let Some(actor) = self.find_actor_by_external_identity(req.issuer, req.subject)? {
            self.audit_provision(&req, Some(&actor.id), "allowed", "recognised existing identity")?;
            return Ok(ProvisioningOutcome {
                actor,
                provisioned: false,
            });
        }

        // 2. Provision under the declared policy. `invited` gates on an
        // invitation before anything is written.
        let invitation = match req.first_login {
            FirstLoginPolicy::Open => None,
            FirstLoginPolicy::Invited => {
                let email = req.verified_email.ok_or_else(|| {
                    self.audit_provision_denied(&req, "invited login without a verified email");
                    RuntimeError::Other(
                        "provisioning denied: `invited` requires a verified email".to_string(),
                    )
                })?;
                match self.find_open_invitation_by_email(email, req.at_ms)? {
                    Some(inv) => Some(inv),
                    None => {
                        self.audit_provision_denied(&req, "no open invitation for verified email");
                        return Err(RuntimeError::Other(
                            "provisioning denied: no open invitation for this subject".to_string(),
                        ));
                    }
                }
            }
        };

        let tenant_id = self.resolve_tenant(&req, invitation.as_ref())?;
        let role_fingerprint = invitation
            .as_ref()
            .map(|inv| inv.role_fingerprint.clone())
            .unwrap_or_default();

        let actor_id = derive_actor_id(req.issuer, req.subject);
        let display_name = req
            .display_name
            .map(str::to_string)
            .unwrap_or_else(|| req.subject.to_string());
        let actor = self.upsert_actor(AuthActor {
            id: actor_id.clone(),
            tenant_id: tenant_id.clone(),
            display_name,
            actor_kind: "user".to_string(),
            auth_method: "oauth".to_string(),
            assurance_level: "aal1".to_string(),
            role_fingerprint,
            permission_fingerprint: String::new(),
            created_ms: 0,
            updated_ms: 0,
        })?;

        // Bind the identity so the next login recognises it, then consume
        // the invitation (after the actor exists, so a failed link never
        // burns an invitation).
        self.link_external_identity(req.provider, req.issuer, req.subject, &actor_id, &tenant_id)?;
        if let Some(inv) = &invitation {
            self.consume_invitation(&inv.id, req.at_ms)?;
        }

        self.audit_provision(&req, Some(&actor.id), "allowed", "provisioned new actor")?;
        Ok(ProvisioningOutcome {
            actor,
            provisioned: true,
        })
    }

    fn resolve_tenant(
        &self,
        req: &ProvisioningRequest<'_>,
        invitation: Option<&Invitation>,
    ) -> Result<String, RuntimeError> {
        match &req.tenant {
            TenantSource::Fixed(id) => {
                if id.trim().is_empty() {
                    return Err(RuntimeError::Other(
                        "provisioning denied: fixed tenant is empty".to_string(),
                    ));
                }
                Ok(id.clone())
            }
            TenantSource::FromInvitation => invitation.map(|inv| inv.tenant_id.clone()).ok_or_else(
                || {
                    self.audit_provision_denied(req, "tenant from_invitation without an invitation");
                    RuntimeError::Other(
                        "provisioning denied: `tenant: from_invitation` requires a matched invitation"
                            .to_string(),
                    )
                },
            ),
            TenantSource::Claim { claim, allowlist } => {
                let value = req.tenant_claim_value.ok_or_else(|| {
                    self.audit_provision_denied(req, "missing configured tenant claim");
                    RuntimeError::Other(format!(
                        "provisioning denied: verified claim `{claim}` is required for the tenant"
                    ))
                })?;
                if !allowlist.iter().any(|allowed| allowed == value) {
                    self.audit_provision_denied(req, "tenant claim value not in allowlist");
                    return Err(RuntimeError::Other(format!(
                        "provisioning denied: tenant claim `{claim}` value is not in the allowlist"
                    )));
                }
                Ok(value.to_string())
            }
        }
    }

    fn audit_provision(
        &self,
        req: &ProvisioningRequest<'_>,
        actor_id: Option<&str>,
        status: &str,
        reason: &str,
    ) -> Result<(), RuntimeError> {
        self.insert_audit(
            "oauth.provision",
            actor_id,
            None,
            None,
            None,
            Some(req.trace_id),
            status,
            reason,
        )
    }

    /// Best-effort denial audit — the caller still returns the error even
    /// if the audit write itself fails, so a failed audit never masks a
    /// security denial.
    fn audit_provision_denied(&self, req: &ProvisioningRequest<'_>, reason: &str) {
        let _ = self.audit_provision(req, None, "denied", reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{InvitationCreate, SessionAuthRuntime};
    use crate::tracing::now_ms;

    fn req<'a>(
        trace_id: &'a str,
        issuer: &'a str,
        subject: &'a str,
        first_login: FirstLoginPolicy,
        tenant: TenantSource,
    ) -> ProvisioningRequest<'a> {
        // Each login carries its own trace id, as serve issues one per
        // callback; distinct traces give distinct audit-event ids.
        ProvisioningRequest {
            provider: "google",
            issuer,
            subject,
            verified_email: None,
            tenant_claim_value: None,
            display_name: None,
            first_login,
            tenant,
            trace_id,
            at_ms: now_ms(),
        }
    }

    #[test]
    fn open_provisions_a_new_actor_and_recognises_it_next_time() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let iss = "https://accounts.google.com";
        let first = auth
            .provision_login(req("login-a", iss, "sub-1", FirstLoginPolicy::Open, TenantSource::Fixed("public".into())))
            .unwrap();
        assert!(first.provisioned);
        assert_eq!(first.actor.tenant_id, "public");
        assert_eq!(first.actor.auth_method, "oauth");

        // Second login with the same subject is recognised, not
        // re-provisioned, and lands on the same actor.
        let second = auth
            .provision_login(req("login-b", iss, "sub-1", FirstLoginPolicy::Open, TenantSource::Fixed("public".into())))
            .unwrap();
        assert!(!second.provisioned);
        assert_eq!(second.actor.id, first.actor.id);
    }

    #[test]
    fn invited_requires_a_matching_invitation() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let iss = "https://accounts.google.com";
        let now = now_ms();

        // No invitation → refused.
        let mut r = req("login-a", iss, "sub-1", FirstLoginPolicy::Invited, TenantSource::FromInvitation);
        r.verified_email = Some("ada@example.com");
        r.at_ms = now;
        let err = auth.provision_login(r).unwrap_err();
        assert!(err.to_string().contains("no open invitation"));

        // With an invitation, the tenant comes from it and it is consumed.
        auth.create_invitation(InvitationCreate {
            id: "inv-1".to_string(),
            email: "ada@example.com".to_string(),
            tenant_id: "acme".to_string(),
            role_fingerprint: "sha256:member".to_string(),
            expires_ms: Some(now + 60_000),
        })
        .unwrap();
        let mut r = req("login-b", iss, "sub-1", FirstLoginPolicy::Invited, TenantSource::FromInvitation);
        r.verified_email = Some("ada@example.com");
        r.at_ms = now;
        let outcome = auth.provision_login(r).unwrap();
        assert!(outcome.provisioned);
        assert_eq!(outcome.actor.tenant_id, "acme");
        assert_eq!(outcome.actor.role_fingerprint, "sha256:member");
        // The invitation is now consumed — a different subject cannot reuse it.
        assert!(auth
            .find_open_invitation_by_email("ada@example.com", now)
            .unwrap()
            .is_none());
    }

    #[test]
    fn invited_without_a_verified_email_is_refused() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let err = auth
            .provision_login(req(
                "login-a",
                "iss",
                "sub-1",
                FirstLoginPolicy::Invited,
                TenantSource::FromInvitation,
            ))
            .unwrap_err();
        assert!(err.to_string().contains("requires a verified email"));
    }

    #[test]
    fn claim_tenant_must_be_in_the_allowlist() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let tenant = TenantSource::Claim {
            claim: "org_id".to_string(),
            allowlist: vec!["acme".to_string(), "globex".to_string()],
        };

        // A value outside the allowlist is refused.
        let mut r = req("login-a", "iss", "sub-1", FirstLoginPolicy::Open, tenant.clone());
        r.tenant_claim_value = Some("evilcorp");
        assert!(auth.provision_login(r).unwrap_err().to_string().contains("allowlist"));

        // An allowlisted value provisions into that tenant.
        let mut r = req("login-b", "iss", "sub-2", FirstLoginPolicy::Open, tenant);
        r.tenant_claim_value = Some("acme");
        let outcome = auth.provision_login(r).unwrap();
        assert_eq!(outcome.actor.tenant_id, "acme");
    }

    #[test]
    fn missing_tenant_claim_is_refused() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let tenant = TenantSource::Claim {
            claim: "org_id".to_string(),
            allowlist: vec!["acme".to_string()],
        };
        // tenant_claim_value left None.
        let err = auth
            .provision_login(req("login-a", "iss", "sub-1", FirstLoginPolicy::Open, tenant))
            .unwrap_err();
        assert!(err.to_string().contains("required for the tenant"));
    }
}
