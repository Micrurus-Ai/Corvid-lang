//! Auth-route wiring for `corvid serve` (slice 52e).
//!
//! Turns an `identity` block into the running login surface —
//! `/auth/{provider}/login|callback|logout|session` — wired to the
//! `corvid-runtime` auth storage and JWT/userinfo identity
//! verification. The submodules split by responsibility:
//!
//! - [`provider`] — resolve a declared provider to its endpoints,
//!   verification method, and client credentials.
//! - [`identity`] — turn a code exchange into a server-verified external
//!   identity (verify the ID token, or fetch userinfo).

pub mod identity;
pub mod provider;
