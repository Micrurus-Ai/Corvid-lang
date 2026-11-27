use crate::errors::{InterpError, InterpErrorKind};
use crate::value::Value;
use corvid_ast::Span;
use corvid_ir::IrType;
use corvid_resolve::DefId;
use std::{collections::HashMap, fmt::Write};

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn invalid_form(span: Span) -> InterpError {
    InterpError::new(InterpErrorKind::Other("form requires a declared route or action and a record of form fields".into()), span)
}

/// `form_for("POST /save", initial, label)`: a form for a declared route.
pub(super) fn render(
    target: &str,
    initial: &Value,
    label: &str,
    types: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    let (method, path) = target.split_once(' ').ok_or_else(|| invalid_form(span))?;
    let mut html = format!("<form method=\"{}\" action=\"{}\">", escape(method), escape(path));
    edit_fields(&mut html, initial, types, span)?;
    write!(html, "<button type=\"submit\">{}</button></form>", escape(label)).unwrap();
    Ok(Value::String(html.into()))
}

/// The opening of a form that posts to an action (B10-2b-5). It carries
/// the reserved CSRF field, which the page script fills from the
/// `corvid_csrf` cookie on submit; without the script the field stays
/// empty and the server refuses the post, which fails closed. The
/// `data-corvid-action` attribute is how the page script finds the form.
fn action_form_open(path: &str) -> String {
    let action = path.strip_prefix(corvid_types::action_input::ACTIONS_PATH).unwrap_or(path);
    format!(
        "<form method=\"POST\" action=\"{}\" data-corvid-action=\"{}\"><input type=\"hidden\" name=\"{}\" value=\"\">",
        escape(path),
        escape(action),
        corvid_types::action_input::CSRF_FORM_FIELD
    )
}

/// `form_for(action, initial, label)`: a form editing the action's input.
pub(super) fn render_action(
    path: &str,
    initial: &Value,
    label: &str,
    types: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    let mut html = action_form_open(path);
    edit_fields(&mut html, initial, types, span)?;
    write!(html, "<button type=\"submit\">{}</button></form>", escape(label)).unwrap();
    Ok(Value::String(html.into()))
}

/// `button_for(action, value, label)`: one button that posts `value` as
/// the action's input, in hidden fields. A record travels field by
/// field; a single value travels in `field`, the action's one input.
pub(super) fn render_action_button(
    path: &str,
    value: &Value,
    label: &str,
    field: &str,
    types: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    let mut html = action_form_open(path);
    if field.is_empty() {
        let Value::Struct(values) = value else { return Err(invalid_form(span)) };
        let record = types.get(&values.type_id()).ok_or_else(|| invalid_form(span))?;
        for field in &record.fields {
            let text = field_text(&values.get_field(&field.name).ok_or_else(|| invalid_form(span))?, span)?;
            write!(html, "<input type=\"hidden\" name=\"{}\" value=\"{}\">", escape(&field.name), escape(&text)).unwrap();
        }
    } else {
        let text = field_text(value, span)?;
        write!(html, "<input type=\"hidden\" name=\"{}\" value=\"{}\">", escape(field), escape(&text)).unwrap();
    }
    write!(html, "<button type=\"submit\">{}</button></form>", escape(label)).unwrap();
    Ok(Value::String(html.into()))
}

/// A form value as the server reads it back: the text
/// `text_parameters::query_field_shape` decodes.
fn field_text(value: &Value, span: Span) -> Result<String, InterpError> {
    Ok(match value {
        Value::String(s) => s.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Enum(e) => e.variant_name().to_string(),
        _ => return Err(invalid_form(span)),
    })
}

/// One labelled control per field of `initial`, holding its value.
fn edit_fields(html: &mut String, initial: &Value, types: &HashMap<DefId, &IrType>, span: Span) -> Result<(), InterpError> {
    let Value::Struct(values) = initial else { return Err(invalid_form(span)) };
    let record = types.get(&values.type_id()).ok_or_else(|| invalid_form(span))?;
    for field in &record.fields {
        let name = escape(&field.name);
        write!(html, "<label>{}", name.replace('_', " ")).unwrap();
        match values.get_field(&field.name).ok_or_else(|| invalid_form(span))? {
            Value::String(s) => write!(html, "<input name=\"{name}\" value=\"{}\">", escape(&s)).unwrap(),
            Value::Int(n) => write!(html, "<input required type=\"number\" step=\"1\" name=\"{name}\" value=\"{n}\">").unwrap(),
            Value::Float(n) => write!(html, "<input required type=\"number\" step=\"any\" name=\"{name}\" value=\"{n}\">").unwrap(),
            Value::Bool(b) => {
                write!(html, "<select name=\"{name}\">").unwrap();
                for value in [true, false] {
                    let selected = if value == b { " selected" } else { "" };
                    write!(html, "<option{selected}>{value}</option>").unwrap();
                }
                html.push_str("</select>");
            }
            // A type of plain variants: one option per variant, by name.
            Value::Enum(e) => {
                let choice = types.get(&e.type_id()).ok_or_else(|| invalid_form(span))?;
                write!(html, "<select name=\"{name}\">").unwrap();
                for variant in &choice.variants {
                    let selected = if variant.name == e.variant_name() { " selected" } else { "" };
                    write!(html, "<option{selected}>{}</option>", escape(&variant.name)).unwrap();
                }
                html.push_str("</select>");
            }
            _ => return Err(invalid_form(span)),
        }
        html.push_str("</label>");
    }
    Ok(())
}

pub(super) fn browser_fields(
    initial: &Value,
    types: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    let mut fields = Vec::new();
    collect_browser_fields(initial, types, span, "", &mut fields)?;
    Ok(Value::Map(crate::value::MapValue::new(fields)))
}

fn collect_browser_fields(
    initial: &Value,
    types: &HashMap<DefId, &IrType>,
    span: Span,
    prefix: &str,
    fields: &mut Vec<(Value, Value)>,
) -> Result<(), InterpError> {
    use crate::value::ListValue;
    let invalid = || {
        InterpError::new(
            InterpErrorKind::Other("expected a bounded integer record for browser state".into()),
            span,
        )
    };
    let Value::Struct(values) = initial else {
        return Err(invalid());
    };
    let record = types.get(&values.type_id()).ok_or_else(invalid)?;
    for field in &record.fields {
        let path = format!("{prefix}{}", field.name);
        let value = values.get_field(&field.name).ok_or_else(invalid)?;
        let row = match value {
            Value::Struct(_) => {
                collect_browser_fields(&value, types, span, &format!("{path}."), fields)?;
                continue;
            }
            Value::Int(n) => {
                let Some(corvid_ast::Refinement::Between { min, max }) = field.refinement else {
                    return Err(invalid());
                };
                vec![
                    "integer".into(),
                    n.to_string(),
                    min.to_string(),
                    max.to_string(),
                ]
            }
            value => {
                use serde_json::json;
                let (schema, initial) = match value {
                    Value::String(s) => {
                        let mut schema = json!({"type":"string"});
                        if let Some(corvid_ast::Refinement::LenBetween { min, max }) =
                            field.refinement
                        {
                            schema["minLength"] = json!(min);
                            schema["maxLength"] = json!(max);
                        }
                        (schema, json!(s.as_ref()))
                    }
                    Value::Bool(b) => (json!({"type":"boolean"}), json!(b)),
                    Value::Enum(e) => {
                        let choice = types.get(&e.type_id()).ok_or_else(invalid)?;
                        let names: Vec<_> = choice.variants.iter().map(|v| &v.name).collect();
                        (
                            json!({"type":"string","enum":names}),
                            json!(e.variant_name()),
                        )
                    }
                    _ => return Err(invalid()),
                };
                vec![
                    "field".into(),
                    json!({"name":path,"schema":schema,"value":initial}).to_string(),
                ]
            }
        };
        fields.push((
            Value::String(path.into()),
            Value::List(ListValue::new(
                row.into_iter().map(|text| Value::String(text.into())),
            )),
        ));
    }
    Ok(())
}
