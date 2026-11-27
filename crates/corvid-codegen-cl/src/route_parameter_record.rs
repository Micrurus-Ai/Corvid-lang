//! Native record layouts for the checked path and caller of a route.

use corvid_ast::Span;
use corvid_ir::{IrField, IrFile, IrType};
use corvid_resolve::DefId;
use corvid_types::{Type, RESPONSE_RECORD_IDS};
use std::collections::{BTreeMap, BTreeSet};

/// Intern structural field lists once, before native ownership and layout
/// analysis. The resulting records use exactly the source declaration order.
pub(crate) fn represent_route_parameters_as_records(ir: &mut IrFile) {
    let declared: BTreeSet<_> = ir.types.iter().map(|record| record.id).collect();
    let mut available = (0..RESPONSE_RECORD_IDS.start)
        .rev()
        .map(DefId)
        .filter(|id| !declared.contains(id));
    let mut layouts = BTreeMap::new();
    corvid_ir::for_each_compiled_type_mut(ir, &mut |slot| {
        slot.replace_within(&mut |ty| {
            let Type::RouteParams(fields) = ty else {
                return false;
            };
            let id = *layouts
                .entry(fields.clone())
                .or_insert_with(|| available.next().expect("native type identifiers exhausted"));
            *ty = Type::Struct(id);
            true
        });
    });
    let span = Span::new(0, 0);
    for (fields, id) in layouts {
        let name = "RouteParameters".to_string();
        ir.types.push(IrType {
            id,
            name,
            fields: fields
                .into_iter()
                .map(|(name, ty)| IrField {
                    name,
                    ty,
                    refinement: None,
                    json_key: None,
                    span,
                })
                .collect(),
            variants: Vec::new(),
            span,
        });
    }
}
