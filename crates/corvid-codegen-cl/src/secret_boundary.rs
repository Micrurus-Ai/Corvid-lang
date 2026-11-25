//! Where native code still cannot use a `Secret` (B04, native dispatch
//! N3), in the one wording the code generator and the driver's tier
//! picker both use.
//!
//! Native code holds a secret as the runtime's opaque object: it can get
//! one from `secret(name)` or `random_token(bytes)` through the runtime
//! call, hold it, pass it between agents and compare it. Two uses wait:
//! a tool of the program's own whose signature holds a secret, because a
//! native tool call records its arguments without redaction, and the
//! cryptography a secret keys (HMAC, `verify_password`,
//! `decode_base64_after`, `hash_password`), which arrives in later
//! slices. A program that needs either runs on the interpreter.

/// The reason, for compile errors and the tier picker.
pub const SECRET_REQUIREMENT: &str = "a `Secret` passed to a tool of the program's own, to `hash_password`, or used to key an HMAC, `verify_password` or `decode_base64_after` runs on the interpreter tier: a native tool call records its arguments without redaction, and native secret cryptography is not built yet";
