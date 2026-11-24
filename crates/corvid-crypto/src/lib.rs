//! Cryptography rules for Corvid.
//!
//! Every rule behind the cryptography builtins lives here once: the
//! `Bytes` type, the `Secret` type, the constant-time comparison both
//! use for `==`, the hex and base64 forms of bytes, and SHA-2 hashing.
//! The interpreter executes through these functions, and the native
//! runtime will call the same ones, so the tiers cannot disagree on a
//! digest or a comparison. The crate depends only on `corvid-text`,
//! whose byte codecs it uses, so the checker can depend on it too.
//!
//! MD5 and SHA-1 are deliberately absent: they are broken for the uses
//! people reach for them in.

mod bytes;
mod constant_time;
mod digest;
mod secret;

pub use bytes::Bytes;
pub use constant_time::constant_time_eq;
pub use digest::{sha256, sha384, sha512};
pub use secret::{Secret, SECRET_DISPLAY};
