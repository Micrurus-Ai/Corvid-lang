//! The decoder's ownership contract: on success every built value is
//! consumed into the root, and on failure every built value is handed
//! back through `discard`. The native runtime counts references, so a
//! value that is neither consumed nor discarded is a leak there.
//! The mapping's accepted shapes and messages are tested through the
//! interpreter in `corvid-vm`'s `public_json` tests.

use super::*;
use corvid_ast::{Refinement, Span};
use corvid_ir::{IrEnumVariant, IrField};
use serde_json::json;
use std::collections::HashSet;

/// Values are tokens; `live` holds every token built and not yet
/// consumed by a parent or discarded.
#[derive(Default)]
struct Counting {
    next: u32,
    live: HashSet<u32>,
}

impl Counting {
    fn make(&mut self) -> u32 {
        self.next += 1;
        self.live.insert(self.next);
        self.next
    }
    fn consume(&mut self, token: u32) {
        assert!(self.live.remove(&token), "token {token} consumed twice or never built");
    }
}

impl ValueBuilder for Counting {
    type Value = u32;
    fn int(&mut self, _: i64) -> u32 { self.make() }
    fn float(&mut self, _: f64) -> u32 { self.make() }
    fn string(&mut self, _: &str) -> u32 { self.make() }
    fn bool(&mut self, _: bool) -> u32 { self.make() }
    fn nothing(&mut self) -> u32 { self.make() }
    fn none(&mut self, _: &Type) -> u32 { self.make() }
    fn some(&mut self, _: &Type, value: u32) -> u32 { self.consume(value); self.make() }
    fn list(&mut self, _: &Type, items: Vec<u32>) -> u32 {
        items.into_iter().for_each(|item| self.consume(item));
        self.make()
    }
    fn map(&mut self, _: &Type, entries: Vec<(u32, u32)>) -> u32 {
        for (key, value) in entries {
            self.consume(key);
            self.consume(value);
        }
        self.make()
    }
    fn ok(&mut self, _: &Type, value: u32) -> u32 { self.consume(value); self.make() }
    fn err(&mut self, _: &Type, value: u32) -> u32 { self.consume(value); self.make() }
    fn record(&mut self, record: &IrType, fields: Vec<u32>) -> u32 {
        assert_eq!(fields.len(), record.fields.len(), "a record gets every declared field");
        fields.into_iter().for_each(|field| self.consume(field));
        self.make()
    }
    fn variant(&mut self, sum: &IrType, index: usize, fields: Vec<u32>) -> u32 {
        assert_eq!(fields.len(), sum.variants[index].fields.len());
        fields.into_iter().for_each(|field| self.consume(field));
        self.make()
    }
    fn json(&mut self, _: &J) -> u32 { self.make() }
    fn text_scalar(&mut self, _: crate::text_scalar::TextScalar) -> u32 { self.make() }
    fn discard(&mut self, value: u32) { self.consume(value); }
}

fn field(name: &str, ty: Type) -> IrField {
    IrField { name: name.into(), ty, refinement: None, json_key: None, span: Span::new(0, 0) }
}

/// `Line { sku: String, quantity: Int where between(1, 99) }`,
/// `Order { id: Int, lines: List<Line>, tags: Map<String, Int>, note: Option<String> }`,
/// `Outcome = Saved(Int) | Changed(Int, String)`.
fn types() -> Vec<IrType> {
    let mut quantity = field("quantity", Type::Int);
    quantity.refinement = Some(Refinement::Between { min: 1, max: 99 });
    let span = Span::new(0, 0);
    vec![
        IrType { id: DefId(1), name: "Line".into(), fields: vec![field("sku", Type::String), quantity], variants: vec![], span },
        IrType {
            id: DefId(2),
            name: "Order".into(),
            fields: vec![
                field("id", Type::Int),
                field("lines", Type::List(Box::new(Type::Struct(DefId(1))))),
                field("tags", Type::Map(Box::new(Type::String), Box::new(Type::Int))),
                field("note", Type::Option(Box::new(Type::String))),
            ],
            variants: vec![],
            span,
        },
        IrType {
            id: DefId(3),
            name: "Outcome".into(),
            fields: vec![],
            variants: vec![
                IrEnumVariant { name: "Saved".into(), fields: vec![field("shelf", Type::Int)], span },
                IrEnumVariant { name: "Changed".into(), fields: vec![field("shelf", Type::Int), field("title", Type::String)], span },
            ],
            span,
        },
    ]
}

fn run(json: J, ty: &Type) -> (Result<u32, JsonDecodeError>, Counting) {
    let types = types();
    let by_id: HashMap<DefId, &IrType> = types.iter().map(|t| (t.id, t)).collect();
    let mut builder = Counting::default();
    let result = decode(&json, ty, &by_id, UnknownFields::Reject, &mut builder);
    (result, builder)
}

const ORDER: Type = Type::Struct(DefId(2));

#[test]
fn a_successful_decode_leaves_only_the_root_alive() {
    let (result, builder) = run(
        json!({"id": 7, "lines": [{"sku": "a", "quantity": 2}], "tags": {"x": 1}, "note": null}),
        &ORDER,
    );
    let root = result.expect("decodes");
    assert_eq!(builder.live, HashSet::from([root]));
}

#[test]
fn every_failure_releases_what_was_built_before_it() {
    let failures = [
        // A refinement failure after the field itself decoded.
        json!({"id": 7, "lines": [{"sku": "a", "quantity": 2}, {"sku": "b", "quantity": 100}], "tags": {}}),
        // A wrong kind deep in a list, after earlier elements built.
        json!({"id": 7, "lines": [{"sku": "a", "quantity": 2}, {"sku": 5, "quantity": 2}], "tags": {}}),
        // A map value failing after other entries built.
        json!({"id": 7, "lines": [], "tags": {"a": 1, "b": "two"}}),
        // A missing required field after earlier fields built.
        json!({"id": 7, "lines": [{"sku": "a", "quantity": 2}]}),
    ];
    for json in failures {
        let (result, builder) = run(json.clone(), &ORDER);
        assert!(result.is_err(), "{json} must fail");
        assert!(builder.live.is_empty(), "{json} leaked {:?}", builder.live);
    }
}

#[test]
fn an_absent_tainted_option_field_is_none() {
    let span = Span::new(0, 0);
    let note = IrType {
        id: DefId(9),
        name: "Note".into(),
        fields: vec![field("origin", Type::Tainted(Box::new(Type::Option(Box::new(Type::Int)))))],
        variants: vec![],
        span,
    };
    let by_id: HashMap<DefId, &IrType> = [(note.id, &note)].into_iter().collect();
    let mut builder = Counting::default();
    let decoded = decode(&json!({}), &Type::Struct(DefId(9)), &by_id, UnknownFields::Reject, &mut builder);
    assert!(decoded.is_ok(), "{decoded:?}");
}

#[test]
fn failures_inside_results_pairs_and_variants_release_too() {
    let outcome = Type::Struct(DefId(3));
    let result_ty = Type::Result(Box::new(Type::List(Box::new(Type::Int))), Box::new(Type::String));
    let pairs = Type::Map(Box::new(Type::Int), Box::new(Type::String));
    let cases = [
        (json!({"ok": [1, 2, "three"]}), result_ty),
        (json!([[1, "a"], [2, 3]]), pairs.clone()),
        (json!([[1, "a"], ["two", "b"]]), pairs),
        (json!({"variant": "Changed", "fields": [3, 4]}), outcome),
    ];
    for (json, ty) in cases {
        let (result, builder) = run(json.clone(), &ty);
        assert!(result.is_err(), "{json} must fail");
        assert!(builder.live.is_empty(), "{json} leaked {:?}", builder.live);
    }
}
