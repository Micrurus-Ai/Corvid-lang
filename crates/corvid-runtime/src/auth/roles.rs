//! Actor roles and route authorization (slice 52f).
//!
//! An actor holds a SET of role names (in `auth_actor_roles`); the
//! app's identity block maps each role to the permissions it grants. A
//! route policy names the roles/permissions it requires, and
//! [`authorize_route`] decides — an actor is allowed iff it holds every
//! required role and, transitively, every required permission (the union
//! of its roles' permission sets). Membership, not fingerprint equality:
//! that is what lets `requires role("admin")` mean "holds admin among
//! its roles."

use std::collections::HashMap;

use rusqlite::params;

use super::{validate_non_empty, SessionAuthRuntime};
use crate::errors::RuntimeError;

impl SessionAuthRuntime {
    /// Grant a role to an actor (idempotent — re-granting the same role
    /// is a no-op). Authority is only ever added by an explicit grant,
    /// never inferred.
    pub fn grant_actor_role(
        &self,
        actor_id: &str,
        role: &str,
        at_ms: u64,
    ) -> Result<(), RuntimeError> {
        validate_non_empty("actor id", actor_id)?;
        validate_non_empty("role", role)?;
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert or ignore into auth_actor_roles (actor_id, role, granted_ms)
                 values (?1, ?2, ?3)",
                params![actor_id, role, at_ms as i64],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to grant role: {err}")))?;
        Ok(())
    }

    /// The set of role names an actor holds, sorted for determinism.
    pub fn actor_roles(&self, actor_id: &str) -> Result<Vec<String>, RuntimeError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("select role from auth_actor_roles where actor_id = ?1 order by role asc")
            .map_err(|err| RuntimeError::Other(format!("failed to read actor roles: {err}")))?;
        let rows = stmt
            .query_map(params![actor_id], |row| row.get::<_, String>(0))
            .map_err(|err| RuntimeError::Other(format!("failed to read actor roles: {err}")))?;
        let mut roles = Vec::new();
        for role in rows {
            roles.push(role.map_err(|err| {
                RuntimeError::Other(format!("failed to read actor role row: {err}"))
            })?);
        }
        Ok(roles)
    }
}

/// What a route's `requires` clause demands. Every listed role and every
/// listed permission must be satisfied (AND, not OR).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoutePolicyRequirement {
    pub authenticated: bool,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
}

/// The decision for one authorization check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAuthzOutcome {
    Allowed,
    /// Denied, with a stable reason for the audit log (never surfaced to
    /// the caller verbatim).
    Denied(String),
}

/// Decide whether an actor satisfies a route policy. `actor_roles` are
/// the roles the actor holds; `role_permissions` is the app's declared
/// role → permission mapping. The caller has already established that a
/// valid session exists (so `authenticated` is met); this checks the
/// role and permission requirements.
pub fn authorize_route(
    actor_roles: &[String],
    role_permissions: &HashMap<String, Vec<String>>,
    requirement: &RoutePolicyRequirement,
) -> RouteAuthzOutcome {
    for required in &requirement.roles {
        if !actor_roles.iter().any(|held| held == required) {
            return RouteAuthzOutcome::Denied(format!("actor lacks required role `{required}`"));
        }
    }
    if !requirement.permissions.is_empty() {
        let effective = effective_permissions(actor_roles, role_permissions);
        for required in &requirement.permissions {
            if !effective.contains(required.as_str()) {
                return RouteAuthzOutcome::Denied(format!(
                    "actor lacks required permission `{required}`"
                ));
            }
        }
    }
    RouteAuthzOutcome::Allowed
}

/// The union of the permissions granted by the actor's roles.
fn effective_permissions<'a>(
    actor_roles: &[String],
    role_permissions: &'a HashMap<String, Vec<String>>,
) -> std::collections::HashSet<&'a str> {
    let mut permissions = std::collections::HashSet::new();
    for role in actor_roles {
        if let Some(granted) = role_permissions.get(role) {
            for permission in granted {
                permissions.insert(permission.as_str());
            }
        }
    }
    permissions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthActor;

    fn actor(id: &str) -> AuthActor {
        AuthActor {
            id: id.to_string(),
            tenant_id: "org-1".to_string(),
            display_name: "Ada".to_string(),
            actor_kind: "user".to_string(),
            auth_method: "oauth".to_string(),
            assurance_level: "aal1".to_string(),
            role_fingerprint: String::new(),
            permission_fingerprint: String::new(),
            created_ms: 1,
            updated_ms: 1,
        }
    }

    fn role_map() -> HashMap<String, Vec<String>> {
        HashMap::from([
            (
                "admin".to_string(),
                vec!["refund:write".to_string(), "user:read".to_string()],
            ),
            ("member".to_string(), vec!["user:read".to_string()]),
        ])
    }

    #[test]
    fn granting_a_role_is_idempotent_and_queryable() {
        let auth = SessionAuthRuntime::open_in_memory().unwrap();
        auth.upsert_actor(actor("u1")).unwrap();
        auth.grant_actor_role("u1", "admin", 1).unwrap();
        auth.grant_actor_role("u1", "admin", 2).unwrap(); // idempotent
        auth.grant_actor_role("u1", "member", 3).unwrap();
        assert_eq!(
            auth.actor_roles("u1").unwrap(),
            vec!["admin".to_string(), "member".to_string()]
        );
        assert!(auth.actor_roles("u2").unwrap().is_empty());
    }

    #[test]
    fn a_held_role_and_its_permissions_are_authorized() {
        let outcome = authorize_route(
            &["admin".to_string()],
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec!["admin".to_string()],
                permissions: vec!["refund:write".to_string()],
            },
        );
        assert_eq!(outcome, RouteAuthzOutcome::Allowed);
    }

    #[test]
    fn a_missing_role_is_denied() {
        let outcome = authorize_route(
            &["member".to_string()],
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec!["admin".to_string()],
                permissions: vec![],
            },
        );
        assert!(matches!(outcome, RouteAuthzOutcome::Denied(r) if r.contains("role `admin`")));
    }

    #[test]
    fn a_permission_not_granted_by_any_held_role_is_denied() {
        // `member` grants only `user:read`, not `refund:write`.
        let outcome = authorize_route(
            &["member".to_string()],
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec![],
                permissions: vec!["refund:write".to_string()],
            },
        );
        assert!(matches!(outcome, RouteAuthzOutcome::Denied(r) if r.contains("refund:write")));
    }

    #[test]
    fn permissions_union_across_multiple_roles() {
        // Holding both roles unions their permissions.
        let outcome = authorize_route(
            &["member".to_string(), "admin".to_string()],
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec![],
                permissions: vec!["refund:write".to_string(), "user:read".to_string()],
            },
        );
        assert_eq!(outcome, RouteAuthzOutcome::Allowed);
    }
}
