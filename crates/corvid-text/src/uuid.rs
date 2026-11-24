//! UUIDs (RFC 9562) as canonical text.
//!
//! Corvid represents a UUID as a `String` in canonical form: 36
//! lowercase characters, hyphenated 8-4-4-4-12. `parse_uuid` is the
//! one way in from other spellings, so two equal UUIDs are always equal
//! strings. Generation lives in the runtime, which supplies the entropy
//! and the clock; the bit layout is here so it is tested in one place.

/// `text.parse_uuid()`: the canonical form of a UUID written
/// hyphenated, as 32 digits, braced (`{...}`) or as a `urn:uuid:` URN,
/// in either case, after trimming surrounding whitespace. Any version
/// and variant is accepted, including the nil and max UUIDs.
pub fn parse_uuid(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    let body = strip_urn(trimmed)
        .or_else(|| trimmed.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')))
        .unwrap_or(trimmed);
    let digits: Option<Vec<u8>> = match body.len() {
        32 => body.bytes().map(hex_value).collect(),
        36 => {
            let hyphens_in_place = body
                .bytes()
                .enumerate()
                .all(|(index, byte)| (byte == b'-') == matches!(index, 8 | 13 | 18 | 23));
            if hyphens_in_place {
                body.bytes().filter(|byte| *byte != b'-').map(hex_value).collect()
            } else {
                None
            }
        }
        _ => None,
    };
    let Some(digits) = digits else {
        return Err(format!(
            "not a UUID: `{trimmed}` (expected 32 hex digits, optionally hyphenated 8-4-4-4-12)"
        ));
    };
    let mut bytes = [0u8; 16];
    for (index, pair) in digits.chunks(2).enumerate() {
        bytes[index] = pair[0] << 4 | pair[1];
    }
    Ok(format_uuid(&bytes))
}

fn strip_urn(text: &str) -> Option<&str> {
    const PREFIX: &str = "urn:uuid:";
    let head = text.get(..PREFIX.len())?;
    head.eq_ignore_ascii_case(PREFIX).then(|| &text[PREFIX.len()..])
}

fn hex_value(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

/// The canonical text of sixteen UUID bytes.
pub fn format_uuid(bytes: &[u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Set the version nibble and the RFC 9562 variant bits (`10`).
fn stamp(bytes: &mut [u8; 16], version: u8) {
    bytes[6] = (bytes[6] & 0x0f) | (version << 4);
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
}

/// A version 4 UUID from sixteen random bytes; six bits are replaced
/// by the version and variant.
pub fn uuid_v4_from_random(random: [u8; 16]) -> String {
    let mut bytes = random;
    stamp(&mut bytes, 4);
    format_uuid(&bytes)
}

/// A version 7 UUID: the Unix time in milliseconds in the first 48
/// bits, so UUIDs sort by creation time, then 74 random bits.
pub fn uuid_v7_from_parts(unix_ms: u64, random: [u8; 10]) -> String {
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&unix_ms.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&random);
    stamp(&mut bytes, 7);
    format_uuid(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6";

    #[test]
    fn every_common_spelling_parses_to_the_canonical_form() {
        for spelling in [
            "f81d4fae-7dec-11d0-a765-00a0c91e6bf6",
            "F81D4FAE-7DEC-11D0-A765-00A0C91E6BF6",
            "f81d4fae7dec11d0a76500a0c91e6bf6",
            "{f81d4fae-7dec-11d0-a765-00a0c91e6bf6}",
            "urn:uuid:f81d4fae-7dec-11d0-a765-00a0c91e6bf6",
            "URN:UUID:F81D4FAE-7DEC-11D0-A765-00A0C91E6BF6",
            "  f81d4fae-7dec-11d0-a765-00a0c91e6bf6\n",
        ] {
            assert_eq!(parse_uuid(spelling).as_deref(), Ok(CANONICAL), "{spelling:?}");
        }
    }

    #[test]
    fn nil_and_max_uuids_parse() {
        let nil = "00000000-0000-0000-0000-000000000000";
        let max = "ffffffff-ffff-ffff-ffff-ffffffffffff";
        assert_eq!(parse_uuid(nil).as_deref(), Ok(nil));
        assert_eq!(parse_uuid(&max.to_uppercase()).as_deref(), Ok(max));
    }

    #[test]
    fn malformed_uuids_are_errors_naming_the_input() {
        for bad in [
            "",
            "f81d4fae-7dec-11d0-a765-00a0c91e6bf",
            "f81d4fae-7dec-11d0-a765-00a0c91e6bf6a",
            "f81d4fae7-dec-11d0-a765-00a0c91e6bf6",
            "g81d4fae-7dec-11d0-a765-00a0c91e6bf6",
            "{f81d4fae-7dec-11d0-a765-00a0c91e6bf6",
            "f81d4fae-7dec-11d0-a765+00a0c91e6bf6",
            "é81d4fae-7dec-11d0-a765-00a0c91e6bf",
        ] {
            let error = parse_uuid(bad).unwrap_err();
            assert!(error.starts_with("not a UUID: `"), "{bad:?}: {error}");
        }
        assert_eq!(
            parse_uuid(" nope ").unwrap_err(),
            "not a UUID: `nope` (expected 32 hex digits, optionally hyphenated 8-4-4-4-12)"
        );
    }

    #[test]
    fn version_4_sets_only_the_version_and_variant_bits() {
        let all_ones = uuid_v4_from_random([0xff; 16]);
        assert_eq!(all_ones, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let all_zeros = uuid_v4_from_random([0; 16]);
        assert_eq!(all_zeros, "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn version_7_matches_the_rfc_9562_example() {
        // RFC 9562 appendix A.6: 2022-02-22 19:22:22 UTC is
        // 0x017F22E279B0 ms, with rand_a 0xCC3 and rand_b
        // 0x18C4DC0C0C07398F.
        let random = [0x0c, 0xc3, 0x18, 0xc4, 0xdc, 0x0c, 0x0c, 0x07, 0x39, 0x8f];
        assert_eq!(
            uuid_v7_from_parts(0x017F_22E2_79B0, random),
            "017f22e2-79b0-7cc3-98c4-dc0c0c07398f"
        );
    }

    #[test]
    fn version_7_uuids_sort_by_time() {
        let earlier = uuid_v7_from_parts(1_700_000_000_000, [0xff; 10]);
        let later = uuid_v7_from_parts(1_700_000_000_001, [0; 10]);
        assert!(earlier < later);
        assert_eq!(parse_uuid(&earlier).as_deref(), Ok(earlier.as_str()));
    }
}
