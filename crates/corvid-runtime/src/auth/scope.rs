//! Structured API-key scope model + subset-check enforcement.
//!
//! `ApiKeyRecord::scope_fingerprint` stores an opaque hash today
//! — the runtime knows a key has *some* scope but cannot reason
//! about which permissions it carries. That is enough for
//! identity-and-tenancy checks but cannot catch the named
//! `scope-escalation` adversarial-corpus threat: a key issued
//! with `{orders.read}` being used to satisfy a required
//! `{refunds.write}` action would slip through because the
//! runtime never compares the two scope sets.
//!
//! This module ships the structured-scope concept the audit
//! named as the threat's dependency. An `ApiKeyScope` is an
//! immutable, lexicographically-sorted set of `Permission`
//! strings of shape `<resource>.<action>`; the
//! `enforce_scope_grant(granted, required)` predicate refuses
//! the call when the required set is not a subset of the
//! granted set, listing every missing permission in the typed
//! error so the audit trail records *exactly* which scope was
//! attempted.
//!
//! The canonical fingerprint is deterministic over the set
//! (independent of insertion order, free of duplicates), so a
//! `scope_fingerprint` written by this module can be compared
//! against an `ApiKeyRecord::scope_fingerprint` written
//! anywhere else without re-computing the source set.
//!
//! Wiring `enforce_scope_grant` into every middleware /
//! handler /  route surface is downstream work. Today the slice
//! ships the **model** (the typed set + the predicate + the
//! fingerprint) so the rest of the runtime can adopt it without
//! reinventing the shape.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// In-binary anchor for the `phase 35V-T1-Drift` inverse-
/// coverage sentinel. Names the registry id whose runtime
/// enforcement lives in `enforce_scope_grant` below.
#[allow(dead_code)]
pub const GUARANTEE_ID_API_KEY_SCOPE_SUBSET_CHECK: &str = "auth.api_key_scope_subset_check";

const SCOPE_FINGERPRINT_DOMAIN: &[u8] = b"corvid-api-key-scope-v1\n";

/// Structured API-key scope: an immutable, deduplicated,
/// lexicographically-sorted set of `<resource>.<action>`
/// permission strings.
///
/// Two scopes constructed from the same underlying set produce
/// identical canonical fingerprints regardless of insertion
/// order, so callers can persist the fingerprint and recompare
/// it later without preserving the source iteration order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ApiKeyScope {
    permissions: BTreeSet<String>,
}

/// Errors surfaced by scope construction or enforcement. Every
/// variant maps to a distinct audit-log reason so an operator
/// can attribute a rejection precisely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// A permission string was empty after trimming.
    EmptyPermission,
    /// A permission contained characters outside the allowed
    /// set (`[A-Za-z0-9_.-]`) or was missing the `.` separator
    /// between resource and action.
    MalformedPermission(String),
    /// The granted scope is not a superset of the required
    /// scope. `missing` names every permission the required
    /// scope demanded but the granted scope did not carry —
    /// the central evidence shape for the `scope-escalation`
    /// adversarial-corpus threat.
    EscalationAttempt { missing: Vec<String> },
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPermission => f.write_str("api-key scope permission must not be empty"),
            Self::MalformedPermission(p) => write!(
                f,
                "api-key scope permission `{p}` is malformed; expected \
                 `<resource>.<action>` with chars [A-Za-z0-9_.-]"
            ),
            Self::EscalationAttempt { missing } => write!(
                f,
                "api-key scope-escalation refused: granted scope does not cover \
                 required permission(s) [{}]",
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for ScopeError {}

impl ApiKeyScope {
    /// Empty scope — grants nothing. Useful as the "required"
    /// scope on a no-privilege endpoint; never as a "granted"
    /// scope (would forbid the key from doing anything).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a scope from an iterator of permission strings,
    /// validating each one. Duplicates are silently collapsed
    /// (the underlying set is a `BTreeSet`).
    pub fn from_permissions<I, S>(items: I) -> Result<Self, ScopeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut permissions = BTreeSet::new();
        for raw in items {
            let permission = validate_permission(raw.as_ref())?;
            permissions.insert(permission);
        }
        Ok(Self { permissions })
    }

    /// Parse a comma-separated scope string like
    /// `"orders.read, refunds.write"`. Whitespace around each
    /// entry is trimmed; empty entries are an error (a stray
    /// trailing comma would otherwise silently produce an
    /// empty permission).
    pub fn parse_comma_separated(raw: &str) -> Result<Self, ScopeError> {
        Self::from_permissions(raw.split(','))
    }

    /// Borrow the lexicographically-sorted permission set.
    pub fn permissions(&self) -> impl Iterator<Item = &str> {
        self.permissions.iter().map(String::as_str)
    }

    /// True iff `other` carries every permission this scope
    /// requires. The named-threat predicate.
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.permissions.is_subset(&other.permissions)
    }

    /// Canonical fingerprint: `sha256:` + hex of
    /// SHA-256(domain || "\n".join(sorted permissions)).
    /// Stable across permission-insertion order and
    /// duplicate-removal. Safe to persist alongside
    /// `ApiKeyRecord::scope_fingerprint` for later equality
    /// checks.
    pub fn canonical_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(SCOPE_FINGERPRINT_DOMAIN);
        for permission in &self.permissions {
            hasher.update(permission.as_bytes());
            hasher.update(b"\n");
        }
        format!("sha256:{}", hex_encode(&hasher.finalize()))
    }
}

/// Enforce that `granted ⊇ required`. Returns the typed
/// `EscalationAttempt` error naming every missing permission
/// when the required scope is not covered — exactly the
/// `scope-escalation` named-threat shape.
///
/// An empty required scope always succeeds (the call site
/// declared "no privilege needed"); an empty granted scope
/// only succeeds against an empty required scope.
pub fn enforce_scope_grant(
    granted: &ApiKeyScope,
    required: &ApiKeyScope,
) -> Result<(), ScopeError> {
    if required.permissions.is_empty() {
        return Ok(());
    }
    let missing: Vec<String> = required
        .permissions
        .difference(&granted.permissions)
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ScopeError::EscalationAttempt { missing })
    }
}

fn validate_permission(raw: &str) -> Result<String, ScopeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ScopeError::EmptyPermission);
    }
    let Some((resource, action)) = trimmed.split_once('.') else {
        return Err(ScopeError::MalformedPermission(trimmed.to_string()));
    };
    if resource.is_empty() || action.is_empty() {
        return Err(ScopeError::MalformedPermission(trimmed.to_string()));
    }
    for byte in trimmed.bytes() {
        let ok = byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.');
        if !ok {
            return Err(ScopeError::MalformedPermission(trimmed.to_string()));
        }
    }
    Ok(trimmed.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(items: &[&str]) -> ApiKeyScope {
        ApiKeyScope::from_permissions(items.iter().copied()).expect("valid scope")
    }

    #[test]
    fn scope_with_subset_satisfies_required_grant() {
        let granted = scope(&["orders.read", "orders.write", "refunds.read"]);
        let required = scope(&["orders.read"]);
        enforce_scope_grant(&granted, &required).unwrap();
    }

    #[test]
    fn empty_required_scope_always_satisfied() {
        let granted = ApiKeyScope::empty();
        enforce_scope_grant(&granted, &ApiKeyScope::empty()).unwrap();
        let with_perms = scope(&["orders.read"]);
        enforce_scope_grant(&with_perms, &ApiKeyScope::empty()).unwrap();
    }

    /// Slice 35V2-P39-K-LR (named-threat: `scope-escalation`):
    /// an API key granted `{orders.read}` cannot satisfy a
    /// required `{refunds.write}` action. The runtime rejects
    /// the call AND surfaces the specific missing permission in
    /// the typed error so the audit trail records exactly which
    /// scope was attempted.
    #[test]
    fn scope_escalation_attempt_refused_with_specific_missing_permission() {
        let granted = scope(&["orders.read"]);
        let required = scope(&["refunds.write"]);
        let err = enforce_scope_grant(&granted, &required).unwrap_err();
        match err {
            ScopeError::EscalationAttempt { missing } => {
                assert_eq!(missing, vec!["refunds.write".to_string()]);
            }
            other => panic!("expected EscalationAttempt, got {other:?}"),
        }
    }

    /// Slice 35V2-P39-K-LR (adversarial, proper-subset shape):
    /// granted `{orders.read}` against required `{orders.read,
    /// orders.write}` is refused naming the one missing
    /// permission — escalation by "additional permission"
    /// rather than "different resource" is the same threat.
    #[test]
    fn scope_proper_subset_escalation_lists_only_missing_permissions() {
        let granted = scope(&["orders.read"]);
        let required = scope(&["orders.read", "orders.write"]);
        match enforce_scope_grant(&granted, &required).unwrap_err() {
            ScopeError::EscalationAttempt { missing } => {
                assert_eq!(missing, vec!["orders.write".to_string()]);
            }
            other => panic!("expected EscalationAttempt, got {other:?}"),
        }
    }

    /// Slice 35V2-P39-K-LR (adversarial): the typed error names
    /// *every* missing permission, not just the first. An
    /// auditor reading the rejection sees the full delta.
    #[test]
    fn scope_escalation_lists_every_missing_permission_not_just_the_first() {
        let granted = scope(&["orders.read"]);
        let required = scope(&["orders.write", "refunds.write", "audit.read"]);
        match enforce_scope_grant(&granted, &required).unwrap_err() {
            ScopeError::EscalationAttempt { missing } => {
                // BTreeSet difference yields sorted output.
                assert_eq!(
                    missing,
                    vec![
                        "audit.read".to_string(),
                        "orders.write".to_string(),
                        "refunds.write".to_string(),
                    ]
                );
            }
            other => panic!("expected EscalationAttempt, got {other:?}"),
        }
    }

    #[test]
    fn canonical_fingerprint_stable_across_permission_reordering() {
        let a = scope(&["orders.read", "refunds.write", "audit.read"]);
        let b = scope(&["audit.read", "orders.read", "refunds.write"]);
        let c = scope(&["refunds.write", "audit.read", "orders.read"]);
        assert_eq!(a.canonical_fingerprint(), b.canonical_fingerprint());
        assert_eq!(b.canonical_fingerprint(), c.canonical_fingerprint());
        // Distinct scope → distinct fingerprint.
        let d = scope(&["orders.read", "refunds.write"]);
        assert_ne!(a.canonical_fingerprint(), d.canonical_fingerprint());
    }

    #[test]
    fn canonical_fingerprint_collapses_duplicates_to_one_entry() {
        let with_dup = scope(&["orders.read", "orders.read", "orders.write"]);
        let no_dup = scope(&["orders.read", "orders.write"]);
        assert_eq!(with_dup.canonical_fingerprint(), no_dup.canonical_fingerprint());
        assert_eq!(
            with_dup.permissions().collect::<Vec<_>>(),
            vec!["orders.read", "orders.write"]
        );
    }

    #[test]
    fn parse_comma_separated_trims_and_validates() {
        let parsed = ApiKeyScope::parse_comma_separated(
            " orders.read , refunds.write ,orders.write",
        )
        .unwrap();
        assert_eq!(
            parsed.permissions().collect::<Vec<_>>(),
            vec!["orders.read", "orders.write", "refunds.write"]
        );
    }

    #[test]
    fn malformed_permissions_refused_with_typed_error() {
        // No `.` separator.
        let err = ApiKeyScope::from_permissions(["orders_read"]).unwrap_err();
        assert!(matches!(err, ScopeError::MalformedPermission(_)));
        // Empty resource.
        let err = ApiKeyScope::from_permissions([".write"]).unwrap_err();
        assert!(matches!(err, ScopeError::MalformedPermission(_)));
        // Empty action.
        let err = ApiKeyScope::from_permissions(["orders."]).unwrap_err();
        assert!(matches!(err, ScopeError::MalformedPermission(_)));
        // Disallowed character (whitespace).
        let err = ApiKeyScope::from_permissions(["orders read.write"]).unwrap_err();
        assert!(matches!(err, ScopeError::MalformedPermission(_)));
        // Empty string entirely.
        let err = ApiKeyScope::from_permissions([""]).unwrap_err();
        assert_eq!(err, ScopeError::EmptyPermission);
        // Empty entry from a stray comma.
        let err = ApiKeyScope::parse_comma_separated("orders.read,, refunds.write")
            .unwrap_err();
        assert_eq!(err, ScopeError::EmptyPermission);
    }

    #[test]
    fn empty_granted_scope_refuses_any_non_empty_required() {
        let granted = ApiKeyScope::empty();
        let required = scope(&["orders.read"]);
        match enforce_scope_grant(&granted, &required).unwrap_err() {
            ScopeError::EscalationAttempt { missing } => {
                assert_eq!(missing, vec!["orders.read".to_string()]);
            }
            other => panic!("expected EscalationAttempt, got {other:?}"),
        }
    }
}
