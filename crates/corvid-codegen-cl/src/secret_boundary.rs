//! Where native code still cannot use a `Secret` (B04, native dispatch
//! N3), in the one wording the code generator and the driver's tier
//! picker both use.
//!
//! Native code holds a secret as the runtime's opaque object, gets one
//! from `secret(name)` or `random_token(bytes)`, compares it, keys HMAC
//! and password checks with it, and passes it to `hash_password`, all
//! through the runtime, which records `<redacted>` in its place. A tool of
//! the program's own is called directly by native code, which records its
//! arguments without redaction, so passing a secret to one waits until
//! user tools move onto the runtime call.

/// The reason, for compile errors and the tier picker.
pub const SECRET_REQUIREMENT: &str = "a `Secret` passed to or returned by a tool of the program's own runs on the interpreter tier: native code calls such a tool directly and records its arguments without redaction";
