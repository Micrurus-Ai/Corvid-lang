//! Where native code still cannot use a `Secret` (B04, native dispatch
//! N3), in the one wording the code generator and the driver's tier
//! picker both use.
//!
//! Native code holds a secret as the runtime's opaque object: it gets one
//! from `secret(name)` or `random_token(bytes)` through the runtime call,
//! holds and compares it, and keys HMAC, `verify_password` and
//! `decode_base64_after` with it. A secret passed to a tool of the
//! program's own, or to `hash_password` until N3c, waits: a native tool
//! call records its arguments without redaction.

/// The reason, for compile errors and the tier picker.
pub const SECRET_REQUIREMENT: &str = "a `Secret` passed to a tool of the program's own, or to `hash_password`, runs on the interpreter tier: a native tool call records its arguments without redaction";
