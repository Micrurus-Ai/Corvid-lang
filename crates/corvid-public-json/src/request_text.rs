//! Request text as public JSON: a path segment or query value becomes
//! the JSON its type's public mapping reads, and the decoder does the
//! rest. So a `Decimal` in a query is read by the `Decimal` parser, a
//! plain variant by its name, and an `Int` refuses `7.5`, each exactly
//! as in a JSON body, with no second parser to drift.

use corvid_types::text_parameters::TextShape;
use serde_json::{json, Value as J};

/// The public JSON for `raw` written as a value of `shape`, or why it
/// is not one, as "`raw` is not ...".
pub fn request_text_json(raw: &str, shape: TextShape) -> Result<J, String> {
    let not = |what: &str| format!("`{raw}` is not {what}");
    Ok(match shape {
        // Digits with an optional minus: no `+`, spaces, point or exponent.
        TextShape::Int => {
            let digits = raw.strip_prefix('-').unwrap_or(raw);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(not("an Int"));
            }
            J::from(raw.parse::<i64>().map_err(|_| not("an Int: it is outside the Int range"))?)
        }
        TextShape::Float => {
            let value = raw.parse::<f64>().ok().filter(|f| f.is_finite() && !raw.starts_with('+'));
            J::from(value.ok_or_else(|| not("a Float"))?)
        }
        TextShape::Bool => match raw {
            "true" => J::Bool(true),
            "false" => J::Bool(false),
            _ => return Err(not("a Bool: write `true` or `false`")),
        },
        TextShape::String | TextShape::Exact => J::String(raw.to_string()),
        // The decoder checks the name against the type's variants.
        TextShape::PlainVariant => json!({ "variant": raw, "fields": [] }),
    })
}

/// The request text for one value's public JSON: the reverse of
/// [`request_text_json`], which a connector uses to write a query
/// parameter exactly as a route reads one in. `None` for JSON that is
/// not one piece of request text (an object with fields, a list).
pub fn json_request_text(json: &J) -> Option<String> {
    Some(match json {
        J::String(text) => text.clone(),
        J::Number(number) => number.to_string(),
        J::Bool(flag) => flag.to_string(),
        // A plain variant: the variant's name.
        J::Object(object) => {
            let fields_empty = object.get("fields").map_or(true, |f| f.as_array().is_some_and(Vec::is_empty));
            match object.get("variant") {
                Some(J::String(name)) if fields_empty => name.clone(),
                _ => return None,
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_then_reading_request_text_gives_back_the_json() {
        let cases = [
            (json!(42), TextShape::Int),
            (json!(-2.5), TextShape::Float),
            (json!(true), TextShape::Bool),
            (json!("19.90"), TextShape::Exact),
            (json!({ "variant": "Open", "fields": [] }), TextShape::PlainVariant),
        ];
        for (json, shape) in cases {
            let text = json_request_text(&json).unwrap_or_else(|| panic!("{json} is request text"));
            assert_eq!(request_text_json(&text, shape), Ok(json.clone()), "{text}");
        }
        assert_eq!(json_request_text(&json!({ "variant": "Done", "fields": [3] })), None, "a variant with fields");
        assert_eq!(json_request_text(&json!([1])), None);
    }

    #[test]
    fn an_int_is_digits_with_an_optional_minus() {
        assert_eq!(request_text_json("42", TextShape::Int), Ok(json!(42)));
        assert_eq!(request_text_json("-7", TextShape::Int), Ok(json!(-7)));
        for bad in ["+7", " 7", "7.0", "1e3", "", "-", "seven"] {
            assert!(request_text_json(bad, TextShape::Int).is_err(), "`{bad}`");
        }
        let overflow = request_text_json("9223372036854775808", TextShape::Int).unwrap_err();
        assert!(overflow.contains("outside the Int range"), "{overflow}");
    }

    #[test]
    fn a_float_is_finite_and_a_bool_is_spelled_out() {
        assert_eq!(request_text_json("2.5", TextShape::Float), Ok(json!(2.5)));
        for bad in ["NaN", "inf", "+2.5", "two"] {
            assert!(request_text_json(bad, TextShape::Float).is_err(), "`{bad}`");
        }
        assert_eq!(request_text_json("true", TextShape::Bool), Ok(J::Bool(true)));
        assert!(request_text_json("1", TextShape::Bool).is_err());
    }

    #[test]
    fn exact_text_and_variants_pass_to_the_decoder_as_written() {
        assert_eq!(request_text_json("19.90", TextShape::Exact), Ok(json!("19.90")));
        assert_eq!(
            request_text_json("Open", TextShape::PlainVariant),
            Ok(json!({ "variant": "Open", "fields": [] }))
        );
    }
}
