use super::*;
use corvid_ast::{Refinement, Span};
use corvid_ir::{IrEnumVariant, IrField};
use serde_json::json;

fn field(name: &str, ty: Type) -> IrField {
    IrField { name: name.into(), ty, refinement: None, span: Span::new(0, 0) }
}

fn record(id: u32, name: &str, fields: Vec<IrField>) -> IrType {
    IrType { id: DefId(id), name: name.into(), fields, variants: Vec::new(), span: Span::new(0, 0) }
}

/// `Note { title: String, tag: Option<String> }`, `Order { id: Int, items: List<Line> }`,
/// `Line { sku: String, quantity: Int where between(1, 99) }` and
/// `Outcome = Saved(Int) | Changed(Int, String)`.
struct Fixture {
    note: IrType,
    order: IrType,
    line: IrType,
    outcome: IrType,
}

impl Fixture {
    fn new() -> Self {
        let mut quantity = field("quantity", Type::Int);
        quantity.refinement = Some(Refinement::Between { min: 1, max: 99 });
        let outcome = IrType {
            id: DefId(4),
            name: "Outcome".into(),
            fields: Vec::new(),
            variants: vec![
                IrEnumVariant { name: "Saved".into(), fields: vec![field("shelf", Type::Int)], span: Span::new(0, 0) },
                IrEnumVariant {
                    name: "Changed".into(),
                    fields: vec![field("shelf", Type::Int), field("title", Type::String)],
                    span: Span::new(0, 0),
                },
            ],
            span: Span::new(0, 0),
        };
        Self {
            note: record(1, "Note", vec![field("title", Type::String), field("tag", Type::Option(Box::new(Type::String)))]),
            order: record(2, "Order", vec![field("id", Type::Int), field("items", Type::List(Box::new(Type::Struct(DefId(3)))))]),
            line: record(3, "Line", vec![field("sku", Type::String), quantity]),
            outcome,
        }
    }

    fn types(&self) -> HashMap<DefId, &IrType> {
        [&self.note, &self.order, &self.line, &self.outcome].into_iter().map(|t| (t.id, t)).collect()
    }
}

fn decode(json: J, ty: &Type, unknown: UnknownFields) -> Result<Value, JsonDecodeError> {
    let fixture = Fixture::new();
    decode_public_json(&json, ty, &fixture.types(), unknown)
}

fn round_trip(json: J, ty: &Type) -> J {
    let value = decode(json, ty, UnknownFields::Reject).expect("decodes");
    encode_public_json(&value).expect("encodes")
}

const NOTE: Type = Type::Struct(DefId(1));
const ORDER: Type = Type::Struct(DefId(2));
const OUTCOME: Type = Type::Struct(DefId(4));

#[test]
fn option_field_accepts_a_value_null_or_absence() {
    assert_eq!(round_trip(json!({"title": "a", "tag": "x"}), &NOTE), json!({"title": "a", "tag": "x"}));
    assert_eq!(round_trip(json!({"title": "a", "tag": null}), &NOTE), json!({"title": "a", "tag": null}));
    assert_eq!(round_trip(json!({"title": "a"}), &NOTE), json!({"title": "a", "tag": null}));
}

#[test]
fn option_field_rejects_the_internal_envelope() {
    let error = decode(json!({"title": "a", "tag": {"tag": "some", "value": "x"}}), &NOTE, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error, JsonDecodeError::new("tag", "String", "an object"));
}

#[test]
fn missing_required_field_names_its_path_and_type() {
    let error = decode(json!({"tag": "x"}), &NOTE, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error.message(), "at `title`: expected String, found no value (the field is missing)");
}

#[test]
fn unknown_fields_follow_the_requested_policy() {
    let body = json!({"title": "a", "extra": 1});
    assert!(decode(body.clone(), &NOTE, UnknownFields::Ignore).is_ok());
    let error = decode(body, &NOTE, UnknownFields::Reject).unwrap_err();
    assert_eq!(error.path, "extra");
    assert!(error.expected.contains("`Note` declares no `extra`"), "{error:?}");
}

#[test]
fn nested_errors_report_the_full_path() {
    let body = json!({"id": 7, "items": [{"sku": "a", "quantity": 1}, {"sku": "b", "quantity": "2"}]});
    let error = decode(body, &ORDER, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error.message(), "at `items[1].quantity`: expected Int, found a string");
}

#[test]
fn refinements_are_enforced_at_their_path() {
    let body = json!({"id": 7, "items": [{"sku": "a", "quantity": 100}]});
    let error = decode(body, &ORDER, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error, JsonDecodeError::new("items[0].quantity", "a value between(1, 99)", "100"));
}

#[test]
fn int_rejects_fractions_and_out_of_range_numbers() {
    let fraction = decode(json!(1.5), &Type::Int, UnknownFields::Ignore).unwrap_err();
    assert_eq!(fraction.found, "the fractional number 1.5");
    let too_large = decode(json!(18446744073709551615u64), &Type::Int, UnknownFields::Ignore).unwrap_err();
    assert_eq!(too_large.found, "the out-of-range integer 18446744073709551615");
    assert!(matches!(decode(json!(2), &Type::Float, UnknownFields::Ignore), Ok(Value::Float(f)) if f == 2.0));
}

#[test]
fn result_uses_ok_and_err_keys() {
    let ty = Type::Result(Box::new(Type::Int), Box::new(Type::String));
    assert_eq!(round_trip(json!({"ok": 3}), &ty), json!({"tag": "ok", "ok": 3}));
    assert_eq!(round_trip(json!({"tag": "err", "err": "no"}), &ty), json!({"tag": "err", "err": "no"}));
    let both = decode(json!({"ok": 3, "err": "no"}), &ty, UnknownFields::Ignore).unwrap_err();
    assert_eq!(both.found, "both `ok` and `err`");
    let contradicted = decode(json!({"tag": "err", "ok": 3}), &ty, UnknownFields::Ignore).unwrap_err();
    assert_eq!(contradicted, JsonDecodeError::new("tag", "\"ok\"", "\"err\""));
    let extra = decode(json!({"ok": 3, "note": 1}), &ty, UnknownFields::Reject).unwrap_err();
    assert_eq!(extra.path, "note");
}

#[test]
fn sum_types_keep_the_published_variant_envelope() {
    let encoded = round_trip(json!({"variant": "Changed", "fields": [2, "Dune"]}), &OUTCOME);
    assert_eq!(encoded, json!({"tag": "variant", "type": "Outcome", "variant": "Changed", "fields": [2, "Dune"]}));
    let unknown = decode(json!({"variant": "Lost"}), &OUTCOME, UnknownFields::Ignore).unwrap_err();
    assert_eq!(unknown, JsonDecodeError::new("variant", "one of Saved, Changed", "\"Lost\""));
    let arity = decode(json!({"variant": "Saved", "fields": []}), &OUTCOME, UnknownFields::Ignore).unwrap_err();
    assert_eq!(arity, JsonDecodeError::new("fields", "1 fields for `Saved`", "0 fields"));
}

#[test]
fn maps_with_string_keys_are_objects_and_other_keys_are_pairs() {
    let by_name = Type::Map(Box::new(Type::String), Box::new(Type::Int));
    assert_eq!(round_trip(json!({"a": 1, "b": 2}), &by_name), json!({"a": 1, "b": 2}));
    let by_number = Type::Map(Box::new(Type::Int), Box::new(Type::String));
    assert_eq!(round_trip(json!([[1, "a"]]), &by_number), json!([[1, "a"]]));
    let error = decode(json!({"a": "x"}), &by_name, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error.path, "a");
}

#[test]
fn keys_that_are_not_identifiers_are_quoted_in_paths() {
    let by_name = Type::Map(Box::new(Type::String), Box::new(Type::Int));
    let error = decode(json!({"user id": "x"}), &by_name, UnknownFields::Ignore).unwrap_err();
    assert_eq!(error.path, "[\"user id\"]");
}

#[test]
fn grounded_values_cannot_be_decoded_from_text() {
    let ty = Type::Grounded(Box::new(Type::String));
    let error = decode(json!({"value": "x", "sources": []}), &ty, UnknownFields::Ignore).unwrap_err();
    assert!(error.found.contains("cannot carry verified provenance"), "{error:?}");
}

#[test]
fn text_that_is_not_json_fails_at_the_root_with_its_position() {
    let fixture = Fixture::new();
    let broken = decode_public_json_text("{\"title\": }", &NOTE, &fixture.types(), UnknownFields::Ignore).unwrap_err();
    assert_eq!(broken, JsonDecodeError::new("", "JSON text", "text that is not valid JSON at line 1, column 11"));
    let short = decode_public_json_text("[1,", &Type::List(Box::new(Type::Int)), &fixture.types(), UnknownFields::Ignore)
        .unwrap_err();
    assert_eq!(short.found, "text that ends early, at line 1, column 3");
}

#[test]
fn a_json_error_value_carries_every_checker_declared_field() {
    let error = JsonDecodeError::new("items[1].quantity", "Int", "a string");
    let Value::Struct(record) = error.to_value() else { panic!("JsonError must be a record") };
    assert_eq!(record.type_name(), "JsonError");
    for (name, _) in corvid_types::json_error_fields() {
        assert!(record.get_field(name).is_some(), "runtime JsonError lacks `{name}`");
    }
    assert_eq!(
        record.get_field("message"),
        Some(Value::String(Arc::from("at `items[1].quantity`: expected Int, found a string")))
    );
}

#[test]
fn encoded_text_is_compact_with_sorted_keys() {
    let value = decode(json!({"title": "a", "tag": "x"}), &NOTE, UnknownFields::Reject).unwrap();
    assert_eq!(encode_public_json_text(&value).unwrap(), r#"{"tag":"x","title":"a"}"#);
}

#[test]
fn values_without_a_json_form_are_reported() {
    assert!(encode_public_json(&Value::Float(f64::NAN)).is_err());
    assert_eq!(encode_public_json(&Value::Nothing), Ok(J::Null));
}
