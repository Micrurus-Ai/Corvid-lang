//! Text and encoding rules for Corvid (B03).
//!
//! Every rule behind the text builtin methods lives here once: the
//! interpreter and the native runtime both call these functions, so the
//! two tiers cannot disagree on an encoding or a match. The crate
//! depends on no other Corvid crate, so the checker uses it too, to
//! validate regex pattern and replacement literals with the parser the
//! runtime compiles them with.
//!
//! Text crosses every encoder as its UTF-8 bytes. Decoders return a
//! message naming the first problem and its position, counted in
//! Unicode scalar values like `String.length()`, and never panic on
//! malformed input.

pub mod encoding;
pub mod regex;
pub mod uuid;

pub use encoding::{
    decode_base64, decode_base64_bytes, decode_base64_url, decode_base64_url_bytes, decode_hex,
    decode_form_component, decode_hex_bytes, decode_url_component, encode_base64, encode_base64_bytes, encode_base64_url,
    encode_base64_url_bytes, encode_hex, encode_hex_bytes, encode_url_component,
};
pub use uuid::{parse_uuid, uuid_v4_from_random, uuid_v7_from_parts};
