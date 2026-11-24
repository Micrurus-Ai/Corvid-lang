//! A decode plan: everything a builder outside the compiler needs to
//! decode one target type, written as text at compile time and read
//! back at run time.
//!
//! The native backend embeds one plan per `T.from_json` site. The plan
//! carries the target type, the declarations it reaches, and an index
//! into a table of allocation metadata for every shape the builder
//! allocates, so the runtime never decides a native layout itself.

use corvid_ast::{Refinement, Span};
use corvid_ir::{IrEnumVariant, IrField, IrType};
use corvid_resolve::DefId;
use corvid_types::types::ImportedStructType;
use corvid_types::Type;
use serde_json::{json, Value as J};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A shape a builder allocates while decoding.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Shape {
    /// An `Option`, `List`, `Map` or `Result` type, exactly as the
    /// decoder passes it to the builder.
    Type(Type),
    /// A record declaration.
    Record(DefId),
    /// One variant of a sum-type declaration.
    Variant(DefId, u32),
}

/// One target type with what is needed to build it.
#[derive(Debug, Clone)]
pub struct DecodePlan {
    pub root: Type,
    /// Every declaration the root reaches, in first-reached order.
    pub types: Vec<IrType>,
    /// The allocation-table index of each shape that has metadata.
    pub shapes: BTreeMap<Shape, u32>,
    /// The `Map` types whose values are reference counted.
    pub owning_maps: Vec<Type>,
}

impl DecodePlan {
    pub fn types_by_id(&self) -> HashMap<DefId, &IrType> {
        self.types.iter().map(|ty| (ty.id, ty)).collect()
    }

    pub fn to_text(&self) -> Result<String, String> {
        let types = self.types.iter().map(write_decl).collect::<Result<Vec<_>, _>>()?;
        let shapes = self
            .shapes
            .iter()
            .map(|(shape, index)| Ok(json!({ "shape": write_shape(shape)?, "index": index })))
            .collect::<Result<Vec<_>, String>>()?;
        let maps = self.owning_maps.iter().map(write_type).collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "root": write_type(&self.root)?, "types": types, "shapes": shapes, "owning_maps": maps })
            .to_string())
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let json: J = serde_json::from_str(text).map_err(|e| format!("decode plan is not JSON: {e}"))?;
        let root = read_type(member(&json, "root")?)?;
        let types = array(&json, "types")?.iter().map(read_decl).collect::<Result<_, _>>()?;
        let shapes = array(&json, "shapes")?
            .iter()
            .map(|entry| {
                let index = member(entry, "index")?.as_u64().ok_or("shape index is not a number")?;
                Ok((read_shape(member(entry, "shape")?)?, index as u32))
            })
            .collect::<Result<_, String>>()?;
        let owning_maps = array(&json, "owning_maps")?.iter().map(read_type).collect::<Result<_, _>>()?;
        Ok(Self { root, types, shapes, owning_maps })
    }
}

/// The declarations `root` reaches, in first-reached order.
pub fn reachable_types<'a>(root: &Type, types_by_id: &HashMap<DefId, &'a IrType>) -> Vec<&'a IrType> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    collect_decls(root, types_by_id, &mut seen, &mut out);
    out
}

fn collect_decls<'a>(
    ty: &Type,
    types_by_id: &HashMap<DefId, &'a IrType>,
    seen: &mut HashSet<DefId>,
    out: &mut Vec<&'a IrType>,
) {
    match ty {
        Type::Option(inner) | Type::List(inner) | Type::Tainted(inner) => collect_decls(inner, types_by_id, seen, out),
        Type::Map(key, value) | Type::Result(key, value) => {
            collect_decls(key, types_by_id, seen, out);
            collect_decls(value, types_by_id, seen, out);
        }
        Type::Struct(id) | Type::ImportedStruct(ImportedStructType { def_id: id, .. }) => {
            let Some(decl) = types_by_id.get(id).copied() else { return };
            if !seen.insert(*id) {
                return;
            }
            out.push(decl);
            for field in decl.fields.iter().chain(decl.variants.iter().flat_map(|v| &v.fields)) {
                collect_decls(&field.ty, types_by_id, seen, out);
            }
        }
        _ => {}
    }
}

/// Every shape the decoder may ask a builder to allocate for `root`,
/// following the decoder's own recursion: `Tainted<T>` is decoded as
/// `T`, and records and variants are reached through their fields.
pub fn builder_shapes(root: &Type, types_by_id: &HashMap<DefId, &IrType>) -> Vec<Shape> {
    let mut seen = HashSet::new();
    let mut decls = HashSet::new();
    let mut out = Vec::new();
    collect_shapes(root, types_by_id, &mut seen, &mut decls, &mut out);
    out
}

fn collect_shapes(
    ty: &Type,
    types_by_id: &HashMap<DefId, &IrType>,
    seen: &mut HashSet<Shape>,
    decls: &mut HashSet<DefId>,
    out: &mut Vec<Shape>,
) {
    fn push(shape: Shape, seen: &mut HashSet<Shape>, out: &mut Vec<Shape>) {
        if seen.insert(shape.clone()) {
            out.push(shape);
        }
    }
    match ty {
        Type::Tainted(inner) => collect_shapes(inner, types_by_id, seen, decls, out),
        Type::Option(inner) | Type::List(inner) => {
            push(Shape::Type(ty.clone()), seen, out);
            collect_shapes(inner, types_by_id, seen, decls, out);
        }
        Type::Map(key, value) | Type::Result(key, value) => {
            push(Shape::Type(ty.clone()), seen, out);
            collect_shapes(key, types_by_id, seen, decls, out);
            collect_shapes(value, types_by_id, seen, decls, out);
        }
        Type::Struct(id) | Type::ImportedStruct(ImportedStructType { def_id: id, .. }) => {
            let Some(decl) = types_by_id.get(id).copied() else { return };
            if !decls.insert(*id) {
                return;
            }
            if decl.variants.is_empty() {
                push(Shape::Record(*id), seen, out);
                for field in &decl.fields {
                    collect_shapes(&field.ty, types_by_id, seen, decls, out);
                }
            } else {
                for (index, variant) in decl.variants.iter().enumerate() {
                    push(Shape::Variant(*id, index as u32), seen, out);
                    for field in &variant.fields {
                        collect_shapes(&field.ty, types_by_id, seen, decls, out);
                    }
                }
            }
        }
        _ => {}
    }
}

fn member<'a>(json: &'a J, key: &str) -> Result<&'a J, String> {
    json.get(key).ok_or_else(|| format!("decode plan has no `{key}`"))
}

fn array<'a>(json: &'a J, key: &str) -> Result<&'a Vec<J>, String> {
    member(json, key)?.as_array().ok_or_else(|| format!("decode plan `{key}` is not an array"))
}

fn text<'a>(json: &'a J, key: &str) -> Result<&'a str, String> {
    member(json, key)?.as_str().ok_or_else(|| format!("decode plan `{key}` is not text"))
}

fn id(json: &J, key: &str) -> Result<DefId, String> {
    let n = member(json, key)?.as_u64().ok_or_else(|| format!("decode plan `{key}` is not an id"))?;
    u32::try_from(n).map(DefId).map_err(|_| format!("decode plan `{key}` is out of range"))
}

fn write_type(ty: &Type) -> Result<J, String> {
    Ok(match ty {
        Type::Int => json!("Int"),
        Type::Float => json!("Float"),
        Type::String => json!("String"),
        Type::Bool => json!("Bool"),
        Type::Nothing => json!("Nothing"),
        Type::JsonValue => json!("JsonValue"),
        // B06 — native code holds every text-scalar type as its
        // canonical text.
        Type::Decimal => json!("Decimal"),
        Type::Duration => json!("Duration"),
        Type::Instant => json!("Instant"),
        Type::TimeZone => json!("TimeZone"),
        Type::ZonedTime => json!("ZonedTime"),
        Type::Currency => json!("Currency"),
        Type::Locale => json!("Locale"),
        Type::Option(inner) => json!({ "Option": write_type(inner)? }),
        Type::List(inner) => json!({ "List": write_type(inner)? }),
        Type::Tainted(inner) => json!({ "Tainted": write_type(inner)? }),
        Type::Map(key, value) => json!({ "Map": [write_type(key)?, write_type(value)?] }),
        Type::Result(ok, err) => json!({ "Result": [write_type(ok)?, write_type(err)?] }),
        Type::Struct(id) => json!({ "Struct": id.0 }),
        Type::ImportedStruct(imported) => json!({
            "Imported": { "module": imported.module_path, "id": imported.def_id.0, "name": imported.name }
        }),
        other => return Err(format!("`{}` has no decode plan form", other.display_name())),
    })
}

fn read_type(json: &J) -> Result<Type, String> {
    if let Some(name) = json.as_str() {
        return Ok(match name {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "String" => Type::String,
            "Bool" => Type::Bool,
            "Nothing" => Type::Nothing,
            "JsonValue" => Type::JsonValue,
            "Decimal" => Type::Decimal,
            "Duration" => Type::Duration,
            "Instant" => Type::Instant,
            "TimeZone" => Type::TimeZone,
            "ZonedTime" => Type::ZonedTime,
            "Currency" => Type::Currency,
            "Locale" => Type::Locale,
            other => return Err(format!("decode plan names an unknown type `{other}`")),
        });
    }
    let J::Object(object) = json else { return Err(format!("decode plan type `{json}` is malformed")) };
    let (kind, body) = object.iter().next().ok_or("decode plan has an empty type")?;
    let pair = |body: &J| -> Result<(Type, Type), String> {
        match body.as_array().map(Vec::as_slice) {
            Some([a, b]) => Ok((read_type(a)?, read_type(b)?)),
            _ => Err(format!("decode plan `{kind}` needs two types")),
        }
    };
    Ok(match kind.as_str() {
        "Option" => Type::Option(Box::new(read_type(body)?)),
        "List" => Type::List(Box::new(read_type(body)?)),
        "Tainted" => Type::Tainted(Box::new(read_type(body)?)),
        "Map" => {
            let (key, value) = pair(body)?;
            Type::Map(Box::new(key), Box::new(value))
        }
        "Result" => {
            let (ok, err) = pair(body)?;
            Type::Result(Box::new(ok), Box::new(err))
        }
        "Struct" => Type::Struct(DefId(body.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("bad struct id")?)),
        "Imported" => Type::ImportedStruct(ImportedStructType {
            module_path: text(body, "module")?.to_string(),
            def_id: id(body, "id")?,
            name: text(body, "name")?.to_string(),
        }),
        other => return Err(format!("decode plan names an unknown type kind `{other}`")),
    })
}

fn write_shape(shape: &Shape) -> Result<J, String> {
    Ok(match shape {
        Shape::Type(ty) => json!({ "type": write_type(ty)? }),
        Shape::Record(id) => json!({ "record": id.0 }),
        Shape::Variant(id, index) => json!({ "variant": [id.0, index] }),
    })
}

fn read_shape(json: &J) -> Result<Shape, String> {
    if let Some(ty) = json.get("type") {
        return Ok(Shape::Type(read_type(ty)?));
    }
    if json.get("record").is_some() {
        return Ok(Shape::Record(id(json, "record")?));
    }
    match json.get("variant").and_then(J::as_array).map(Vec::as_slice) {
        Some([owner, index]) => {
            let owner = owner.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("bad variant owner")?;
            let index = index.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("bad variant index")?;
            Ok(Shape::Variant(DefId(owner), index))
        }
        _ => Err(format!("decode plan shape `{json}` is malformed")),
    }
}

fn write_fields(fields: &[IrField]) -> Result<J, String> {
    fields
        .iter()
        .map(|field| {
            let refinement = match field.refinement {
                None => J::Null,
                Some(Refinement::Between { min, max }) => json!({ "between": [min, max] }),
                Some(Refinement::LenBetween { min, max }) => json!({ "len_between": [min, max] }),
            };
            Ok(json!({ "name": field.name, "type": write_type(&field.ty)?, "refinement": refinement }))
        })
        .collect::<Result<Vec<_>, String>>()
        .map(J::Array)
}

fn read_fields(json: &J) -> Result<Vec<IrField>, String> {
    let span = Span::new(0, 0);
    json.as_array()
        .ok_or("decode plan fields are not an array")?
        .iter()
        .map(|field| {
            let range = |key: &str| -> Option<(i64, i64)> {
                match field.get("refinement")?.get(key)?.as_array()?.as_slice() {
                    [min, max] => Some((min.as_i64()?, max.as_i64()?)),
                    _ => None,
                }
            };
            let refinement = if let Some((min, max)) = range("between") {
                Some(Refinement::Between { min, max })
            } else {
                range("len_between").map(|(min, max)| Refinement::LenBetween { min: min as u64, max: max as u64 })
            };
            Ok(IrField { name: text(field, "name")?.to_string(), ty: read_type(member(field, "type")?)?, refinement, span })
        })
        .collect()
}

fn write_decl(decl: &IrType) -> Result<J, String> {
    let variants = decl
        .variants
        .iter()
        .map(|variant| Ok(json!({ "name": variant.name, "fields": write_fields(&variant.fields)? })))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!({ "id": decl.id.0, "name": decl.name, "fields": write_fields(&decl.fields)?, "variants": variants }))
}

fn read_decl(json: &J) -> Result<IrType, String> {
    let span = Span::new(0, 0);
    let variants = array(json, "variants")?
        .iter()
        .map(|variant| {
            Ok(IrEnumVariant {
                name: text(variant, "name")?.to_string(),
                fields: read_fields(member(variant, "fields")?)?,
                span,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(IrType {
        id: id(json, "id")?,
        name: text(json, "name")?.to_string(),
        fields: read_fields(member(json, "fields")?)?,
        variants,
        span,
    })
}

#[cfg(test)]
mod tests;
