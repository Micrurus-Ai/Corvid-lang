//! B06 — the types whose public JSON form is their exact text.
//!
//! `Decimal`, `Duration`, `Instant`, `TimeZone`, `ZonedTime`,
//! `Currency` and `Locale` each cross JSON as a string in their one
//! canonical text form, the form their own crate prints and parses:
//!
//! | Type | JSON | Example |
//! | --- | --- | --- |
//! | `Decimal` | the exact digits, scale kept | `"19.90"` |
//! | `Duration` | ISO 8601, no calendar units | `"PT1H30M"` |
//! | `Instant` | RFC 3339 in UTC | `"2026-07-11T08:30:00Z"` |
//! | `TimeZone` | the IANA name | `"Europe/Berlin"` |
//! | `ZonedTime` | RFC 9557 | `"2026-03-29T03:30:00+02:00[Europe/Berlin]"` |
//! | `Currency` | the ISO 4217 code | `"EUR"` |
//! | `Locale` | the canonical BCP 47 tag | `"de-DE"` |
//!
//! A string, not a JSON number, carries a `Decimal`, because JSON
//! numbers lose digits in JavaScript and in any decoder that goes
//! through a binary float; a number is refused with a message saying
//! so. Every layer that crosses JSON (the decoder, the encoder, tool
//! and prompt values on the interpreter, OpenAPI, prompt schemas)
//! reads this one module, so the forms cannot drift apart.

use corvid_currency::{Currency, Locale};
use corvid_decimal::Decimal;
use corvid_time::{Duration, Instant, TimeZone, ZonedTime};
use corvid_types::Type;
use serde_json::{json, Value as J};

/// A parsed value of one of the text-scalar types.
#[derive(Debug, Clone, PartialEq)]
pub enum TextScalar {
    Decimal(Decimal),
    Duration(Duration),
    Instant(Instant),
    TimeZone(TimeZone),
    ZonedTime(ZonedTime),
    Currency(Currency),
    Locale(Locale),
}

impl TextScalar {
    /// The canonical text, exactly what [`parse`] reads back.
    pub fn to_text(&self) -> String {
        match self {
            TextScalar::Decimal(d) => d.to_string(),
            TextScalar::Duration(d) => d.to_string(),
            TextScalar::Instant(i) => i.to_string(),
            TextScalar::TimeZone(z) => z.to_string(),
            TextScalar::ZonedTime(z) => z.to_string(),
            TextScalar::Currency(c) => c.to_string(),
            TextScalar::Locale(l) => l.to_string(),
        }
    }
}

/// The names of the text-scalar types, as contracts and SDKs write them.
pub const NAMES: [&str; 7] = ["Decimal", "Duration", "Instant", "TimeZone", "ZonedTime", "Currency", "Locale"];

/// Whether a contract type name is a text scalar.
pub fn is_name(name: &str) -> bool {
    NAMES.contains(&name)
}

/// The type's name when it is a text scalar.
pub fn name(ty: &Type) -> Option<&'static str> {
    Some(match ty {
        Type::Decimal => "Decimal",
        Type::Duration => "Duration",
        Type::Instant => "Instant",
        Type::TimeZone => "TimeZone",
        Type::ZonedTime => "ZonedTime",
        Type::Currency => "Currency",
        Type::Locale => "Locale",
        _ => return None,
    })
}

/// Reads the canonical text of a text-scalar type `ty`. The error
/// says what the text is not, in the type's own words.
pub fn parse(ty: &Type, text: &str) -> Result<TextScalar, String> {
    let problem = |error: &dyn std::fmt::Display| error.to_string();
    match ty {
        Type::Decimal => Decimal::parse(text).map(TextScalar::Decimal).map_err(|e| problem(&e)),
        Type::Duration => Duration::parse(text).map(TextScalar::Duration).map_err(|e| problem(&e)),
        Type::Instant => Instant::parse(text).map(TextScalar::Instant).map_err(|e| problem(&e)),
        Type::TimeZone => TimeZone::parse(text).map(TextScalar::TimeZone).map_err(|e| problem(&e)),
        Type::ZonedTime => ZonedTime::parse(text).map(TextScalar::ZonedTime).map_err(|e| problem(&e)),
        Type::Currency => Currency::parse(text).map(TextScalar::Currency).map_err(|e| problem(&e)),
        Type::Locale => Locale::parse(text).map(TextScalar::Locale).map_err(|e| problem(&e)),
        other => Err(format!("`{}` is not a text-scalar type", other.display_name())),
    }
}

/// How a decode error describes a JSON value that is not a string,
/// for a text scalar. A number for a `Decimal` says why it is refused.
pub fn needs_a_string(ty: &Type, found: &J) -> String {
    match (ty, found) {
        (Type::Decimal, J::Number(n)) => format!(
            "the number {n}; a Decimal is written as a string, such as \"19.99\", so no digit is lost"
        ),
        _ => crate::decode::json_kind(found).to_string(),
    }
}

/// The JSON Schema for a text-scalar type by name, as OpenAPI and
/// prompt schemas publish it. `None` for any other name.
pub fn json_schema_by_name(name: &str) -> Option<J> {
    Some(match name {
        "Decimal" => json!({
            "type": "string",
            "format": "decimal",
            "pattern": "^[+-]?[0-9]+(\\.[0-9]+)?$",
            "description": "an exact decimal number written as a string, such as \"19.99\"",
        }),
        "Duration" => json!({
            "type": "string",
            "format": "duration",
            "description": "an ISO 8601 duration of hours, minutes and seconds, such as PT1H30M",
        }),
        "Instant" => json!({ "type": "string", "format": "date-time" }),
        "TimeZone" => json!({ "type": "string", "description": "an IANA time zone name, such as Europe/Berlin" }),
        "ZonedTime" => json!({
            "type": "string",
            "description": "RFC 9557 zoned time: local time, offset and IANA zone, such as 2026-03-29T03:30:00+02:00[Europe/Berlin]",
        }),
        "Currency" => json!({
            "type": "string",
            "pattern": "^[A-Z]{3}$",
            "description": "an ISO 4217 currency code, such as EUR",
        }),
        "Locale" => json!({ "type": "string", "description": "a BCP 47 locale tag, such as de-DE" }),
        _ => return None,
    })
}

/// The JSON Schema for a text-scalar type.
pub fn json_schema(ty: &Type) -> Option<J> {
    name(ty).and_then(json_schema_by_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_form_round_trips() {
        let cases = [
            (Type::Decimal, "19.90"),
            (Type::Duration, "PT1H30M"),
            (Type::Instant, "2026-07-11T08:30:00.012Z"),
            (Type::TimeZone, "Europe/Berlin"),
            (Type::ZonedTime, "2026-03-29T03:30:00+02:00[Europe/Berlin]"),
            (Type::Currency, "EUR"),
            (Type::Locale, "de-DE"),
        ];
        for (ty, text) in cases {
            let value = parse(&ty, text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(value.to_text(), text);
            assert!(json_schema(&ty).is_some(), "{}", ty.display_name());
        }
    }

    #[test]
    fn a_decimal_number_is_refused_with_the_reason() {
        let message = needs_a_string(&Type::Decimal, &json!(19.99));
        assert!(message.contains("written as a string"), "{message}");
    }

    #[test]
    fn other_types_are_not_text_scalars() {
        assert_eq!(name(&Type::Float), None);
        assert_eq!(json_schema_by_name("Float"), None);
    }
}
