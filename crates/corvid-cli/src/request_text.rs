//! Typed values from request text: a query string, a form body or
//! command-line flags. Every field or parameter is read by the one
//! request-text rule (`corvid_vm::public_json::decode_request_text`):
//! a text type once, an `Option` left out when absent, a `List` by
//! repeating the name. `corvid serve` and `corvid act` both read
//! through here.

use std::collections::HashMap;

use corvid_driver::Value;
use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;

/// Parse a URL query string (`a=1&b=x`) into the declared query struct
/// value, coercing each field from its string form (slice 52a).
pub(crate) fn urlencoded_record(
    encoded: &str,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    source: &str,
) -> Result<Value, String> {
    let def_id = match ty {
        Type::Struct(id) => id,
        Type::ImportedStruct(record) => &record.def_id,
        _ => return Err(format!("{source} type must be a struct")),
    };
    let ir_type = types_by_id
        .get(def_id)
        .copied()
        .ok_or_else(|| format!("unknown {source} struct type"))?;
    let given = urlencoded_values(encoded)?;
    let mut fields: Vec<(String, Value)> = Vec::new();
    for field in &ir_type.fields {
        let value = urlencoded_value(&given, &field.name, &field.ty, types_by_id, source)?;
        corvid_vm::conv::check_field_refinement(&ir_type.name, field, &value)
            .map_err(|err| err.to_string())?;
        fields.push((field.name.clone(), value));
    }
    Ok(Value::Struct(corvid_driver::StructValue::new(
        ir_type.id,
        ir_type.name.clone(),
        fields,
    )))
}

/// Every value under each name of form-encoded text, in order: a `List`
/// reads them all, and a name of one value refuses being given twice.
pub(crate) fn urlencoded_values(encoded: &str) -> Result<HashMap<String, Vec<String>>, String> {
    let mut given: HashMap<String, Vec<String>> = HashMap::new();
    for pair in encoded.split('&').filter(|s| !s.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        given.entry(urldecode(key)?).or_default().push(urldecode(value)?);
    }
    Ok(given)
}

/// One named value of form-encoded text, read as `ty` by the request
/// text rule: a text type is required once, an `Option` of one may be
/// left out, and a `List` of one repeats the name. A record's fields and
/// an action's parameters both read through it.
pub(crate) fn urlencoded_value(
    given: &HashMap<String, Vec<String>>,
    name: &str,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    source: &str,
) -> Result<Value, String> {
    use corvid_types::text_parameters::QueryFieldShape;
    let plain = |ty: &Type| corvid_vm::public_json::has_plain_variants(ty, types_by_id);
    let read = |raw: &str, ty: &Type| {
        coerce_str_to_value(raw, ty, types_by_id).map_err(|e| format!("{source} param `{name}`: {e}"))
    };
    let values = given.get(name).map(Vec::as_slice).unwrap_or(&[]);
    let once = || -> Result<Option<String>, String> {
        match values {
            [] => Ok(None),
            [one] => Ok(Some(one.clone())),
            _ => Err(format!("{source} param `{name}` is given {} times, but holds one value", values.len())),
        }
    };
    let cannot = || format!("{source} param `{name}` has a type a query string cannot carry");
    match (corvid_types::text_parameters::query_field_shape(ty, &plain).ok_or_else(cannot)?, ty) {
        (QueryFieldShape::One(_), ty) => {
            let raw = once()?.ok_or_else(|| format!("missing {source} param `{name}`"))?;
            read(&raw, ty)
        }
        (QueryFieldShape::Optional(_), Type::Option(inner)) => Ok(match once()? {
            Some(raw) => Value::OptionSome(corvid_vm::value::BoxedValue::new(read(&raw, inner)?)),
            None => Value::OptionNone,
        }),
        (QueryFieldShape::Repeated(_), Type::List(inner)) => {
            let items = values.iter().map(|raw| read(raw, inner)).collect::<Result<Vec<_>, _>>()?;
            Ok(Value::List(corvid_driver::ListValue::new(items)))
        }
        _ => Err(cannot()),
    }
}

/// Decode form-encoded query text as UTF-8 after expanding percent escapes.
pub(crate) fn urldecode(s: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => decoded.push(b' '),
            b'%' => {
                let mut digit = || bytes.next().and_then(|b| char::from(b).to_digit(16))
                    .ok_or_else(|| "invalid percent escape in query".to_string());
                decoded.push((digit()? * 16 + digit()?) as u8);
            }
            other => decoded.push(other),
        }
    }
    String::from_utf8(decoded).map_err(|_| "query parameter must be UTF-8".into())
}

pub(crate) fn coerce_str_to_value(raw: &str, ty: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Result<Value, String> {
    corvid_vm::public_json::decode_request_text(raw, ty, types_by_id)
        .unwrap_or_else(|| Err(format!("`{}` cannot be read from request text", ty.display_name())))
}
