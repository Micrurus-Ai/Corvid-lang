//! Decode failures and the decode policy for undeclared object keys.

use serde_json::{json, Value as J};

/// How a record decode treats object keys that are not declared fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownFields {
    /// Skip undeclared keys. The default for `T.from_json`, because
    /// external APIs add fields without notice.
    Ignore,
    /// Fail on the first undeclared key. Route bodies use this, as
    /// the published contract declares `additionalProperties: false`.
    Reject,
}

/// A decode failure located at a JSON path such as `items[2].price`.
/// The root is the empty path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonDecodeError {
    pub path: String,
    pub expected: String,
    pub found: String,
}

impl JsonDecodeError {
    pub fn new(path: &str, expected: impl Into<String>, found: impl Into<String>) -> Self {
        Self { path: path.to_string(), expected: expected.into(), found: found.into() }
    }

    /// A one-line message for people: ``at `tag`: expected String, found number``.
    pub fn message(&self) -> String {
        if self.path.is_empty() {
            format!("expected {}, found {}", self.expected, self.found)
        } else {
            format!("at `{}`: expected {}, found {}", self.path, self.expected, self.found)
        }
    }

    /// The structured form returned to HTTP clients.
    pub fn to_json(&self) -> J {
        json!({ "path": self.path, "expected": self.expected, "found": self.found })
    }
}

impl std::fmt::Display for JsonDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for JsonDecodeError {}

/// The record declaration of the compiler-known `JsonError`: the
/// reserved [`corvid_types::JSON_ERROR_TYPE_ID`] and the String fields
/// [`corvid_types::json_error_fields`] lists, in that order.
pub fn json_error_decl() -> corvid_ir::IrType {
    let span = corvid_ast::Span::new(0, 0);
    corvid_ir::IrType {
        id: corvid_types::JSON_ERROR_TYPE_ID,
        name: "JsonError".into(),
        fields: corvid_types::json_error_fields()
            .into_iter()
            .map(|(name, ty)| corvid_ir::IrField { name: name.into(), ty, refinement: None, json_key: None, span })
            .collect(),
        variants: Vec::new(),
        span,
    }
}

/// The sum-type declaration of a `Conflict<Row>` (B09): the reserved
/// [`corvid_types::CONFLICT_TYPE_ID`] and one variant per
/// [`corvid_types::ConflictCase`], in order, whose one field, if it has
/// one, is the row. Encoding and decoding read a conflict through it, so
/// it crosses JSON exactly as a declared sum type does.
pub fn conflict_decl(row: &corvid_types::Type) -> corvid_ir::IrType {
    let span = corvid_ast::Span::new(0, 0);
    corvid_ir::IrType {
        id: corvid_types::CONFLICT_TYPE_ID,
        name: "Conflict".into(),
        fields: Vec::new(),
        variants: corvid_types::ConflictCase::ALL
            .into_iter()
            .map(|case| corvid_ir::IrEnumVariant {
                name: case.name().into(),
                fields: case
                    .field()
                    .map(|name| corvid_ir::IrField { name: name.into(), ty: row.clone(), refinement: None, json_key: None, span })
                    .into_iter()
                    .collect(),
                span,
            })
            .collect(),
        span,
    }
}

/// Parse JSON text. Text that is not JSON fails at the root path,
/// naming where the text stopped making sense.
pub fn parse_json_text(text: &str) -> Result<J, JsonDecodeError> {
    serde_json::from_str(text).map_err(|error| {
        let position = format!("line {}, column {}", error.line(), error.column());
        let found = match error.classify() {
            serde_json::error::Category::Eof => format!("text that ends early, at {position}"),
            _ => format!("text that is not valid JSON at {position}"),
        };
        JsonDecodeError::new("", "JSON text", found)
    })
}
