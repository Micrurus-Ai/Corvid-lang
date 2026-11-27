//! Cryptography rules for Corvid.
//!
//! Every rule behind the cryptography builtins lives here once: the
//! `Bytes` type, the `Secret` type, the constant-time comparison both
//! use for `==`, the hex and base64 forms of bytes, SHA-2 hashing,
//! HMAC, strict Ed25519 signature verification, random tokens from
//! operating-system entropy, and Argon2id password hashing at a
//! declared cost. The interpreter executes through these functions,
//! and the native runtime will call the same ones, so the tiers cannot
//! disagree on a digest, a tag, a signature or a comparison. The
//! runtime's API-key store hashes and verifies through the same
//! password functions. Besides the cryptography crates, the crate
//! depends only on `corvid-text`, whose byte codecs it uses, so the
//! checker can depend on it too.
//!
//! MD5 and SHA-1 are deliberately absent: they are broken for the uses
//! people reach for them in.

mod aead;
mod bytes;
mod constant_time;
mod digest;
mod entropy;
mod mac;
mod password;
mod secret;
mod signature;
mod token;

pub use aead::{DataKey, SealError, DATA_KEY_BYTES};
pub use bytes::Bytes;
pub use constant_time::constant_time_eq;
pub use digest::{sha256, sha384, sha512};
pub use entropy::fill_random;
pub use mac::{hmac_sha256, hmac_sha512, HmacKey};
pub use password::{hash_password, verify_password, PasswordCost, MAX_MEMORY_KIB};
pub use secret::{Secret, SECRET_DISPLAY};
pub use signature::{verify_ed25519, verify_ed25519_signature, ED25519_PUBLIC_KEY_BYTES, ED25519_SIGNATURE_BYTES};
pub use token::{random_token, MAX_TOKEN_BYTES, MIN_TOKEN_BYTES};
