use crate::errors::RuntimeError;
use crate::io::IoRuntime;
use serde_json::{json, Value};
use std::path::Path;

/// An `Option<String>` answer in the public form a tool answers in: the
/// text, or `null`.
fn option_text(value: Option<String>) -> Value {
    match value {
        Some(text) => json!(text),
        None => Value::Null,
    }
}

/// Only the five std/path tool names are handled here.
pub(super) fn dispatch(
    io: &IoRuntime,
    name: &str,
    args: &[Value],
) -> Option<Result<Value, RuntimeError>> {
    use Value::String as Text;
    let result = match (name, args) {
        ("path_join", [Text(base), Text(child)]) => json!(io.join_path(base, child)),
        ("path_parent", [Text(path)]) => option_text(
            io.parent_path(path)
                .map(|p| p.to_string_lossy().into_owned()),
        ),
        ("path_filename", [Text(path)]) => option_text(io.file_name(path)),
        ("path_extension", [Text(path)]) => option_text(io.extension(path)),
        ("path_is_absolute", [Text(path)]) => json!(Path::new(path).is_absolute()),
        (
            "path_join" | "path_parent" | "path_filename" | "path_extension" | "path_is_absolute",
            _,
        ) => {
            return Some(Err(RuntimeError::ToolFailed {
                tool: name.to_owned(),
                message: if name == "path_join" {
                    "expected two String arguments"
                } else {
                    "expected one String argument"
                }
                .into(),
            }))
        }
        _ => return None,
    };
    Some(Ok(result))
}
