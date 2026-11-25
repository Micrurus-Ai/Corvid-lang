//! Why native code has no `Secret` (B04), in the one wording the code
//! generator and the driver's tier picker both use.
//!
//! A program gets a secret only from a tool: `secret(name)` from
//! std/secrets or `random_token(bytes)` from std/crypto. Native
//! binaries do not dispatch the standard library's tools, and a native
//! tool call has no path that hands an implementation a secret's text
//! while every record of the call (the trace, replay, approvals) holds
//! `<redacted>`, and that scrubs a secret the tool echoes from the
//! recorded result only. Rather than run a secret through code that
//! cannot keep those guarantees, a program that mentions `Secret`
//! anywhere runs on the interpreter, which keeps all of them.

/// The reason, for compile errors and the tier picker.
pub const SECRET_REQUIREMENT: &str = "`Secret` runs on the interpreter tier: a secret comes only from a tool (`secret(name)`, `random_token(bytes)`) and reaches tools as text, and native binaries neither dispatch the standard library's tools nor redact a secret in a native tool call's record";
