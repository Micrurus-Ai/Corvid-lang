//! Delegation grants (B11): a signed-in person grants an assistant named
//! actions for a bounded time, and the assistant presents the grant's
//! bearer token on its calls.
//!
//! A grant authorizes nothing by itself. A caller checks each call
//! against the intersection of what the grant names, what the source
//! offers to assistants, and the person's current authority; this store
//! keeps the first and says whether the grant is still live. Tokens are
//! stored only as a hash, under a prefix of their own so a session token
//! can never be presented as a grant or the reverse.

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::errors::RuntimeError;

use super::{validate_non_empty, SessionAuthRuntime};

/// How an assistant may call one granted action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegationMode {
    /// Each call waits for the person's confirmation.
    Confirm,
    /// Calls run without confirmation, when the action itself allows it.
    Allow,
}

impl DelegationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DelegationMode::Confirm => "confirm",
            DelegationMode::Allow => "allow",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "confirm" => Some(DelegationMode::Confirm),
            "allow" => Some(DelegationMode::Allow),
            _ => None,
        }
    }
}

/// One action a grant covers, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedAction {
    pub action: String,
    pub mode: DelegationMode,
}

/// A grant to record. `max_lifetime_ms` is the program's declared cap
/// (`delegation: up_to ...`); a grant asking for longer is refused.
#[derive(Debug, Clone)]
pub struct DelegationCreate {
    pub id: String,
    pub actor_id: String,
    pub tenant_id: String,
    pub label: String,
    pub actions: Vec<DelegatedAction>,
    pub raw_token: String,
    pub issued_ms: u64,
    pub expires_ms: u64,
    pub max_lifetime_ms: u64,
}

/// A recorded grant. The token itself is never kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationRecord {
    pub id: String,
    pub actor_id: String,
    pub tenant_id: String,
    pub label: String,
    pub actions: Vec<DelegatedAction>,
    pub issued_ms: u64,
    pub expires_ms: u64,
    pub revoked_ms: Option<u64>,
}

impl DelegationRecord {
    /// Whether the grant still authorizes anything at `now_ms`.
    pub fn is_live(&self, now_ms: u64) -> bool {
        self.revoked_ms.is_none() && now_ms < self.expires_ms
    }

    /// How the grant lets the assistant call `action`, if it covers it.
    pub fn mode_for(&self, action: &str) -> Option<DelegationMode> {
        self.actions.iter().find(|granted| granted.action == action).map(|granted| granted.mode)
    }
}

pub fn hash_delegation_secret(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"corvid-auth-delegation-v1:");
    hasher.update(raw_token.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn storage_error(error: rusqlite::Error) -> RuntimeError {
    RuntimeError::Other(format!("delegation store failed: {error}"))
}

pub(super) fn initialize(conn: &Connection) -> Result<(), RuntimeError> {
    conn.execute_batch(
        "create table if not exists auth_delegations (
            id text primary key,
            actor_id text not null,
            tenant_id text not null,
            label text not null,
            actions text not null,
            token_hash text not null unique,
            issued_ms integer not null,
            expires_ms integer not null,
            revoked_ms integer
        );
        create index if not exists auth_delegations_actor on auth_delegations(actor_id);",
    )
    .map_err(storage_error)
}

/// `move_card=confirm,tag_card=allow`: action names are identifiers, so
/// neither separator can occur in one.
fn encode_actions(actions: &[DelegatedAction]) -> String {
    actions.iter().map(|a| format!("{}={}", a.action, a.mode.as_str())).collect::<Vec<_>>().join(",")
}

fn decode_actions(text: &str) -> Vec<DelegatedAction> {
    text.split(',')
        .filter_map(|pair| {
            let (action, mode) = pair.split_once('=')?;
            Some(DelegatedAction { action: action.to_string(), mode: DelegationMode::parse(mode)? })
        })
        .collect()
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DelegationRecord> {
    Ok(DelegationRecord {
        id: row.get(0)?,
        actor_id: row.get(1)?,
        tenant_id: row.get(2)?,
        label: row.get(3)?,
        actions: decode_actions(&row.get::<_, String>(4)?),
        issued_ms: row.get::<_, i64>(5)? as u64,
        expires_ms: row.get::<_, i64>(6)? as u64,
        revoked_ms: row.get::<_, Option<i64>>(7)?.map(|ms| ms as u64),
    })
}

const COLUMNS: &str = "id, actor_id, tenant_id, label, actions, issued_ms, expires_ms, revoked_ms";

impl SessionAuthRuntime {
    /// Record a grant for a signed-in person. Refuses an empty grant, an
    /// action named twice, a lifetime beyond the program's cap, and a
    /// person or tenant the store does not hold.
    pub fn create_delegation(&self, input: DelegationCreate) -> Result<DelegationRecord, RuntimeError> {
        validate_non_empty("delegation id", &input.id)?;
        validate_non_empty("delegation label", &input.label)?;
        validate_non_empty("delegation token", &input.raw_token)?;
        if input.actions.is_empty() {
            return Err(RuntimeError::Other("a delegation must name at least one action".into()));
        }
        for (index, granted) in input.actions.iter().enumerate() {
            validate_non_empty("delegated action", &granted.action)?;
            if input.actions[..index].iter().any(|earlier| earlier.action == granted.action) {
                return Err(RuntimeError::Other(format!("action `{}` is named twice in one delegation", granted.action)));
            }
        }
        if input.expires_ms <= input.issued_ms {
            return Err(RuntimeError::Other("a delegation must expire after it is issued".into()));
        }
        if input.expires_ms - input.issued_ms > input.max_lifetime_ms {
            return Err(RuntimeError::Other(format!(
                "a delegation may last at most {} ms, the program's declared cap",
                input.max_lifetime_ms
            )));
        }
        let actor = self
            .get_actor(&input.actor_id)?
            .ok_or_else(|| RuntimeError::Other(format!("auth actor `{}` not found", input.actor_id)))?;
        if actor.tenant_id != input.tenant_id {
            return Err(RuntimeError::Other("delegation actor tenant mismatch".into()));
        }
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into auth_delegations
                 (id, actor_id, tenant_id, label, actions, token_hash, issued_ms, expires_ms, revoked_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, null)",
                params![
                    input.id,
                    input.actor_id,
                    input.tenant_id,
                    input.label,
                    encode_actions(&input.actions),
                    hash_delegation_secret(&input.raw_token),
                    input.issued_ms as i64,
                    input.expires_ms as i64,
                ],
            )
            .map_err(storage_error)?;
        self.get_delegation(&input.id)?
            .ok_or_else(|| RuntimeError::Other(format!("delegation `{}` not found", input.id)))
    }

    pub fn get_delegation(&self, id: &str) -> Result<Option<DelegationRecord>, RuntimeError> {
        self.conn
            .lock()
            .unwrap()
            .query_row(&format!("select {COLUMNS} from auth_delegations where id = ?1"), params![id], read_row)
            .optional()
            .map_err(storage_error)
    }

    /// The live grant a bearer token presents, or `None` for an unknown,
    /// revoked or expired token, or one whose person is gone or moved
    /// tenant. Callers still check the action against the source and the
    /// person's current authority.
    pub fn resolve_delegation(&self, raw_token: &str, now_ms: u64) -> Result<Option<DelegationRecord>, RuntimeError> {
        let record = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                &format!("select {COLUMNS} from auth_delegations where token_hash = ?1"),
                params![hash_delegation_secret(raw_token)],
                read_row,
            )
            .optional()
            .map_err(storage_error)?;
        let Some(record) = record.filter(|record| record.is_live(now_ms)) else {
            return Ok(None);
        };
        let same_person = self.get_actor(&record.actor_id)?.is_some_and(|actor| actor.tenant_id == record.tenant_id);
        Ok(same_person.then_some(record))
    }

    /// A person's grants, newest first, revoked and expired ones included
    /// so the person can see what they gave.
    pub fn list_delegations(&self, actor_id: &str) -> Result<Vec<DelegationRecord>, RuntimeError> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn
            .prepare(&format!(
                "select {COLUMNS} from auth_delegations where actor_id = ?1 order by issued_ms desc, id"
            ))
            .map_err(storage_error)?;
        let rows = statement.query_map(params![actor_id], read_row).map_err(storage_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(storage_error)
    }

    /// Revoke a grant. Only the person who made it may; `false` when the
    /// grant is not theirs or is already revoked.
    pub fn revoke_delegation(&self, id: &str, actor_id: &str, now_ms: u64) -> Result<bool, RuntimeError> {
        let changed = self
            .conn
            .lock()
            .unwrap()
            .execute(
                "update auth_delegations set revoked_ms = ?3
                 where id = ?1 and actor_id = ?2 and revoked_ms is null",
                params![id, actor_id, now_ms as i64],
            )
            .map_err(storage_error)?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthActor;

    const HOUR: u64 = 60 * 60 * 1000;

    fn store_with(actor: &str, tenant: &str) -> SessionAuthRuntime {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(AuthActor {
            id: actor.into(),
            tenant_id: tenant.into(),
            display_name: actor.into(),
            actor_kind: "user".into(),
            auth_method: "oauth".into(),
            assurance_level: "aal1".into(),
            role_fingerprint: String::new(),
            permission_fingerprint: String::new(),
            created_ms: 0,
            updated_ms: 0,
        })
        .unwrap();
        auth
    }

    fn grant(token: &str, issued: u64, expires: u64) -> DelegationCreate {
        DelegationCreate {
            id: format!("grant-{token}"),
            actor_id: "person-1".into(),
            tenant_id: "public".into(),
            label: "Desk assistant".into(),
            actions: vec![
                DelegatedAction { action: "move_card".into(), mode: DelegationMode::Confirm },
                DelegatedAction { action: "tag_card".into(), mode: DelegationMode::Allow },
            ],
            raw_token: token.into(),
            issued_ms: issued,
            expires_ms: expires,
            max_lifetime_ms: 8 * HOUR,
        }
    }

    #[test]
    fn a_live_grant_resolves_from_its_token_and_names_its_actions() {
        let auth = store_with("person-1", "public");
        auth.create_delegation(grant("secret-a", 1_000, 1_000 + HOUR)).unwrap();
        let record = auth.resolve_delegation("secret-a", 2_000).unwrap().expect("a live grant");
        assert_eq!(record.mode_for("tag_card"), Some(DelegationMode::Allow));
        assert_eq!(record.mode_for("wipe_board"), None, "a grant covers only what it names");
        assert!(auth.resolve_delegation("secret-b", 2_000).unwrap().is_none(), "an unknown token is no grant");
        assert!(
            auth.resolve_delegation(&hash_delegation_secret("secret-a"), 2_000).unwrap().is_none(),
            "the stored hash is not a token"
        );
    }

    #[test]
    fn expiry_revocation_and_the_cap_end_or_refuse_a_grant() {
        let auth = store_with("person-1", "public");
        auth.create_delegation(grant("secret-a", 1_000, 1_000 + HOUR)).unwrap();
        assert!(auth.resolve_delegation("secret-a", 1_000 + HOUR).unwrap().is_none(), "expired at its expiry");

        auth.create_delegation(grant("secret-b", 1_000, 1_000 + HOUR)).unwrap();
        assert!(!auth.revoke_delegation("grant-secret-b", "someone-else", 2_000).unwrap(), "only its person revokes it");
        assert!(auth.revoke_delegation("grant-secret-b", "person-1", 2_000).unwrap());
        assert!(auth.resolve_delegation("secret-b", 2_500).unwrap().is_none(), "revoked at once");
        assert!(!auth.revoke_delegation("grant-secret-b", "person-1", 3_000).unwrap(), "revoked once");

        let too_long = auth.create_delegation(grant("secret-c", 0, 9 * HOUR)).unwrap_err();
        assert!(too_long.to_string().contains("declared cap"), "{too_long}");
        let listed = auth.list_delegations("person-1").unwrap();
        assert_eq!(listed.len(), 2, "the person sees every grant they made");
    }

    #[test]
    fn a_grant_needs_a_known_person_in_their_tenant_and_distinct_actions() {
        let auth = store_with("person-1", "public");
        let mut other_tenant = grant("secret-a", 0, HOUR);
        other_tenant.tenant_id = "elsewhere".into();
        assert!(auth.create_delegation(other_tenant).is_err());
        let mut twice = grant("secret-b", 0, HOUR);
        twice.actions.push(DelegatedAction { action: "move_card".into(), mode: DelegationMode::Allow });
        assert!(auth.create_delegation(twice).unwrap_err().to_string().contains("named twice"));
        let mut empty = grant("secret-c", 0, HOUR);
        empty.actions.clear();
        assert!(auth.create_delegation(empty).is_err());
    }
}
