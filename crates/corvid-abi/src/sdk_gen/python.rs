//! Python models from the Application Contract.
//!
//! Records become dataclasses and sum types become a base class with
//! one dataclass per variant. Every model has `from_json` and
//! `to_json`, which follow the public JSON mapping exactly: a sum-type
//! value is `{"tag": "variant", "type": ..., "variant": ..., "fields":
//! [...]}` with positional fields, a `Result` is `{"tag": "ok", "ok":
//! ...}` or `{"tag": "err", "err": ...}`, and a map whose keys are not
//! `String` is an array of `[key, value]` pairs. The module describes
//! each type once, in `_RECORDS` and `_SUMS`, and one generic codec
//! walks those descriptions.

use super::type_name::TypeName;
use super::NOTE_SCAFFOLD;
use crate::app_contract::{ApplicationContract, ContractField};
use corvid_types::Type;

/// A field's name and contract type name.
type Field = (String, String);

fn fields_of(fields: &[ContractField]) -> Vec<Field> {
    fields.iter().map(|f| (f.name.clone(), f.type_name.clone())).collect()
}

pub(super) fn emit_python(contract: &ApplicationContract) -> String {
    let mut out = String::from("\"\"\"");
    out.push_str(NOTE_SCAFFOLD);
    out.push_str("\"\"\"\nfrom __future__ import annotations\n\nimport math\nfrom dataclasses import dataclass\nfrom typing import Any, Generic, Optional, TypeVar, Union\n\nT = TypeVar(\"T\")\nE = TypeVar(\"E\")\n\n");
    out.push_str(BUILTIN_MODELS);
    out.push_str(&json_error_model());
    out.push_str(&page_model());

    let mut records = vec![record_entry("JsonError", &json_error_fields())];
    let mut sums = Vec::new();
    for ty in &contract.types {
        if ty.variants.is_empty() {
            out.push_str(&format!("@dataclass\nclass {}:\n", ty.name));
            for field in &ty.fields {
                out.push_str(&format!("    {}: {}\n", field.name, annotation(&TypeName::parse(&field.type_name))));
            }
            out.push_str(&model_methods(&ty.name));
            records.push(record_entry(&ty.name, &fields_of(&ty.fields)));
        } else {
            out.push_str(&format!(
                "class {name}:\n    \"\"\"One of: {variants}.\"\"\"\n{methods}",
                name = ty.name,
                variants = ty.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", "),
                methods = model_methods(&ty.name),
            ));
            // Variant classes carry the sum's name so two sum types may
            // share a variant name; `RefundError.ApprovalDenied` aliases
            // `RefundErrorApprovalDenied`.
            for variant in &ty.variants {
                let class = format!("{}{}", ty.name, variant.name);
                out.push_str(&format!("@dataclass\nclass {class}({}):\n", ty.name));
                if variant.fields.is_empty() {
                    out.push_str("    pass\n");
                }
                for field in &variant.fields {
                    out.push_str(&format!("    {}: {}\n", field.name, annotation(&TypeName::parse(&field.type_name))));
                }
                out.push_str(&format!("\n\n{}.{} = {class}\n\n\n", ty.name, variant.name));
            }
            sums.push(sum_entry(&ty.name, &ty.variants));
        }
    }
    out.push_str("_RECORDS: dict[str, tuple[type, list[tuple[str, Any]]]] = {\n");
    for entry in &records {
        out.push_str(entry);
    }
    out.push_str("}\n\n_SUMS: dict[str, dict[str, tuple[type, list[Any]]]] = {\n");
    for entry in &sums {
        out.push_str(entry);
    }
    out.push_str("}\n\n");
    // B06 — the types whose JSON is their exact canonical text; the SDK
    // keeps that text as a `str` (see `corvid_public_json::text_scalar`).
    out.push_str(&format!(
        "_TEXT_SCALARS = ({})\n\n",
        corvid_public_json::text_scalar::NAMES.iter().map(|name| format!("{name:?}, ")).collect::<String>()
    ));
    out.push_str(CODEC);
    out
}

/// `Ok`, `Err` and the decode error the codec raises.
const BUILTIN_MODELS: &str = r#"@dataclass
class Ok(Generic[T]):
    """The `ok` branch of a Corvid `Result`."""
    value: T


@dataclass
class Err(Generic[E]):
    """The `err` branch of a Corvid `Result`."""
    error: E


class WireError(ValueError):
    """JSON whose shape does not match the type it decodes into."""


"#;

/// The model methods every record and sum type gets.
fn model_methods(name: &str) -> String {
    format!(
        "\n    @classmethod\n    def from_json(cls, data: Any) -> \"{name}\":\n        return from_json(data, \"{name}\")\n\n    def to_json(self) -> Any:\n        return to_json(self, \"{name}\")\n\n\n"
    )
}

/// `JsonError`: why the server could not decode a body, or the error a
/// `T.from_json` route returns. Its fields come from the compiler.
fn json_error_fields() -> Vec<Field> {
    corvid_types::json_error_fields().iter().map(|(name, ty)| (name.to_string(), contract_type_name(ty))).collect()
}

fn json_error_model() -> String {
    let mut out = String::from("@dataclass\nclass JsonError:\n    \"\"\"Why JSON did not decode: where, what was expected, and what was found.\"\"\"\n");
    for (name, type_name) in json_error_fields() {
        out.push_str(&format!("    {name}: {}\n", annotation(&TypeName::parse(&type_name))));
    }
    out.push_str(&model_methods("JsonError"));
    out
}

/// The cursor-pagination envelope, from `Type::page_fields`. The item
/// type is the class's type parameter.
fn page_model() -> String {
    let mut out = String::from("@dataclass\nclass Page(Generic[T]):\n    \"\"\"One page of items and the cursor of the next page, if any.\"\"\"\n");
    let mut descriptors = Vec::new();
    for (name, ty) in Type::page_fields(&Type::Unknown) {
        let parsed = TypeName::parse(&contract_type_name(&ty));
        out.push_str(&format!("    {name}: {}\n", annotation(&parsed)));
        descriptors.push(format!("({name:?}, {})", descriptor(&parsed)));
    }
    out.push_str(&format!("\n\n_PAGE_FIELDS = [{}]\n\n\n", descriptors.join(", ")));
    out
}

/// The contract spelling of a compiler-known type. `Unknown` stands for
/// a page's item type, which the codec fills in per use.
fn contract_type_name(ty: &Type) -> String {
    match ty {
        Type::List(inner) => format!("List<{}>", contract_type_name(inner)),
        Type::Option(inner) => format!("Option<{}>", contract_type_name(inner)),
        Type::Unknown => "$item".into(),
        other => other.display_name(),
    }
}

fn record_entry(name: &str, fields: &[Field]) -> String {
    let fields = fields
        .iter()
        .map(|(field, type_name)| format!("({field:?}, {})", descriptor(&TypeName::parse(type_name))))
        .collect::<Vec<_>>()
        .join(", ");
    format!("    {name:?}: ({name}, [{fields}]),\n")
}

fn sum_entry(name: &str, variants: &[crate::app_contract::ContractVariant]) -> String {
    let mut out = format!("    {name:?}: {{\n");
    for variant in variants {
        let fields = variant
            .fields
            .iter()
            .map(|f| descriptor(&TypeName::parse(&f.type_name)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("        {:?}: ({name}{}, [{fields}]),\n", variant.name, variant.name));
    }
    out.push_str("    },\n");
    out
}

/// The codec's description of a type: a name, or a tuple of a head and
/// its arguments.
fn descriptor(ty: &TypeName) -> String {
    if ty.args.is_empty() {
        format!("{:?}", ty.head)
    } else {
        let args = ty.args.iter().map(descriptor).collect::<Vec<_>>().join(", ");
        format!("({:?}, {args})", ty.head)
    }
}

/// The Python annotation for a type.
fn annotation(ty: &TypeName) -> String {
    match (ty.head.as_str(), ty.args.len()) {
        ("Int", 0) => "int".into(),
        ("Float", 0) => "float".into(),
        ("String" | "TraceId", 0) => "str".into(),
        (name, 0) if corvid_public_json::text_scalar::is_name(name) => "str".into(),
        ("Bool", 0) => "bool".into(),
        ("Nothing", 0) => "None".into(),
        ("JsonValue", 0) => "Any".into(),
        ("$item", 0) => "T".into(),
        ("List", 1) => format!("list[{}]", annotation(ty.arg(0))),
        ("Option", 1) => format!("Optional[{}]", annotation(ty.arg(0))),
        ("Stream" | "Tainted" | "Grounded", 1) => annotation(ty.arg(0)),
        ("Page", 1) => format!("Page[{}]", annotation(ty.arg(0))),
        ("Upload", 1) => "bytes".into(),
        ("Map", 2) => format!("dict[{}, {}]", annotation(ty.arg(0)), annotation(ty.arg(1))),
        ("Result", 2) => format!("Union[Ok[{}], Err[{}]]", annotation(ty.arg(0)), annotation(ty.arg(1))),
        (name, _) => name.to_string(),
    }
}

/// The generic codec over `_RECORDS` and `_SUMS`.
const CODEC: &str = r#"def _parse_type(text: str) -> Any:
    """Parse a contract type name such as `List<Line>` into a descriptor."""
    def parse(at: int) -> tuple[Any, int]:
        end = at
        while end < len(text) and text[end] not in "<,>":
            end += 1
        head = text[at:end].strip()
        if end < len(text) and text[end] == "<":
            args = []
            at = end + 1
            while True:
                arg, at = parse(at)
                args.append(arg)
                if text[at] == ",":
                    at += 1
                    continue
                return (head, *args), at + 1
        return head, end
    return parse(0)[0]


def _kind(data: Any) -> str:
    if data is None:
        return "null"
    if isinstance(data, bool):
        return "a boolean"
    if isinstance(data, (int, float)):
        return "a number"
    if isinstance(data, str):
        return "a string"
    if isinstance(data, list):
        return "an array"
    return "an object"


def _fail(path: str, expected: str, data: Any) -> WireError:
    where = f"at `{path}`: " if path else ""
    return WireError(f"{where}expected {expected}, found {_kind(data)}")


def _key(path: str, key: str) -> str:
    return f"{path}.{key}" if path else key


def _optional(ty: Any) -> bool:
    while isinstance(ty, tuple) and ty[0] == "Tainted":
        ty = ty[1]
    return isinstance(ty, tuple) and ty[0] == "Option"


def _decode(ty: Any, data: Any, path: str) -> Any:
    if isinstance(ty, tuple):
        head, args = ty[0], ty[1:]
        if head in ("Tainted", "Stream"):
            return _decode(args[0], data, path)
        if head == "Grounded":
            if not isinstance(data, dict) or "value" not in data:
                raise _fail(path, "a grounded value", data)
            return _decode(args[0], data["value"], _key(path, "value"))
        if head == "Option":
            return None if data is None else _decode(args[0], data, path)
        if head == "List":
            if not isinstance(data, list):
                raise _fail(path, "an array", data)
            return [_decode(args[0], item, f"{path}[{i}]") for i, item in enumerate(data)]
        if head == "Map":
            if args[0] == "String":
                if not isinstance(data, dict):
                    raise _fail(path, "an object", data)
                return {k: _decode(args[1], v, _key(path, k)) for k, v in data.items()}
            if not isinstance(data, list):
                raise _fail(path, "an array of [key, value] pairs", data)
            out = {}
            for i, pair in enumerate(data):
                if not isinstance(pair, list) or len(pair) != 2:
                    raise _fail(f"{path}[{i}]", "a [key, value] pair", pair)
                out[_decode(args[0], pair[0], f"{path}[{i}][0]")] = _decode(args[1], pair[1], f"{path}[{i}][1]")
            return out
        if head == "Result":
            if not isinstance(data, dict) or (("ok" in data) == ("err" in data)):
                raise _fail(path, "an object with exactly one of `ok` or `err`", data)
            if "ok" in data:
                return Ok(_decode(args[0], data["ok"], _key(path, "ok")))
            return Err(_decode(args[1], data["err"], _key(path, "err")))
        if head == "Page":
            if not isinstance(data, dict):
                raise _fail(path, "a page", data)
            values = {}
            for name, field_ty in _PAGE_FIELDS:
                field_ty = _fill_item(field_ty, args[0])
                if name in data:
                    values[name] = _decode(field_ty, data[name], _key(path, name))
                elif _optional(field_ty):
                    values[name] = None
                else:
                    raise _fail(_key(path, name), "a value", None)
            return Page(**values)
        raise WireError(f"`{head}` has no JSON form")
    if ty == "Int":
        if isinstance(data, bool) or not isinstance(data, int):
            raise _fail(path, "Int", data)
        return data
    if ty == "Float":
        if isinstance(data, bool) or not isinstance(data, (int, float)):
            raise _fail(path, "Float", data)
        return float(data)
    if ty in _TEXT_SCALARS:
        if not isinstance(data, str):
            raise _fail(path, ty, data)
        return data
    if ty in ("String", "TraceId"):
        if not isinstance(data, str):
            raise _fail(path, "String", data)
        return data
    if ty == "Bool":
        if not isinstance(data, bool):
            raise _fail(path, "Bool", data)
        return data
    if ty == "Nothing":
        if data is not None:
            raise _fail(path, "null", data)
        return None
    if ty == "JsonValue":
        return data
    if ty in _RECORDS:
        cls, fields = _RECORDS[ty]
        if not isinstance(data, dict):
            raise _fail(path, ty, data)
        values = {}
        for name, field_ty in fields:
            if name in data:
                values[name] = _decode(field_ty, data[name], _key(path, name))
            elif _optional(field_ty):
                values[name] = None
            else:
                raise WireError(f"at `{_key(path, name)}`: expected a value, found no value (the field is missing)")
        return cls(**values)
    if ty in _SUMS:
        variants = _SUMS[ty]
        if not isinstance(data, dict):
            raise _fail(path, ty, data)
        name = data.get("variant")
        if name not in variants:
            raise WireError(f"at `{_key(path, 'variant')}`: expected one of {', '.join(variants)}, found {name!r}")
        cls, field_types = variants[name]
        payload = data.get("fields", [])
        if not isinstance(payload, list) or len(payload) != len(field_types):
            raise WireError(f"at `{_key(path, 'fields')}`: expected {len(field_types)} fields for `{name}`")
        return cls(*[_decode(t, v, f"{_key(path, 'fields')}[{i}]") for i, (t, v) in enumerate(zip(field_types, payload))])
    raise WireError(f"unknown type `{ty}`")


def _fill_item(ty: Any, item: Any) -> Any:
    if ty == "$item":
        return item
    if isinstance(ty, tuple):
        return (ty[0], *(_fill_item(arg, item) for arg in ty[1:]))
    return ty


def _encode(ty: Any, value: Any) -> Any:
    if isinstance(ty, tuple):
        head, args = ty[0], ty[1:]
        if head in ("Tainted", "Stream"):
            return _encode(args[0], value)
        if head == "Option":
            return None if value is None else _encode(args[0], value)
        if head == "List":
            return [_encode(args[0], item) for item in value]
        if head == "Map":
            if args[0] == "String":
                return {k: _encode(args[1], v) for k, v in value.items()}
            return [[_encode(args[0], k), _encode(args[1], v)] for k, v in value.items()]
        if head == "Result":
            if isinstance(value, Ok):
                return {"tag": "ok", "ok": _encode(args[0], value.value)}
            return {"tag": "err", "err": _encode(args[1], value.error)}
        if head == "Page":
            return {name: _encode(_fill_item(field_ty, args[0]), getattr(value, name)) for name, field_ty in _PAGE_FIELDS}
        raise WireError(f"`{head}` values are not sent as JSON")
    if ty == "Float":
        if not math.isfinite(value):
            raise WireError(f"the Float {value} has no JSON form")
        return float(value)
    if ty in ("Int", "String", "TraceId", "Bool", "Nothing", "JsonValue") or ty in _TEXT_SCALARS:
        return value
    if ty in _RECORDS:
        _, fields = _RECORDS[ty]
        return {name: _encode(field_ty, getattr(value, name)) for name, field_ty in fields}
    if ty in _SUMS:
        for name, (cls, field_types) in _SUMS[ty].items():
            if isinstance(value, cls):
                values = [getattr(value, f) for f in cls.__dataclass_fields__]
                return {"tag": "variant", "type": ty, "variant": name,
                        "fields": [_encode(t, v) for t, v in zip(field_types, values)]}
        raise WireError(f"{value!r} is not a `{ty}` variant")
    raise WireError(f"unknown type `{ty}`")


def from_json(data: Any, type_name: str) -> Any:
    """Decode parsed JSON into the model for `type_name`, such as `List<Line>`."""
    return _decode(_parse_type(type_name), data, "")


def to_json(value: Any, type_name: str) -> Any:
    """Encode a model of type `type_name` as JSON-ready Python values."""
    return _encode(_parse_type(type_name), value)
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk_gen::tests::{contract_for, SRC};

    #[test]
    fn python_models_map_records_and_sum_types() {
        let s = emit_python(&contract_for(SRC));
        assert!(s.contains("@dataclass\nclass Answer:"));
        assert!(s.contains("    text: str"));
        assert!(s.contains("    score: int"));
        assert!(s.contains("class RefundError:"));
        assert!(s.contains("class RefundErrorApprovalDenied(RefundError):"));
        assert!(s.contains("RefundError.ApprovalDenied = RefundErrorApprovalDenied"));
        assert!(s.contains("\"ApprovalDenied\": (RefundErrorApprovalDenied, [\"String\"]),"));
        assert!(s.contains("class JsonError:"));
        assert!(s.contains("    path: str"));
    }
}
