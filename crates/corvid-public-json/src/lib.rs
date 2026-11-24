//! The public JSON mapping between Corvid values and ordinary JSON.
//!
//! This is the shape Corvid promises to code outside the program:
//! HTTP request and response bodies, the published OpenAPI schemas,
//! and `T.from_json(text)` / `value.to_json()`. It follows the JSON
//! conventions other ecosystems use:
//!
//! | Corvid                 | JSON                                              |
//! | ---------------------- | ------------------------------------------------- |
//! | `Int`                  | integer (a fraction or out-of-range value fails)  |
//! | `Float`                | number (integers are accepted)                    |
//! | `String`, `Bool`       | string, boolean                                   |
//! | `Nothing`              | `null`                                            |
//! | record                 | object; an `Option` field may be `null` or absent |
//! | `Option<T>`            | the `T` value, or `null` for `None`               |
//! | `List<T>`              | array                                             |
//! | `Map<String, V>`       | object                                            |
//! | `Map<K, V>` (other K)  | array of `[key, value]` pairs                     |
//! | `Result<T, E>`         | `{"tag": "ok", "ok": T}` or `{"tag": "err", "err": E}`; `tag` is optional on input |
//! | sum type               | `{"tag": "variant", "type": …, "variant": …, "fields": [...]}` |
//! | `Grounded<T>`          | `{"value": T, "sources": [...]}` (encode only)    |
//! | `JsonValue`            | the JSON itself                                   |
//!
//! Decoding never fabricates provenance: a `Grounded<T>` target is
//! rejected, because JSON text cannot prove where a value came from.
//!
//! The decoder is written once, against the [`ValueBuilder`] trait.
//! The interpreter builds its own values and the native runtime builds
//! native heap objects, so the two targets accept the same JSON and
//! report the same path, expectation and finding for every failure.

mod decode;
mod error;

pub use decode::{decode, decode_text, ValueBuilder};
pub use error::{parse_json_text, JsonDecodeError, UnknownFields};
