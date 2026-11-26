"""Durable login fixtures for serve integration checks."""
from contextlib import closing
import hashlib
import hmac
import secrets
import sqlite3
import time


def seed_session(auth, actor, tenant, csrf_secret, lifetime_ms=600000):
    # Seed the durable state a completed login produces. No provider is contacted.
    token = secrets.token_hex(32)
    hashed = "sha256:" + hashlib.sha256(("corvid-auth-session-v1:" + token).encode()).hexdigest()
    binding = secrets.token_hex(16)
    mac = hmac.new(csrf_secret.encode(), ("corvid-csrf-v1:" + binding).encode(), "sha256")
    csrf = binding + "." + mac.hexdigest()
    with closing(sqlite3.connect(auth)) as db, db:
        db.execute("INSERT INTO auth_actors (id, tenant_id, display_name, actor_kind, auth_method, "
                   "assurance_level, role_fingerprint, permission_fingerprint, created_ms, updated_ms) "
                   "VALUES (?, ?, ?, 'user', 'oauth', 'aal1', '', '', 0, 0)", (actor, tenant, actor))
        db.execute("INSERT INTO auth_sessions (id, actor_id, tenant_id, token_hash, issued_ms, "
                   "expires_ms, rotation_counter, csrf_binding_id, created_ms, updated_ms) "
                   "VALUES (?, ?, ?, ?, 1, ?, 0, ?, 1, 1)",
                   (actor, actor, tenant, hashed, int(time.time() * 1000) + lifetime_ms, binding))
    return {"Cookie": f"corvid_session={token}; corvid_csrf={csrf}", "X-CSRF-Token": csrf}

def set_role(auth, actor, role, granted):
    with closing(sqlite3.connect(auth)) as db, db:
        if granted:
            db.execute("INSERT INTO auth_actor_roles VALUES (?, ?, 1)", (actor, role))
        else:
            db.execute("DELETE FROM auth_actor_roles WHERE actor_id=? AND role=?", (actor, role))
