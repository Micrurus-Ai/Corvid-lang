//! The encoder: a value of a Corvid type in, public JSON out.
//!
//! The walk follows the type, not the value, so a representation that
//! does not tag its values (native code) reads them the same way the
//! interpreter does, and an empty `Map<Int, V>` still encodes as an
//! array of pairs.

use corvid_ir::IrType;
use corvid_resolve::DefId;
use corvid_types::Type;
use serde_json::{json, Map, Value as J};
use std::collections::HashMap;

/// Reads values of one representation for the encoder. Each method
/// receives a value its type says has that shape; a representation
/// that finds otherwise reports it, and encoding fails with that reason.
pub trait ValueReader {
    type Value;

    fn int(&mut self, value: &Self::Value) -> Result<i64, String>;
    fn float(&mut self, value: &Self::Value) -> Result<f64, String>;
    fn bool(&mut self, value: &Self::Value) -> Result<bool, String>;
    fn string(&mut self, value: &Self::Value) -> Result<String, String>;
    /// The payload of an `Option`, or `None`.
    fn option(&mut self, option: &Type, value: &Self::Value) -> Result<Option<Self::Value>, String>;
    fn list(&mut self, list: &Type, value: &Self::Value) -> Result<Vec<Self::Value>, String>;
    fn map(&mut self, map: &Type, value: &Self::Value) -> Result<Vec<(Self::Value, Self::Value)>, String>;
    /// Whether a `Result` is `Ok`, and its payload.
    fn result(&mut self, result: &Type, value: &Self::Value) -> Result<(bool, Self::Value), String>;
    /// A record's fields in declaration order.
    fn record(&mut self, record: &IrType, value: &Self::Value) -> Result<Vec<Self::Value>, String>;
    /// The variant index of a sum-type value and that variant's fields.
    fn variant(&mut self, sum: &IrType, value: &Self::Value) -> Result<(usize, Vec<Self::Value>), String>;
    /// The inner value and the provenance entries of a `Grounded` value.
    fn grounded(&mut self, grounded: &Type, value: &Self::Value) -> Result<(Self::Value, Vec<J>), String>;
    fn json(&mut self, value: &Self::Value) -> Result<J, String>;
    /// B06 — a `Decimal`, time or currency value of type `ty`.
    fn text_scalar(&mut self, ty: &Type, value: &Self::Value) -> Result<crate::text_scalar::TextScalar, String>;
}

/// Encode `value`, of type `ty`, as public JSON.
pub fn encode<R: ValueReader>(
    value: &R::Value,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    reader: &mut R,
) -> Result<J, String> {
    Encoder { types_by_id, reader }.encode(value, ty)
}

/// Encode `value` as compact public JSON text with sorted object keys:
/// `value.to_json()`.
pub fn encode_text<R: ValueReader>(
    value: &R::Value,
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    reader: &mut R,
) -> Result<String, String> {
    encode(value, ty, types_by_id, reader).map(|json| json.to_string())
}

struct Encoder<'a, R: ValueReader> {
    types_by_id: &'a HashMap<DefId, &'a IrType>,
    reader: &'a mut R,
}

impl<R: ValueReader> Encoder<'_, R> {
    fn encode(&mut self, value: &R::Value, ty: &Type) -> Result<J, String> {
        Ok(match ty {
            Type::Int => J::from(self.reader.int(value)?),
            Type::Float => {
                let f = self.reader.float(value)?;
                if !f.is_finite() {
                    return Err(format!("the Float {f} has no JSON form"));
                }
                J::from(f)
            }
            Type::String => J::String(self.reader.string(value)?),
            Type::Bool => J::Bool(self.reader.bool(value)?),
            Type::Nothing => J::Null,
            Type::Tainted(inner) => self.encode(value, inner)?,
            Type::Option(inner) => match self.reader.option(ty, value)? {
                Some(payload) => self.encode(&payload, inner)?,
                None => J::Null,
            },
            Type::List(element) => {
                let items = self.reader.list(ty, value)?;
                J::Array(items.iter().map(|item| self.encode(item, element)).collect::<Result<_, _>>()?)
            }
            Type::Map(key, item) => {
                let entries = self.reader.map(ty, value)?;
                if matches!(**key, Type::String) {
                    let mut object = Map::new();
                    for (k, v) in &entries {
                        let name = self.reader.string(k)?;
                        object.insert(name, self.encode(v, item)?);
                    }
                    J::Object(object)
                } else {
                    J::Array(
                        entries
                            .iter()
                            .map(|(k, v)| Ok(J::Array(vec![self.encode(k, key)?, self.encode(v, item)?])))
                            .collect::<Result<_, String>>()?,
                    )
                }
            }
            // `tag` repeats which key is present. The contract schema
            // allows it, and existing clients branch on it.
            Type::Result(ok, err) => match self.reader.result(ty, value)? {
                (true, payload) => json!({ "tag": "ok", "ok": self.encode(&payload, ok)? }),
                (false, payload) => json!({ "tag": "err", "err": self.encode(&payload, err)? }),
            },
            Type::Struct(id) | Type::ImportedStruct(corvid_types::types::ImportedStructType { def_id: id, .. }) => {
                let decl = *self
                    .types_by_id
                    .get(id)
                    .ok_or_else(|| format!("the type `{}` has no declaration", ty.display_name()))?;
                if decl.variants.is_empty() {
                    self.record(decl, value)?
                } else {
                    let (index, fields) = self.reader.variant(decl, value)?;
                    let variant = decl
                        .variants
                        .get(index)
                        .ok_or_else(|| format!("`{}` has no variant {index}", decl.name))?;
                    let encoded = variant
                        .fields
                        .iter()
                        .zip(&fields)
                        .map(|(field, field_value)| self.encode(field_value, &field.ty))
                        .collect::<Result<Vec<_>, _>>()?;
                    json!({ "tag": "variant", "type": decl.name, "variant": variant.name, "fields": encoded })
                }
            }
            Type::JsonError => self.record(&crate::json_error_decl(), value)?,
            // The cursor-pagination envelope `{items, next_cursor, has_more}`.
            Type::Page(item) => {
                let span = corvid_ast::Span::new(0, 0);
                // Only the fields are read; the id is never looked up.
                let decl = IrType {
                    id: DefId(0),
                    name: "Page".into(),
                    fields: Type::page_fields(item)
                        .into_iter()
                        .map(|(name, ty)| corvid_ir::IrField { name: name.into(), ty, refinement: None, span })
                        .collect(),
                    variants: Vec::new(),
                    span,
                };
                self.record(&decl, value)?
            }
            Type::Grounded(inner) => {
                let (payload, sources) = self.reader.grounded(ty, value)?;
                json!({ "value": self.encode(&payload, inner)?, "sources": sources })
            }
            Type::JsonValue => self.reader.json(value)?,
            // B06 — exact text, printed by the type's own crate.
            ty if crate::text_scalar::name(ty).is_some() => J::String(self.reader.text_scalar(ty, value)?.to_text()),
            other => return Err(no_json_form(other)),
        })
    }

    fn record(&mut self, decl: &IrType, value: &R::Value) -> Result<J, String> {
        let fields = self.reader.record(decl, value)?;
        let mut object = Map::new();
        for (field, field_value) in decl.fields.iter().zip(&fields) {
            object.insert(field.name.clone(), self.encode(field_value, &field.ty)?);
        }
        Ok(J::Object(object))
    }
}

fn no_json_form(ty: &Type) -> String {
    match ty {
        Type::Weak(_, _) => "a weak reference has no JSON form".into(),
        Type::Function { .. } => "a function has no JSON form".into(),
        Type::DbHandle => "a database handle has no JSON form".into(),
        Type::JsonBuilder => "a JSON builder has no JSON form; call json_object_finish".into(),
        Type::Stream(_) => "a stream has no single JSON form".into(),
        Type::Partial(_) => "a partial value has no JSON form".into(),
        Type::ResumeToken(_) => "a resume token has no JSON form".into(),
        other => format!("`{}` has no JSON form", other.display_name()),
    }
}
