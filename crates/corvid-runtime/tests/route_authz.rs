//! Adversarial coverage for route authorization (slice 52f).
//!
//! `corvid serve`'s `enforce_route_policy` composes exactly these runtime
//! primitives — `resolve_session_cookie` → `actor_roles` →
//! `authorize_route`, plus `verify_csrf_double_submit` — so exercising
//! them together proves the enforcement decision for every adversarial
//! shape the acceptance criteria name, without a live HTTP server. The
//! serve_smoke suite proves the wiring (an unauthenticated request to a
//! protected route is a 401 before the handler runs).

use std::collections::HashMap;

use corvid_runtime::{
    authorize_route, mint_csrf_token, verify_csrf_double_submit, AuthActor, CsrfRequestMethod,
    RouteAuthzOutcome, RoutePolicyRequirement, SessionAuthRuntime, SessionCreate,
};

fn actor(id: &str, tenant: &str) -> AuthActor {
    AuthActor {
        id: id.to_string(),
        tenant_id: tenant.to_string(),
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

/// Seed an actor with a live session; returns the raw session token.
fn seed_session(auth: &SessionAuthRuntime, actor_id: &str, tenant: &str, expires_ms: u64) -> String {
    auth.upsert_actor(actor(actor_id, tenant)).unwrap();
    let token = format!("tok-{actor_id}");
    auth.create_session(SessionCreate {
        id: format!("sess-{actor_id}"),
        actor_id: actor_id.to_string(),
        tenant_id: tenant.to_string(),
        raw_token: token.clone(),
        issued_ms: 1_000,
        expires_ms,
        csrf_binding_id: format!("csrf-{actor_id}"),
    })
    .unwrap();
    token
}

/// The happy path: a valid session for an actor holding the required role
/// is allowed, and the required permission resolves through the role.
#[test]
fn a_valid_session_with_the_required_role_is_allowed() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    let token = seed_session(&auth, "u1", "org-1", 9_000_000_000);
    auth.grant_actor_role("u1", "admin", 1_000).unwrap();

    let resolution = auth.resolve_session_cookie(&token, "trace", 2_000).unwrap();
    let roles = auth.actor_roles(&resolution.actor.id).unwrap();
    let outcome = authorize_route(
        &roles,
        &role_map(),
        &RoutePolicyRequirement {
            authenticated: true,
            roles: vec!["admin".to_string()],
            permissions: vec!["refund:write".to_string()],
        },
    );
    assert_eq!(outcome, RouteAuthzOutcome::Allowed);
}

/// A forged / unknown cookie resolves to nothing → 401 territory.
#[test]
fn a_forged_cookie_is_rejected() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    seed_session(&auth, "u1", "org-1", 9_000_000_000);
    assert!(auth
        .resolve_session_cookie("not-a-real-token", "trace", 2_000)
        .is_err());
}

/// An expired session is rejected at resolution.
#[test]
fn an_expired_session_is_rejected() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    let token = seed_session(&auth, "u1", "org-1", 5_000);
    // now (6_000) is past expiry (5_000).
    assert!(auth.resolve_session_cookie(&token, "trace", 6_000).is_err());
}

/// A revoked session is rejected — this is the "stale roles after
/// revocation" guarantee: revoking a role invalidates the actor's
/// sessions, so the old cookie no longer resolves AND a fresh read shows
/// no role.
#[test]
fn revoking_a_role_invalidates_sessions_and_clears_the_role() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    let token = seed_session(&auth, "u1", "org-1", 9_000_000_000);
    auth.grant_actor_role("u1", "admin", 1_000).unwrap();
    // Before revocation: session resolves and the role is held.
    assert!(auth.resolve_session_cookie(&token, "t", 2_000).is_ok());
    assert_eq!(auth.actor_roles("u1").unwrap(), vec!["admin".to_string()]);

    let had = auth.revoke_actor_role("u1", "admin", 3_000).unwrap();
    assert!(had);
    // After revocation: the role is gone AND the session is invalidated.
    assert!(auth.actor_roles("u1").unwrap().is_empty());
    assert!(
        auth.resolve_session_cookie(&token, "t", 4_000).is_err(),
        "the session must be invalidated after a privilege change"
    );
}

/// An authenticated actor lacking the required role is denied (403) —
/// authenticated-but-insufficient.
#[test]
fn an_authenticated_but_insufficient_actor_is_denied() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    let token = seed_session(&auth, "u1", "org-1", 9_000_000_000);
    auth.grant_actor_role("u1", "member", 1_000).unwrap();
    let resolution = auth.resolve_session_cookie(&token, "t", 2_000).unwrap();
    let roles = auth.actor_roles(&resolution.actor.id).unwrap();
    // `member` cannot satisfy a `requires role("admin")`.
    let outcome = authorize_route(
        &roles,
        &role_map(),
        &RoutePolicyRequirement {
            authenticated: true,
            roles: vec!["admin".to_string()],
            permissions: vec![],
        },
    );
    assert!(matches!(outcome, RouteAuthzOutcome::Denied(_)));
    // And a permission `member` does not grant is denied too.
    let outcome = authorize_route(
        &roles,
        &role_map(),
        &RoutePolicyRequirement {
            authenticated: true,
            roles: vec![],
            permissions: vec!["refund:write".to_string()],
        },
    );
    assert!(matches!(outcome, RouteAuthzOutcome::Denied(_)));
}

/// A CSRF double-submit mismatch on a mutation is rejected.
#[test]
fn a_csrf_mismatch_on_a_mutation_is_rejected() {
    let secret = b"server-csrf-secret";
    let good = mint_csrf_token("binding-1", secret).unwrap();
    // Missing header, or a header that doesn't match the cookie, both fail.
    assert!(verify_csrf_double_submit(
        CsrfRequestMethod::StateChanging,
        None,
        Some(&good),
        secret
    )
    .is_err());
    assert!(verify_csrf_double_submit(
        CsrfRequestMethod::StateChanging,
        Some("attacker.deadbeef"),
        Some(&good),
        secret
    )
    .is_err());
    // The honest double-submit succeeds.
    assert!(verify_csrf_double_submit(
        CsrfRequestMethod::StateChanging,
        Some(&good),
        Some(&good),
        secret
    )
    .is_ok());
    // A safe method never requires CSRF.
    assert!(verify_csrf_double_submit(CsrfRequestMethod::Safe, None, None, secret).is_ok());
}

/// Roles are not honored across a tenant boundary: a session whose tenant
/// does not match the actor's is refused at resolution, so a role held in
/// one tenant cannot be exercised under another.
#[test]
fn a_cross_tenant_session_is_rejected() {
    let auth = SessionAuthRuntime::open_in_memory().unwrap();
    // Actor lives in org-1, but a session is minted claiming org-2.
    auth.upsert_actor(actor("u1", "org-1")).unwrap();
    // The tenant boundary is enforced at the earliest possible point: a
    // session for an actor in a different tenant cannot even be created,
    // so a role held in one tenant can never be exercised under another.
    let result = auth.create_session(SessionCreate {
        id: "sess-x".to_string(),
        actor_id: "u1".to_string(),
        tenant_id: "org-2".to_string(),
        raw_token: "tok-x".to_string(),
        issued_ms: 1_000,
        expires_ms: 9_000_000_000,
        csrf_binding_id: "csrf-x".to_string(),
    });
    assert!(
        result.is_err(),
        "a session must not be created for an actor in a different tenant"
    );
}

/// The permission union across multiple held roles is honored — but only
/// permissions some held role actually grants (no escalation).
#[test]
fn permission_union_grants_only_what_a_held_role_provides() {
    let roles = vec!["member".to_string()];
    // `member` grants `user:read` (allowed) but NOT `refund:write` (denied).
    assert_eq!(
        authorize_route(
            &roles,
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec![],
                permissions: vec!["user:read".to_string()],
            }
        ),
        RouteAuthzOutcome::Allowed
    );
    assert!(matches!(
        authorize_route(
            &roles,
            &role_map(),
            &RoutePolicyRequirement {
                authenticated: true,
                roles: vec![],
                permissions: vec!["refund:write".to_string()],
            }
        ),
        RouteAuthzOutcome::Denied(_)
    ));
}
