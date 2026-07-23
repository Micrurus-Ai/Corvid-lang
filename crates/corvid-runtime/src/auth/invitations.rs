//! Invitation storage — the operator-created artifacts that gate
//! `first_login: invited` provisioning (52e).
//!
//! An invitation is created BEFORE the invited person ever logs in, so
//! it is keyed by the email the operator sent it to. At callback the
//! provisioning executor matches an invitation against the *verified*
//! email claim and consumes it. Matching a verified email to an explicit
//! invitation authorises provisioning a NEW actor — it is deliberately
//! NOT a lookup of an existing account by email (Corvid never identifies
//! or merges accounts by email; see `auth_external_identities`).

use rusqlite::{params, OptionalExtension};

use super::{validate_non_empty, Invitation, InvitationCreate, SessionAuthRuntime};
use crate::errors::RuntimeError;
use crate::tracing::now_ms;

const INVITATION_COLUMNS: &str =
    "id, email, tenant_id, role_fingerprint, expires_ms, consumed_ms, created_ms";

fn read_invitation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Invitation> {
    Ok(Invitation {
        id: row.get(0)?,
        email: row.get(1)?,
        tenant_id: row.get(2)?,
        role_fingerprint: row.get(3)?,
        expires_ms: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
        consumed_ms: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        created_ms: row.get::<_, i64>(6)? as u64,
    })
}

/// Normalise an email for matching: trim and lowercase. An invitation
/// and a token claim must match case-insensitively, since email casing
/// is not significant to the identity but IdPs are inconsistent about it.
pub(super) fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

impl SessionAuthRuntime {
    pub fn create_invitation(
        &self,
        input: InvitationCreate,
    ) -> Result<Invitation, RuntimeError> {
        validate_non_empty("invitation id", &input.id)?;
        validate_non_empty("invitation email", &input.email)?;
        validate_non_empty("tenant id", &input.tenant_id)?;
        let now = now_ms();
        if let Some(expires_ms) = input.expires_ms {
            if expires_ms <= now {
                return Err(RuntimeError::Other(
                    "invitation expiry must be in the future".to_string(),
                ));
            }
        }
        let email = normalize_email(&input.email);
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_invitations
                 (id, email, tenant_id, role_fingerprint, expires_ms, consumed_ms, created_ms)
                 values (?1, ?2, ?3, ?4, ?5, null, ?6)",
                params![
                    input.id,
                    email,
                    input.tenant_id,
                    input.role_fingerprint,
                    input.expires_ms.map(|v| v as i64),
                    now as i64,
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to create invitation: {err}")))?;
        self.get_invitation(&input.id)?
            .ok_or_else(|| RuntimeError::Other(format!("invitation `{}` not found", input.id)))
    }

    pub fn get_invitation(&self, id: &str) -> Result<Option<Invitation>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                &format!("select {INVITATION_COLUMNS} from auth_invitations where id = ?1"),
                params![id],
                read_invitation_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read invitation: {err}")))
    }

    /// Find an unconsumed, unexpired invitation for `email` (matched
    /// case-insensitively). Returns the oldest match so a re-sent
    /// invitation does not shadow the original.
    pub fn find_open_invitation_by_email(
        &self,
        email: &str,
        at_ms: u64,
    ) -> Result<Option<Invitation>, RuntimeError> {
        validate_non_empty("invitation email", email)?;
        let email = normalize_email(email);
        self.conn
            .lock()
            .unwrap()
            .query_row(
                &format!(
                    "select {INVITATION_COLUMNS} from auth_invitations
                     where email = ?1 and consumed_ms is null
                       and (expires_ms is null or expires_ms > ?2)
                     order by created_ms asc limit 1"
                ),
                params![email, at_ms as i64],
                read_invitation_row,
            )
            .optional()
            .map_err(|err| RuntimeError::Other(format!("failed to read invitation: {err}")))
    }

    /// Mark an invitation consumed. The conditional `where ... and
    /// consumed_ms is null` makes it single-use even under a race — a
    /// second consume affects zero rows and is reported as an error.
    pub fn consume_invitation(&self, id: &str, at_ms: u64) -> Result<(), RuntimeError> {
        validate_non_empty("invitation id", id)?;
        let affected = self
            .conn
            .lock()
            .unwrap()
            .execute(
                "update auth_invitations set consumed_ms = ?2
                 where id = ?1 and consumed_ms is null",
                params![id, at_ms as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to consume invitation: {err}")))?;
        if affected == 0 {
            return Err(RuntimeError::Other(format!(
                "invitation `{id}` is already consumed or does not exist"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_invitation_matches_email_case_insensitively_and_is_single_use() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let now = now_ms();
        auth.create_invitation(InvitationCreate {
            id: "inv-1".to_string(),
            email: "Ada@Example.com".to_string(),
            tenant_id: "org-1".to_string(),
            role_fingerprint: "sha256:admin".to_string(),
            expires_ms: Some(now + 60_000),
        })
        .unwrap();

        // Case-insensitive match.
        let found = auth
            .find_open_invitation_by_email("ada@example.com", now)
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "inv-1");
        assert_eq!(found.tenant_id, "org-1");

        auth.consume_invitation("inv-1", now).unwrap();
        // Consumed → no longer open.
        assert!(auth
            .find_open_invitation_by_email("ada@example.com", now)
            .unwrap()
            .is_none());
        // Double-consume is rejected.
        assert!(auth.consume_invitation("inv-1", now).is_err());
    }

    #[test]
    fn expired_invitation_is_not_open() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let now = now_ms();
        auth.create_invitation(InvitationCreate {
            id: "inv-1".to_string(),
            email: "ada@example.com".to_string(),
            tenant_id: "org-1".to_string(),
            role_fingerprint: String::new(),
            expires_ms: Some(now + 1_000),
        })
        .unwrap();
        // At a time past expiry, it is not returned.
        assert!(auth
            .find_open_invitation_by_email("ada@example.com", now + 2_000)
            .unwrap()
            .is_none());
    }

    #[test]
    fn create_invitation_rejects_past_expiry() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        let now = now_ms();
        let err = auth
            .create_invitation(InvitationCreate {
                id: "inv-1".to_string(),
                email: "ada@example.com".to_string(),
                tenant_id: "org-1".to_string(),
                role_fingerprint: String::new(),
                expires_ms: Some(now.saturating_sub(1_000)),
            })
            .unwrap_err();
        assert!(err.to_string().contains("expiry must be in the future"));
    }
}
