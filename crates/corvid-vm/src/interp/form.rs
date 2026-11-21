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

pub(super) fn render(
    target: &str,
    initial: &Value,
    label: &str,
    types: &HashMap<DefId, &IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    let invalid = || {
        InterpError::new(
            InterpErrorKind::Other("form requires a declared route and scalar record".into()),
            span,
        )
    };
    let (method, path) = target.split_once(' ').ok_or_else(invalid)?;
    let Value::Struct(values) = initial else {
        return Err(invalid());
    };
    let record = types.get(&values.type_id()).ok_or_else(invalid)?;
    let mut html = format!(
        "<form method=\"{}\" action=\"{}\">",
        escape(method),
        escape(path)
    );
    for field in &record.fields {
        let name = escape(&field.name);
        write!(html, "<label>{}", name.replace('_', " ")).unwrap();
        match values.get_field(&field.name).ok_or_else(invalid)? {
            Value::String(s) => {
                write!(html, "<input name=\"{name}\" value=\"{}\">", escape(&s)).unwrap()
            }
            Value::Int(n) => write!(
                html,
                "<input required type=\"number\" step=\"1\" name=\"{name}\" value=\"{n}\">"
            )
            .unwrap(),
            Value::Float(n) => write!(
                html,
                "<input required type=\"number\" step=\"any\" name=\"{name}\" value=\"{n}\">"
            )
            .unwrap(),
            Value::Bool(b) => {
                write!(html, "<select name=\"{name}\">").unwrap();
                for value in [true, false] {
                    let selected = if value == b { " selected" } else { "" };
                    write!(html, "<option{selected}>{value}</option>").unwrap();
                }
                html.push_str("</select>");
            }
            _ => return Err(invalid()),
        }
        html.push_str("</label>");
    }
    write!(
        html,
        "<button type=\"submit\">{}</button></form>",
        escape(label)
    )
    .unwrap();
    Ok(Value::String(html.into()))
}
