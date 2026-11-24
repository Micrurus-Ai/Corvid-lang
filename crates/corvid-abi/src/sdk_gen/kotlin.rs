//! Kotlin models from the Application Contract.
//!
//! Records become data classes and sum types become sealed interfaces.
//! Each model has a generated kotlinx.serialization `KSerializer` that
//! reads and writes through `JsonElement`, so the public JSON mapping is
//! followed exactly without the serialization compiler plugin: a sum
//! type is `{"tag": "variant", "type": ..., "variant": ..., "fields":
//! [...]}` with positional fields, a `Result` is a `CorvidResult`, and a
//! map whose keys are not `String` is an array of `[key, value]` pairs.
//! Corvid's `Int` is 64-bit, so it is a Kotlin `Long`.

use super::type_name::TypeName;
use super::NOTE_SCAFFOLD;
use crate::app_contract::ApplicationContract;
use corvid_types::Type;

pub(super) fn emit_kotlin(contract: &ApplicationContract) -> String {
    let mut out = format!("// {}\n\n", NOTE_SCAFFOLD.replace('\n', "\n// "));
    out.push_str(PRELUDE);
    out.push_str(&page_model());
    let json_error: Vec<(String, TypeName)> = corvid_types::json_error_fields()
        .iter()
        .map(|(name, ty)| (name.to_string(), TypeName::parse(&ty.display_name())))
        .collect();
    out.push_str(&record("JsonError", &json_error));
    for ty in &contract.types {
        if ty.variants.is_empty() {
            let fields: Vec<(String, TypeName)> =
                ty.fields.iter().map(|f| (f.name.clone(), TypeName::parse(&f.type_name))).collect();
            out.push_str(&record(&ty.name, &fields));
        } else {
            out.push_str(&sum(ty));
        }
    }
    out
}

fn record(name: &str, fields: &[(String, TypeName)]) -> String {
    let params = fields
        .iter()
        .map(|(field, ty)| format!("val {}: {}", ident(field), kotlin_type(ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let writes: String = fields
        .iter()
        .map(|(field, ty)| {
            format!(
                "        put({field:?}, json.encodeToJsonElement({}, value.{}))\n",
                serializer(ty),
                ident(field)
            )
        })
        .collect();
    let reads = fields
        .iter()
        .map(|(field, ty)| {
            format!(
                "            {} = json.decodeFromJsonElement({}, obj[{field:?}] ?: JsonNull)",
                ident(field),
                serializer(ty)
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "@Serializable(with = {name}Serializer::class)\ndata class {name}({params}) {{\n{methods}}}\n\n\
object {name}Serializer : CorvidJsonSerializer<{name}>({name:?}) {{\n    override fun write(json: Json, value: {name}): JsonElement = buildJsonObject {{\n{writes}    }}\n\n    override fun read(json: Json, element: JsonElement): {name} {{\n        val obj = element.corvidObject({name:?})\n        return {name}(\n{reads}\n        )\n    }}\n}}\n\n",
        methods = model_methods(name),
    )
}

fn sum(ty: &crate::app_contract::ContractType) -> String {
    let name = &ty.name;
    let mut out = format!("@Serializable(with = {name}Serializer::class)\nsealed interface {name} {{\n");
    for variant in &ty.variants {
        if variant.fields.is_empty() {
            out.push_str(&format!("    data object {} : {name}\n", variant.name));
        } else {
            let params = variant
                .fields
                .iter()
                .map(|f| format!("val {}: {}", ident(&f.name), kotlin_type(&TypeName::parse(&f.type_name))))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("    data class {}({params}) : {name}\n", variant.name));
        }
    }
    out.push('\n');
    out.push_str(&model_methods(name));
    out.push_str("}\n\n");

    let names = ty.variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", ");
    let mut writes = String::new();
    let mut reads = String::new();
    for variant in &ty.variants {
        let encoded = variant
            .fields
            .iter()
            .map(|f| format!("json.encodeToJsonElement({}, value.{})", serializer(&TypeName::parse(&f.type_name)), ident(&f.name)))
            .collect::<Vec<_>>()
            .join(", ");
        writes.push_str(&format!(
            "        is {name}.{v} -> corvidVariant({name:?}, {v:?}, listOf({encoded}))\n",
            v = variant.name
        ));
        let decoded = variant
            .fields
            .iter()
            .enumerate()
            .map(|(index, f)| format!("json.decodeFromJsonElement({}, fields[{index}])", serializer(&TypeName::parse(&f.type_name))))
            .collect::<Vec<_>>()
            .join(", ");
        let value = if variant.fields.is_empty() {
            format!("{name}.{}", variant.name)
        } else {
            format!("{name}.{}({decoded})", variant.name)
        };
        reads.push_str(&format!(
            "            {v:?} -> {{\n                corvidArity({v:?}, fields, {count})\n                {value}\n            }}\n",
            v = variant.name,
            count = variant.fields.len(),
        ));
    }
    out.push_str(&format!(
        "object {name}Serializer : CorvidJsonSerializer<{name}>({name:?}) {{\n    override fun write(json: Json, value: {name}): JsonElement = when (value) {{\n{writes}    }}\n\n    override fun read(json: Json, element: JsonElement): {name} {{\n        val obj = element.corvidObject({name:?})\n        val fields = obj[\"fields\"]?.let {{ it as? JsonArray ?: throw SerializationException(\"at `fields`: expected an array\") }} ?: JsonArray(emptyList())\n        return when (val variant = (obj[\"variant\"] as? JsonPrimitive)?.contentOrNull) {{\n{reads}            else -> throw SerializationException(\"at `variant`: expected one of {names}, found $variant\")\n        }}\n    }}\n}}\n\n"
    ));
    out
}

fn model_methods(name: &str) -> String {
    format!(
        "    fun toJson(): String = CorvidJson.encodeToString({name}Serializer, this)\n\n    companion object {{\n        fun fromJson(text: String): {name} = CorvidJson.decodeFromString({name}Serializer, text)\n    }}\n"
    )
}

/// The cursor-pagination envelope, with field names from
/// `Type::page_fields`.
fn page_model() -> String {
    let fields = Type::page_fields(&Type::Unknown);
    let [(items, _), (cursor, _), (more, _)] = &fields;
    format!(
        "/** One page of items and the cursor of the next page, if any. */\ndata class CorvidPage<T>(val {items}: List<T>, val {cursor}: String?, val {more}: Boolean)\n\n\
class CorvidPageSerializer<T>(private val item: KSerializer<T>) : CorvidJsonSerializer<CorvidPage<T>>(\"Page\") {{\n    override fun write(json: Json, value: CorvidPage<T>): JsonElement = buildJsonObject {{\n        put({items:?}, json.encodeToJsonElement(ListSerializer(item), value.{items}))\n        put({cursor:?}, json.encodeToJsonElement(String.serializer().nullable, value.{cursor}))\n        put({more:?}, JsonPrimitive(value.{more}))\n    }}\n\n    override fun read(json: Json, element: JsonElement): CorvidPage<T> {{\n        val obj = element.corvidObject(\"Page\")\n        return CorvidPage(\n            json.decodeFromJsonElement(ListSerializer(item), obj[{items:?}] ?: JsonNull),\n            json.decodeFromJsonElement(String.serializer().nullable, obj[{cursor:?}] ?: JsonNull),\n            json.decodeFromJsonElement(Boolean.serializer(), obj[{more:?}] ?: JsonNull),\n        )\n    }}\n}}\n\n"
    )
}

/// The Kotlin type for a contract type.
fn kotlin_type(ty: &TypeName) -> String {
    match (ty.head.as_str(), ty.args.len()) {
        ("Int", 0) => "Long".into(),
        ("Float", 0) => "Double".into(),
        ("String" | "TraceId", 0) => "String".into(),
        ("Bool", 0) => "Boolean".into(),
        ("Nothing", 0) => "Unit".into(),
        ("JsonValue", 0) => "JsonElement".into(),
        ("List", 1) => format!("List<{}>", kotlin_type(ty.arg(0))),
        ("Option", 1) => format!("{}?", kotlin_type(ty.arg(0))),
        ("Stream" | "Tainted" | "Grounded", 1) => kotlin_type(ty.arg(0)),
        ("Page", 1) => format!("CorvidPage<{}>", kotlin_type(ty.arg(0))),
        ("Upload", 1) => "ByteArray".into(),
        ("Map", 2) => format!("Map<{}, {}>", kotlin_type(ty.arg(0)), kotlin_type(ty.arg(1))),
        ("Result", 2) => format!("CorvidResult<{}, {}>", kotlin_type(ty.arg(0)), kotlin_type(ty.arg(1))),
        (name, _) => name.to_string(),
    }
}

/// The serializer expression for a contract type.
fn serializer(ty: &TypeName) -> String {
    match (ty.head.as_str(), ty.args.len()) {
        ("Int", 0) => "Long.serializer()".into(),
        ("Float", 0) => "Double.serializer()".into(),
        ("String" | "TraceId", 0) => "String.serializer()".into(),
        ("Bool", 0) => "Boolean.serializer()".into(),
        ("Nothing", 0) => "CorvidNothingSerializer".into(),
        ("JsonValue", 0) => "JsonElement.serializer()".into(),
        ("List", 1) => format!("ListSerializer({})", serializer(ty.arg(0))),
        ("Option", 1) => format!("{}.nullable", serializer(ty.arg(0))),
        ("Stream" | "Tainted", 1) => serializer(ty.arg(0)),
        ("Grounded", 1) => format!("CorvidGroundedSerializer({})", serializer(ty.arg(0))),
        ("Page", 1) => format!("CorvidPageSerializer({})", serializer(ty.arg(0))),
        ("Upload", 1) => "CorvidUploadSerializer".into(),
        ("Map", 2) if ty.arg(0).head == "String" => {
            format!("MapSerializer(String.serializer(), {})", serializer(ty.arg(1)))
        }
        ("Map", 2) => format!("CorvidPairsSerializer({}, {})", serializer(ty.arg(0)), serializer(ty.arg(1))),
        ("Result", 2) => format!("CorvidResultSerializer({}, {})", serializer(ty.arg(0)), serializer(ty.arg(1))),
        (name, _) => format!("{name}Serializer"),
    }
}

/// A Kotlin identifier for a field name, quoted when it is a keyword.
fn ident(name: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "as", "break", "class", "continue", "do", "else", "false", "for", "fun", "if", "in", "interface", "is",
        "null", "object", "package", "return", "super", "this", "throw", "true", "try", "typealias", "typeof",
        "val", "var", "when", "while",
    ];
    if KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_string()
    }
}

/// The runtime support every generated file carries.
const PRELUDE: &str = r#"@file:Suppress("unused", "RedundantVisibilityModifier")

import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.nullable
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.descriptors.buildClassSerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.*

/** The `Json` instance the models use. */
val CorvidJson: Json = Json

/** A Corvid `Result`: `{"tag": "ok", "ok": ...}` or `{"tag": "err", "err": ...}`. */
sealed interface CorvidResult<out T, out E> {
    data class Ok<out T>(val value: T) : CorvidResult<T, Nothing>
    data class Err<out E>(val error: E) : CorvidResult<Nothing, E>
}

/** A serializer that reads and writes Corvid's public JSON through `JsonElement`. */
abstract class CorvidJsonSerializer<T>(name: String) : KSerializer<T> {
    override val descriptor: SerialDescriptor = buildClassSerialDescriptor(name)

    abstract fun write(json: Json, value: T): JsonElement

    abstract fun read(json: Json, element: JsonElement): T

    final override fun serialize(encoder: Encoder, value: T) {
        val out = encoder as? JsonEncoder ?: throw SerializationException("Corvid models encode only to JSON")
        out.encodeJsonElement(write(out.json, value))
    }

    final override fun deserialize(decoder: Decoder): T {
        val input = decoder as? JsonDecoder ?: throw SerializationException("Corvid models decode only from JSON")
        return read(input.json, input.decodeJsonElement())
    }
}

internal fun JsonElement.corvidObject(type: String): JsonObject =
    this as? JsonObject ?: throw SerializationException("expected $type, found ${corvidKind(this)}")

internal fun corvidKind(element: JsonElement): String = when {
    element is JsonNull -> "null"
    element is JsonArray -> "an array"
    element is JsonObject -> "an object"
    element is JsonPrimitive && element.isString -> "a string"
    element is JsonPrimitive && element.booleanOrNull != null -> "a boolean"
    else -> "a number"
}

internal fun corvidVariant(type: String, variant: String, fields: List<JsonElement>): JsonElement = buildJsonObject {
    put("tag", "variant")
    put("type", type)
    put("variant", variant)
    put("fields", JsonArray(fields))
}

internal fun corvidArity(variant: String, fields: JsonArray, count: Int) {
    if (fields.size != count) throw SerializationException("at `fields`: expected $count fields for `$variant`, found ${fields.size} fields")
}

class CorvidResultSerializer<T, E>(private val ok: KSerializer<T>, private val err: KSerializer<E>) :
    CorvidJsonSerializer<CorvidResult<T, E>>("Result") {
    override fun write(json: Json, value: CorvidResult<T, E>): JsonElement = when (value) {
        is CorvidResult.Ok -> buildJsonObject {
            put("tag", "ok")
            put("ok", json.encodeToJsonElement(ok, value.value))
        }
        is CorvidResult.Err -> buildJsonObject {
            put("tag", "err")
            put("err", json.encodeToJsonElement(err, value.error))
        }
    }

    override fun read(json: Json, element: JsonElement): CorvidResult<T, E> {
        val obj = element.corvidObject("Result")
        if (("ok" in obj) == ("err" in obj)) throw SerializationException("expected an object with exactly one of `ok` or `err`")
        return if ("ok" in obj) CorvidResult.Ok(json.decodeFromJsonElement(ok, obj.getValue("ok")))
        else CorvidResult.Err(json.decodeFromJsonElement(err, obj.getValue("err")))
    }
}

/** A map whose keys are not `String` travels as an array of `[key, value]` pairs. */
class CorvidPairsSerializer<K, V>(private val key: KSerializer<K>, private val item: KSerializer<V>) :
    CorvidJsonSerializer<Map<K, V>>("Map") {
    override fun write(json: Json, value: Map<K, V>): JsonElement =
        JsonArray(value.map { (k, v) -> JsonArray(listOf(json.encodeToJsonElement(key, k), json.encodeToJsonElement(item, v))) })

    override fun read(json: Json, element: JsonElement): Map<K, V> {
        val pairs = element as? JsonArray ?: throw SerializationException("expected an array of [key, value] pairs, found ${corvidKind(element)}")
        return pairs.withIndex().associate { (index, pair) ->
            val kv = pair as? JsonArray
            if (kv == null || kv.size != 2) throw SerializationException("at `[$index]`: expected a [key, value] pair")
            json.decodeFromJsonElement(key, kv[0]) to json.decodeFromJsonElement(item, kv[1])
        }
    }
}

/** A `Grounded` value arrives as `{"value": ..., "sources": [...]}`; only the value is kept. */
class CorvidGroundedSerializer<T>(private val inner: KSerializer<T>) : CorvidJsonSerializer<T>("Grounded") {
    override fun write(json: Json, value: T): JsonElement =
        throw SerializationException("grounded values come from the server; send the value itself")

    override fun read(json: Json, element: JsonElement): T =
        json.decodeFromJsonElement(inner, element.corvidObject("Grounded")["value"] ?: JsonNull)
}

/** `Nothing` is JSON `null`. */
object CorvidNothingSerializer : CorvidJsonSerializer<Unit>("Nothing") {
    override fun write(json: Json, value: Unit): JsonElement = JsonNull

    override fun read(json: Json, element: JsonElement) {
        if (element !is JsonNull) throw SerializationException("expected null, found ${corvidKind(element)}")
    }
}

/** Uploads are sent as multipart form data, never as JSON. */
object CorvidUploadSerializer : CorvidJsonSerializer<ByteArray>("Upload") {
    override fun write(json: Json, value: ByteArray): JsonElement =
        throw SerializationException("uploads are sent as multipart form data, not JSON")

    override fun read(json: Json, element: JsonElement): ByteArray =
        throw SerializationException("uploads are sent as multipart form data, not JSON")
}

"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdk_gen::tests::{contract_for, SRC};

    #[test]
    fn kotlin_models_map_records_and_sealed_interfaces() {
        let s = emit_kotlin(&contract_for(SRC));
        assert!(s.contains("data class Answer(val text: String, val score: Long)"));
        assert!(s.contains("sealed interface RefundError {"));
        assert!(s.contains("    data object PaymentNotFound : RefundError"));
        assert!(s.contains("    data class ApprovalDenied(val reason: String) : RefundError"));
        assert!(s.contains("is RefundError.ApprovalDenied -> corvidVariant(\"RefundError\", \"ApprovalDenied\""));
        assert!(s.contains("data class JsonError(val path: String, val expected: String, val found: String, val message: String)"));
    }
}
