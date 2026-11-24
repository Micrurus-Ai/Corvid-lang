//! Text and encoding rules for Corvid (B03).
//!
//! Every rule behind the text builtin methods lives here once: the
//! interpreter and the native runtime both call these functions, so the
//! two tiers cannot disagree on an encoding. The crate depends on no
//! other Corvid crate, so the checker can use it too.
//!
//! Text crosses every encoder as its UTF-8 bytes. Decoders return a
//! message naming the first problem and its position, counted in
//! Unicode scalar values like `String.length()`, and never panic on
//! malformed input.

pub mod encoding;
pub mod uuid;

pub use encoding::{
    decode_base64, decode_base64_url, decode_hex, decode_url_component, encode_base64,
    encode_base64_url, encode_hex, encode_url_component,
};
pub use uuid::{parse_uuid, uuid_v4_from_random, uuid_v7_from_parts};
