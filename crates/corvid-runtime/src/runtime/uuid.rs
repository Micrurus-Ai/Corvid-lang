//! The executing std/uuid tools (B03): `uuid_v4` and `uuid_v7`.
//!
//! The bit layout and canonical text come from `corvid-text`, which
//! also parses UUIDs for `String.parse_uuid()`; this file supplies only
//! the entropy and the clock. Both tools go through `call_tool`, so the
//! result is traced and replay substitutes the recorded UUID before
//! dispatch ever reaches here.

use crate::errors::RuntimeError;
use serde_json::{json, Value};

/// Only the two std/uuid tool names are handled here.
pub(super) fn dispatch(name: &str, args: &[Value]) -> Option<Result<Value, RuntimeError>> {
    if !matches!(name, "uuid_v4" | "uuid_v7") {
        return None;
    }
    if !args.is_empty() {
        return Some(Err(RuntimeError::ToolFailed {
            tool: name.to_owned(),
            message: "expected no arguments".into(),
        }));
    }
    Some(generate(name).map(|uuid| json!(uuid)))
}

fn generate(name: &str) -> Result<String, RuntimeError> {
    if name == "uuid_v4" {
        let mut random = [0u8; 16];
        fill_random(name, &mut random)?;
        return Ok(corvid_text::uuid_v4_from_random(random));
    }
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| RuntimeError::ToolFailed {
            tool: name.to_owned(),
            message: "the system clock is set before 1970".into(),
        })?
        .as_millis();
    let unix_ms = u64::try_from(unix_ms).map_err(|_| RuntimeError::ToolFailed {
        tool: name.to_owned(),
        message: "the system clock is beyond the version 7 range".into(),
    })?;
    let mut random = [0u8; 10];
    fill_random(name, &mut random)?;
    Ok(corvid_text::uuid_v7_from_parts(unix_ms, random))
}

fn fill_random(tool: &str, bytes: &mut [u8]) -> Result<(), RuntimeError> {
    use rand_core::{OsRng, RngCore};
    OsRng.try_fill_bytes(bytes).map_err(|error| RuntimeError::ToolFailed {
        tool: tool.to_owned(),
        message: format!("OS entropy source failed: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generated(name: &str) -> String {
        match dispatch(name, &[]) {
            Some(Ok(Value::String(uuid))) => uuid,
            other => panic!("expected a UUID string, got {other:?}"),
        }
    }

    #[test]
    fn generated_uuids_are_canonical_and_carry_their_version() {
        for (name, version) in [("uuid_v4", '4'), ("uuid_v7", '7')] {
            let uuid = generated(name);
            assert_eq!(corvid_text::parse_uuid(&uuid).as_deref(), Ok(uuid.as_str()));
            assert_eq!(uuid.chars().nth(14), Some(version), "{uuid}");
            assert!(matches!(uuid.chars().nth(19), Some('8' | '9' | 'a' | 'b')), "{uuid}");
        }
    }

    #[test]
    fn two_generated_uuids_differ() {
        assert_ne!(generated("uuid_v4"), generated("uuid_v4"));
        assert_ne!(generated("uuid_v7"), generated("uuid_v7"));
    }

    #[test]
    fn version_7_starts_with_the_current_time() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let uuid = generated("uuid_v7");
        let millis = u64::from_str_radix(&uuid.replace('-', "")[..12], 16).unwrap();
        assert!(millis >= before && millis < before + 60_000, "{uuid}");
    }

    #[test]
    fn other_names_and_arguments_are_not_uuid_calls() {
        assert!(dispatch("random_int", &[]).is_none());
        assert!(matches!(dispatch("uuid_v4", &[json!(1)]), Some(Err(_))));
    }
}
