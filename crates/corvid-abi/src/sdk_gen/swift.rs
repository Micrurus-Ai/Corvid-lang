//! Swift models from the Application Contract.
//!
//! Records become structs and sum types become enums with associated
//! values. Every model writes its own `Codable` methods, because the
//! synthesized ones do not follow the public JSON mapping: a sum type is
//! `{"tag": "variant", "type": ..., "variant": ..., "fields": [...]}`
//! with positional fields, a `None` field is written as `null` rather
//! than omitted, a `Result` is a `CorvidResult`, and a map whose keys
//! are not `String` is a `CorvidPairs` travelling as `[key, value]`
//! pairs.

use super::type_name::TypeName;
use super::{lower_first, NOTE_SCAFFOLD};
use crate::app_contract::{ApplicationContract, ContractType};
use corvid_types::Type;

pub(super) fn emit_swift(contract: &ApplicationContract) -> String {
    let mut out = format!("// {}\n\n", NOTE_SCAFFOLD.replace('\n', "\n// "));
    out.push_str(PRELUDE);
    out.push_str(&page_model());
    out.push_str(&conflict_model());
    let json_error: Vec<(String, String, TypeName)> = corvid_types::json_error_fields()
        .iter()
        .map(|(name, ty)| (name.to_string(), name.to_string(), TypeName::parse(&ty.display_name())))
        .collect();
    out.push_str(&record("JsonError", &json_error));
    for ty in &contract.types {
        if ty.variants.is_empty() {
            let fields: Vec<(String, String, TypeName)> = ty
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.key().to_string(), TypeName::parse(&f.type_name)))
                .collect();
            out.push_str(&record(&ty.name, &fields));
        } else {
            out.push_str(&sum(ty));
        }
    }
    out.push_str(&format!("// {} public agent(s), {} route(s).\n", contract.agents.len(), contract.routes.len()));
    out
}

/// A record's fields are `(property, wire key, type)`; the key is the
/// field's JSON key (`@json(key:)`, B01), which `CodingKeys` maps.
fn record(name: &str, fields: &[(String, String, TypeName)]) -> String {
    let mut out = format!("public struct {name}: Codable {{\n");
    for (field, _, ty) in fields {
        out.push_str(&format!("    public var {}: {}\n", ident(field), swift_type(ty)));
    }
    let params = fields.iter().map(|(f, _, ty)| format!("{}: {}", ident(f), swift_type(ty))).collect::<Vec<_>>().join(", ");
    out.push_str(&format!("\n    public init({params}) {{\n"));
    for (field, _, _) in fields {
        out.push_str(&format!("        self.{0} = {0}\n", ident(field)));
    }
    out.push_str("    }\n\n");
    if fields.is_empty() {
        out.push_str("    public init(from decoder: Decoder) throws {}\n\n    public func encode(to encoder: Encoder) throws {\n        _ = encoder.container(keyedBy: CorvidNoKeys.self)\n    }\n}\n\n");
        return out;
    }
    out.push_str("    private enum CodingKeys: String, CodingKey {\n");
    for (field, key, _) in fields {
        if key == field {
            out.push_str(&format!("        case {}\n", ident(field)));
        } else {
            out.push_str(&format!("        case {} = {key:?}\n", ident(field)));
        }
    }
    out.push_str("    }\n\n    public init(from decoder: Decoder) throws {\n        let c = try decoder.container(keyedBy: CodingKeys.self)\n");
    for (field, _, ty) in fields {
        let key = ident(field);
        let read = match optional_payload(ty) {
            Some(inner) => format!("try c.decodeIfPresent({}.self, forKey: .{key})", swift_type(inner)),
            None => format!("try c.decode({}.self, forKey: .{key})", swift_type(ty)),
        };
        out.push_str(&format!("        self.{key} = {read}\n"));
    }
    out.push_str("    }\n\n    public func encode(to encoder: Encoder) throws {\n        var c = encoder.container(keyedBy: CodingKeys.self)\n");
    for (field, _, _) in fields {
        // An optional encodes as `null`, as the public mapping writes it.
        out.push_str(&format!("        try c.encode(self.{0}, forKey: .{0})\n", ident(field)));
    }
    out.push_str("    }\n}\n\n");
    out
}

fn sum(ty: &ContractType) -> String {
    let name = &ty.name;
    let mut out = format!("public enum {name}: Codable {{\n");
    for variant in &ty.variants {
        let case = ident(&lower_first(&variant.name));
        if variant.fields.is_empty() {
            out.push_str(&format!("    case {case}\n"));
        } else {
            let params = variant
                .fields
                .iter()
                .map(|f| format!("{}: {}", ident(&f.name), swift_type(&TypeName::parse(&f.type_name))))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("    case {case}({params})\n"));
        }
    }
    let names = ty.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ");
    out.push_str("\n    public init(from decoder: Decoder) throws {\n        let c = try decoder.container(keyedBy: CorvidVariantKeys.self)\n        let variant = try c.decode(String.self, forKey: .variant)\n        switch variant {\n");
    for variant in &ty.variants {
        let case = ident(&lower_first(&variant.name));
        out.push_str(&format!("        case {:?}:\n", variant.name));
        if variant.fields.is_empty() {
            out.push_str(&format!("            self = .{case}\n"));
            continue;
        }
        out.push_str(&format!(
            "            var fields = try c.nestedUnkeyedContainer(forKey: .fields)\n            try corvidArity(fields, {}, {:?}, c)\n",
            variant.fields.len(),
            variant.name
        ));
        let args = variant
            .fields
            .iter()
            .map(|f| format!("{}: try fields.decode({}.self)", ident(&f.name), swift_type(&TypeName::parse(&f.type_name))))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("            self = .{case}({args})\n"));
    }
    out.push_str(&format!(
        "        default:\n            throw DecodingError.dataCorruptedError(forKey: .variant, in: c, debugDescription: \"expected one of {names}, found \\(variant)\")\n        }}\n    }}\n\n"
    ));
    out.push_str(&format!(
        "    public func encode(to encoder: Encoder) throws {{\n        var c = encoder.container(keyedBy: CorvidVariantKeys.self)\n        try c.encode(\"variant\", forKey: .tag)\n        try c.encode({name:?}, forKey: .type)\n        switch self {{\n"
    ));
    for variant in &ty.variants {
        let case = ident(&lower_first(&variant.name));
        if variant.fields.is_empty() {
            out.push_str(&format!(
                "        case .{case}:\n            try c.encode({:?}, forKey: .variant)\n            _ = c.nestedUnkeyedContainer(forKey: .fields)\n",
                variant.name
            ));
            continue;
        }
        let bindings = variant.fields.iter().map(|f| ident(&f.name)).collect::<Vec<_>>().join(", ");
        out.push_str(&format!(
            "        case let .{case}({bindings}):\n            try c.encode({:?}, forKey: .variant)\n            var fields = c.nestedUnkeyedContainer(forKey: .fields)\n",
            variant.name
        ));
        for field in &variant.fields {
            out.push_str(&format!("            try fields.encode({})\n", ident(&field.name)));
        }
    }
    out.push_str("        }\n    }\n}\n\n");
    out
}

/// The cursor-pagination envelope, with field names from
/// `Type::page_fields`.
fn page_model() -> String {
    let fields = Type::page_fields(&Type::Unknown);
    let [(items, _), (cursor, _), (more, _)] = &fields;
    format!(
        "/// One page of items and the cursor of the next page, if any.\npublic struct CorvidPage<T: Codable>: Codable {{\n    public var {items}: [T]\n    public var {cursor}: String?\n    public var {more}: Bool\n\n    private enum CodingKeys: String, CodingKey {{\n        case {items}, {cursor}, {more}\n    }}\n\n    public init(from decoder: Decoder) throws {{\n        let c = try decoder.container(keyedBy: CodingKeys.self)\n        {items} = try c.decode([T].self, forKey: .{items})\n        {cursor} = try c.decodeIfPresent(String.self, forKey: .{cursor})\n        {more} = try c.decode(Bool.self, forKey: .{more})\n    }}\n\n    public func encode(to encoder: Encoder) throws {{\n        var c = encoder.container(keyedBy: CodingKeys.self)\n        try c.encode({items}, forKey: .{items})\n        try c.encode({cursor}, forKey: .{cursor})\n        try c.encode({more}, forKey: .{more})\n    }}\n}}\n\n"
    )
}

/// A table write's conflict (B09): a generic enum with one case per
/// `corvid_types::ConflictCase`, travelling as the sum type it is.
fn conflict_model() -> String {
    let cases = corvid_types::ConflictCase::ALL;
    let mut out = String::from("/// Why a table write did not happen.\npublic enum CorvidConflict<Row: Codable>: Codable {\n");
    for case in cases {
        let name = lower_first(case.name());
        match case.field() {
            Some(field) => out.push_str(&format!("    case {name}({field}: Row)\n")),
            None => out.push_str(&format!("    case {name}\n")),
        }
    }
    let names = cases.iter().map(|case| case.name()).collect::<Vec<_>>().join(", ");
    out.push_str("\n    public init(from decoder: Decoder) throws {\n        let c = try decoder.container(keyedBy: CorvidVariantKeys.self)\n        let variant = try c.decode(String.self, forKey: .variant)\n        switch variant {\n");
    for case in cases {
        let name = lower_first(case.name());
        out.push_str(&format!("        case {:?}:\n", case.name()));
        match case.field() {
            Some(field) => out.push_str(&format!(
                "            var fields = try c.nestedUnkeyedContainer(forKey: .fields)\n            try corvidArity(fields, 1, {:?}, c)\n            self = .{name}({field}: try fields.decode(Row.self))\n",
                case.name()
            )),
            None => out.push_str(&format!("            self = .{name}\n")),
        }
    }
    out.push_str(&format!(
        "        default:\n            throw DecodingError.dataCorruptedError(forKey: .variant, in: c, debugDescription: \"expected one of {names}, found \\(variant)\")\n        }}\n    }}\n\n"
    ));
    out.push_str("    public func encode(to encoder: Encoder) throws {\n        var c = encoder.container(keyedBy: CorvidVariantKeys.self)\n        try c.encode(\"variant\", forKey: .tag)\n        try c.encode(\"Conflict\", forKey: .type)\n        switch self {\n");
    for case in cases {
        let name = lower_first(case.name());
        match case.field() {
            Some(field) => out.push_str(&format!(
                "        case let .{name}({field}):\n            try c.encode({:?}, forKey: .variant)\n            var fields = c.nestedUnkeyedContainer(forKey: .fields)\n            try fields.encode({field})\n",
                case.name()
            )),
            None => out.push_str(&format!(
                "        case .{name}:\n            try c.encode({:?}, forKey: .variant)\n            _ = c.nestedUnkeyedContainer(forKey: .fields)\n",
                case.name()
            )),
        }
    }
    out.push_str("        }\n    }\n}\n\n");
    out
}

/// The payload of an `Option` field, which decodes when present.
fn optional_payload(ty: &TypeName) -> Option<&TypeName> {
    match (ty.head.as_str(), ty.args.len()) {
        ("Option", 1) => Some(ty.arg(0)),
        ("Tainted", 1) => optional_payload(ty.arg(0)),
        _ => None,
    }
}

/// The Swift type for a contract type.
fn swift_type(ty: &TypeName) -> String {
    match (ty.head.as_str(), ty.args.len()) {
        ("Int", 0) => "Int".into(),
        ("Float", 0) => "Double".into(),
        ("String" | "TraceId", 0) => "String".into(),
        // B06 — exact canonical text, the one mapping every SDK can hold
        // without losing digits, scale or nanoseconds.
        (name, 0) if corvid_public_json::text_scalar::is_name(name) => "String".into(),
        ("Bool", 0) => "Bool".into(),
        ("Nothing", 0) => "CorvidNothing".into(),
        ("JsonValue", 0) => "CorvidJSONValue".into(),
        ("List", 1) => format!("[{}]", swift_type(ty.arg(0))),
        ("Option", 1) => format!("{}?", swift_type(ty.arg(0))),
        ("Stream" | "Tainted", 1) => swift_type(ty.arg(0)),
        ("Grounded", 1) => format!("CorvidGrounded<{}>", swift_type(ty.arg(0))),
        ("Page", 1) => format!("CorvidPage<{}>", swift_type(ty.arg(0))),
        ("Conflict", 1) => format!("CorvidConflict<{}>", swift_type(ty.arg(0))),
        ("Upload", 1) => "Data".into(),
        ("Map", 2) if ty.arg(0).head == "String" => format!("[String: {}]", swift_type(ty.arg(1))),
        ("Map", 2) => format!("CorvidPairs<{}, {}>", swift_type(ty.arg(0)), swift_type(ty.arg(1))),
        ("Result", 2) => format!("CorvidResult<{}, {}>", swift_type(ty.arg(0)), swift_type(ty.arg(1))),
        (name, _) => name.to_string(),
    }
}

/// A Swift identifier, quoted when it is a keyword.
fn ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "associatedtype", "break", "case", "catch", "class", "continue", "default", "defer", "deinit",
        "do", "else", "enum", "extension", "fallthrough", "false", "fileprivate", "for", "func", "guard", "if",
        "import", "in", "init", "inout", "internal", "is", "let", "nil", "operator", "private", "protocol",
        "public", "repeat", "rethrows", "return", "self", "static", "struct", "subscript", "super", "switch",
        "throw", "throws", "true", "try", "typealias", "var", "where", "while",
    ];
    if KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

/// The runtime support every generated file carries.
const PRELUDE: &str = r#"import Foundation

enum CorvidVariantKeys: String, CodingKey {
    case tag, type, variant, fields
}

enum CorvidNoKeys: CodingKey {}

func corvidArity(_ fields: UnkeyedDecodingContainer, _ count: Int, _ variant: String, _ c: KeyedDecodingContainer<CorvidVariantKeys>) throws {
    if let found = fields.count, found != count {
        throw DecodingError.dataCorruptedError(forKey: .fields, in: c, debugDescription: "expected \(count) fields for `\(variant)`, found \(found) fields")
    }
}

/// A Corvid `Result`: `{"tag": "ok", "ok": ...}` or `{"tag": "err", "err": ...}`.
public enum CorvidResult<T: Codable, E: Codable>: Codable {
    case ok(T)
    case err(E)

    private enum Keys: String, CodingKey {
        case tag, ok, err
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        guard c.contains(.ok) != c.contains(.err) else {
            throw DecodingError.dataCorrupted(.init(codingPath: c.codingPath, debugDescription: "expected an object with exactly one of `ok` or `err`"))
        }
        self = c.contains(.ok) ? .ok(try c.decode(T.self, forKey: .ok)) : .err(try c.decode(E.self, forKey: .err))
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .ok(let value):
            try c.encode("ok", forKey: .tag)
            try c.encode(value, forKey: .ok)
        case .err(let error):
            try c.encode("err", forKey: .tag)
            try c.encode(error, forKey: .err)
        }
    }
}

/// A map whose keys are not `String`, travelling as `[key, value]` pairs.
public struct CorvidPairs<K: Codable & Hashable, V: Codable>: Codable {
    public var value: [K: V]

    public init(_ value: [K: V]) {
        self.value = value
    }

    public init(from decoder: Decoder) throws {
        var c = try decoder.unkeyedContainer()
        var out: [K: V] = [:]
        while !c.isAtEnd {
            var pair = try c.nestedUnkeyedContainer()
            guard pair.count == 2 else {
                throw DecodingError.dataCorrupted(.init(codingPath: pair.codingPath, debugDescription: "expected a [key, value] pair"))
            }
            let key = try pair.decode(K.self)
            out[key] = try pair.decode(V.self)
        }
        value = out
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.unkeyedContainer()
        for (key, item) in value {
            var pair = c.nestedUnkeyedContainer()
            try pair.encode(key)
            try pair.encode(item)
        }
    }
}

/// A `Grounded` value arrives as `{"value": ..., "sources": [...]}`; the value is kept.
public struct CorvidGrounded<T: Codable>: Codable {
    public var value: T

    private enum Keys: String, CodingKey {
        case value
    }

    public init(from decoder: Decoder) throws {
        value = try decoder.container(keyedBy: Keys.self).decode(T.self, forKey: .value)
    }

    public func encode(to encoder: Encoder) throws {
        throw EncodingError.invalidValue(value, .init(codingPath: encoder.codingPath, debugDescription: "grounded values come from the server; send the value itself"))
    }
}

/// `Nothing` is JSON `null`.
public struct CorvidNothing: Codable {
    public init() {}

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        guard c.decodeNil() else {
            throw DecodingError.dataCorrupted(.init(codingPath: c.codingPath, debugDescription: "expected null"))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encodeNil()
    }
}

/// A `JsonValue`: the JSON itself.
public indirect enum CorvidJSONValue: Codable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([CorvidJSONValue])
    case object([String: CorvidJSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([CorvidJSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: CorvidJSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}

"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk_gen::tests::{contract_for, SRC};

    #[test]
    fn swift_models_map_records_and_enums_with_wire_codable() {
        let s = emit_swift(&contract_for(SRC));
        assert!(s.contains("public struct Answer: Codable {"));
        assert!(s.contains("public var text: String"));
        assert!(s.contains("public var score: Int"));
        assert!(s.contains("public enum RefundError: Codable {"));
        assert!(s.contains("case paymentNotFound"));
        assert!(s.contains("case approvalDenied(reason: String)"));
        assert!(s.contains("try c.encode(\"RefundError\", forKey: .type)"));
        assert!(s.contains("public struct JsonError: Codable {"));
    }
}
