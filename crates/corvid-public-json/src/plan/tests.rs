use super::*;
use crate::{decode, UnknownFields, ValueBuilder};

fn field(name: &str, ty: Type) -> IrField {
    IrField { name: name.into(), ty, refinement: None, span: Span::new(0, 0) }
}

/// `Line { sku: Tainted<String>, quantity: Int where between(1, 99) }`,
/// `Order { lines: List<Line>, tags: Map<String, List<Int>>, note: Option<Option<String>>,
///          outcome: Result<Outcome, String>, origin: Tainted<Option<Int>> }`,
/// `Outcome = Saved(Int) | Changed(Line)`.
fn decls() -> Vec<IrType> {
    let span = Span::new(0, 0);
    let mut quantity = field("quantity", Type::Int);
    quantity.refinement = Some(Refinement::Between { min: 1, max: 99 });
    let mut sku = field("sku", Type::Tainted(Box::new(Type::String)));
    sku.refinement = Some(Refinement::LenBetween { min: 1, max: 8 });
    vec![
        IrType { id: DefId(2), name: "Order".into(), fields: vec![
            field("lines", Type::List(Box::new(Type::Struct(DefId(1))))),
            field("tags", Type::Map(Box::new(Type::String), Box::new(Type::List(Box::new(Type::Int))))),
            field("note", Type::Option(Box::new(Type::Option(Box::new(Type::String))))),
            field("outcome", Type::Result(Box::new(Type::Struct(DefId(3))), Box::new(Type::String))),
            field("origin", Type::Tainted(Box::new(Type::Option(Box::new(Type::Int))))),
        ], variants: vec![], span },
        IrType { id: DefId(1), name: "Line".into(), fields: vec![sku, quantity], variants: vec![], span },
        IrType { id: DefId(3), name: "Outcome".into(), fields: vec![], variants: vec![
            IrEnumVariant { name: "Saved".into(), fields: vec![field("shelf", Type::Int)], span },
            IrEnumVariant { name: "Changed".into(), fields: vec![field("line", Type::Struct(DefId(1)))], span },
        ], span },
    ]
}

const ORDER: Type = Type::Struct(DefId(2));

#[test]
fn a_plan_reads_back_exactly_as_written() {
    let decls = decls();
    let by_id: HashMap<DefId, &IrType> = decls.iter().map(|t| (t.id, t)).collect();
    let types: Vec<IrType> = reachable_types(&ORDER, &by_id).into_iter().cloned().collect();
    assert_eq!(types.iter().map(|t| t.id).collect::<Vec<_>>(), vec![DefId(2), DefId(1), DefId(3)]);
    let shapes = builder_shapes(&ORDER, &by_id).into_iter().enumerate().map(|(i, s)| (s, i as u32)).collect();
    let plan = DecodePlan {
        root: ORDER,
        types,
        shapes,
        owning_maps: vec![Type::Map(Box::new(Type::String), Box::new(Type::List(Box::new(Type::Int))))],
    };
    let read = DecodePlan::from_text(&plan.to_text().expect("writes")).expect("reads");
    assert_eq!(read.root, plan.root);
    assert_eq!(read.shapes, plan.shapes);
    assert_eq!(read.owning_maps, plan.owning_maps);
    for (a, b) in read.types.iter().zip(&plan.types) {
        assert_eq!((a.id, &a.name), (b.id, &b.name));
        let fields = |t: &IrType| t.fields.iter().chain(t.variants.iter().flat_map(|v| &v.fields))
            .map(|f| (f.name.clone(), f.ty.clone(), f.refinement)).collect::<Vec<_>>();
        assert_eq!(fields(a), fields(b));
        assert_eq!(a.variants.iter().map(|v| &v.name).collect::<Vec<_>>(), b.variants.iter().map(|v| &v.name).collect::<Vec<_>>());
    }
}

#[test]
fn a_type_without_a_plan_form_is_refused_by_name() {
    let plan = DecodePlan { root: Type::Stream(Box::new(Type::Int)), types: vec![], shapes: BTreeMap::new(), owning_maps: vec![] };
    let error = plan.to_text().expect_err("streams have no plan form");
    assert!(error.contains("Stream<Int>"), "{error}");
}

#[test]
fn every_text_scalar_type_has_a_plan_form() {
    for name in crate::text_scalar::NAMES {
        let ty = corvid_types::Type::compiler_known(name).expect("a text scalar is a compiler-known type");
        let root = Type::List(Box::new(Type::Option(Box::new(ty))));
        let plan = DecodePlan { root: root.clone(), types: vec![], shapes: BTreeMap::new(), owning_maps: vec![] };
        let read = DecodePlan::from_text(&plan.to_text().expect("writes")).expect("reads");
        assert_eq!(read.root, root);
    }
}

/// Records each shape the decoder asks for.
#[derive(Default)]
struct Recording(Vec<Shape>);

impl ValueBuilder for Recording {
    type Value = ();
    fn int(&mut self, _: i64) {}
    fn float(&mut self, _: f64) {}
    fn string(&mut self, _: &str) {}
    fn bool(&mut self, _: bool) {}
    fn nothing(&mut self) {}
    fn none(&mut self, option: &Type) { self.0.push(Shape::Type(option.clone())) }
    fn some(&mut self, option: &Type, _: ()) { self.0.push(Shape::Type(option.clone())) }
    fn list(&mut self, list: &Type, _: Vec<()>) { self.0.push(Shape::Type(list.clone())) }
    fn map(&mut self, map: &Type, _: Vec<((), ())>) { self.0.push(Shape::Type(map.clone())) }
    fn ok(&mut self, result: &Type, _: ()) { self.0.push(Shape::Type(result.clone())) }
    fn err(&mut self, result: &Type, _: ()) { self.0.push(Shape::Type(result.clone())) }
    fn record(&mut self, record: &IrType, _: Vec<()>) { self.0.push(Shape::Record(record.id)) }
    fn variant(&mut self, sum: &IrType, index: usize, _: Vec<()>) { self.0.push(Shape::Variant(sum.id, index as u32)) }
    fn json(&mut self, _: &J) {}
    fn text_scalar(&mut self, _: crate::text_scalar::TextScalar) {}
}

#[test]
fn the_plan_names_every_shape_the_decoder_builds() {
    let decls = decls();
    let by_id: HashMap<DefId, &IrType> = decls.iter().map(|t| (t.id, t)).collect();
    let planned: HashSet<Shape> = builder_shapes(&ORDER, &by_id).into_iter().collect();
    let inputs = [
        json!({"lines": [{"sku": "a", "quantity": 2}], "tags": {"x": [1, 2]}, "note": "n",
               "outcome": {"ok": {"variant": "Changed", "fields": [{"sku": "b", "quantity": 3}]}}, "origin": 4}),
        json!({"lines": [], "tags": {}, "note": null, "outcome": {"err": "no"}, "origin": null}),
        // `note` is absent; an absent `Option` field decodes as `None`.
        json!({"lines": [], "tags": {}, "outcome": {"ok": {"variant": "Saved", "fields": [1]}}, "origin": null}),
    ];
    for input in inputs {
        let mut recording = Recording::default();
        decode(&input, &ORDER, &by_id, UnknownFields::Reject, &mut recording).expect("decodes");
        for shape in recording.0 {
            assert!(planned.contains(&shape), "{shape:?} built for {input} is not in the plan");
        }
    }
}
