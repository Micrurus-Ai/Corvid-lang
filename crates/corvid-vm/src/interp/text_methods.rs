//! The B03 text builtin methods on the interpreter tier: encoders,
//! decoders, UUID parsing and regular expressions. Every rule is
//! `corvid-text`'s, the same code the native runtime calls, so this
//! file only moves values in and out of `Value`.

use crate::errors::{InterpError, InterpErrorKind};
use crate::value::{BoxedValue, ListValue, StructValue, Value};
use corvid_ast::Span;
use corvid_types::BuiltinMethodKind;
use std::sync::Arc;

fn string(text: impl Into<Arc<str>>) -> Value {
    Value::String(text.into())
}

/// Evaluate a text encoder or decoder on the receiver `text`.
pub(super) fn eval_text_method(
    kind: BuiltinMethodKind,
    text: &str,
    span: Span,
) -> Result<Value, InterpError> {
    use BuiltinMethodKind::*;
    let result = |r: Result<String, String>| match r {
        Ok(text) => Value::ResultOk(BoxedValue::new(string(text))),
        Err(message) => Value::ResultErr(BoxedValue::new(string(message))),
    };
    Ok(match kind {
        StringEncodeBase64 => string(corvid_text::encode_base64(text)),
        StringEncodeBase64Url => string(corvid_text::encode_base64_url(text)),
        StringEncodeHex => string(corvid_text::encode_hex(text)),
        StringEncodeUrlComponent => string(corvid_text::encode_url_component(text)),
        StringDecodeBase64 => result(corvid_text::decode_base64(text)),
        StringDecodeBase64Url => result(corvid_text::decode_base64_url(text)),
        StringDecodeHex => result(corvid_text::decode_hex(text)),
        StringDecodeUrlComponent => result(corvid_text::decode_url_component(text)),
        StringParseUuid => result(corvid_text::parse_uuid(text)),
        other => return Err(not_a_text_method(other, span)),
    })
}

/// Evaluate a regex method on the receiver `text`. `args[0]` is the
/// pattern the checker validated; `replace_regex` also takes the
/// replacement.
pub(super) fn eval_regex_method(
    kind: BuiltinMethodKind,
    text: &str,
    args: &[Value],
    span: Span,
) -> Result<Value, InterpError> {
    use corvid_text::regex;
    use BuiltinMethodKind::*;
    let argument = |index: usize| match args.get(index) {
        Some(Value::String(s)) => Ok(s.clone()),
        other => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "String".into(),
                got: other.map_or_else(|| "no argument".into(), Value::type_name),
            },
            span,
        )),
    };
    let pattern = argument(0)?;
    let internal = |message: String| InterpError::new(InterpErrorKind::Other(message), span);
    Ok(match kind {
        StringContainsRegex => Value::Bool(regex::contains(text, &pattern).map_err(internal)?),
        StringFindRegex => match regex::find(text, &pattern).map_err(internal)? {
            Some(found) => Value::OptionSome(BoxedValue::new(regex_match_value(found))),
            None => Value::OptionNone,
        },
        StringFindAllRegex => Value::List(ListValue::new(
            regex::find_all(text, &pattern).map_err(internal)?.into_iter().map(regex_match_value),
        )),
        StringReplaceRegex => string(regex::replace(text, &pattern, &argument(1)?).map_err(internal)?),
        StringReplaceRegexVerbatim => {
            string(regex::replace_verbatim(text, &pattern, &argument(1)?).map_err(internal)?)
        }
        StringSplitRegex => Value::List(ListValue::new(
            regex::split(text, &pattern).map_err(internal)?.into_iter().map(string),
        )),
        other => return Err(not_a_text_method(other, span)),
    })
}

/// The compiler-known `RegexMatch` record, with the fields
/// `corvid_types::regex_match_fields` declares, in that order.
fn regex_match_value(found: corvid_text::regex::RegexMatch) -> Value {
    let corvid_text::regex::RegexMatch { text, start, end, groups } = found;
    let mut text = Some(text);
    let mut groups = Some(groups);
    let fields = corvid_types::regex_match_fields().map(|(name, _)| {
        let value = match name {
            "text" => string(text.take().unwrap_or_default()),
            "start" => Value::Int(start),
            "end" => Value::Int(end),
            "groups" => Value::List(ListValue::new(groups.take().unwrap_or_default().into_iter().map(
                |group| match group {
                    Some(group) => Value::OptionSome(BoxedValue::new(string(group))),
                    None => Value::OptionNone,
                },
            ))),
            other => unreachable!("RegexMatch field `{other}` has no runtime value"),
        };
        (name.to_string(), value)
    });
    Value::Struct(StructValue::new(corvid_types::REGEX_MATCH_TYPE_ID, "RegexMatch", fields))
}

fn not_a_text_method(kind: BuiltinMethodKind, span: Span) -> InterpError {
    InterpError::new(
        InterpErrorKind::Other(format!("`{kind:?}` is not a text method")),
        span,
    )
}
