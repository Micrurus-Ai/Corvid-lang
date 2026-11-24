use super::*;
use corvid_abi::equality::EqualityLayout as Layout;

struct LayoutTable<'a, 'types> {
    lookup: &'a dyn Fn(DefId) -> Option<&'types corvid_ir::IrType>,
    offsets: HashMap<Type, usize>,
    words: Vec<u32>,
    span: Span,
}

impl LayoutTable<'_, '_> {
    fn reserve(&mut self, kind: Layout, children: usize, counted: bool) -> usize {
        let start = self.words.len();
        self.words.push(kind as u32);
        if counted {
            self.words.push(children as u32);
        }
        self.words.resize(self.words.len() + children, 0);
        start
    }

    fn fields(&mut self, start: usize, fields: &[corvid_ir::IrField]) -> Result<(), CodegenError> {
        for (i, field) in fields.iter().enumerate() {
            self.words[start + 2 + i] = self.node(&field.ty)? as u32;
        }
        Ok(())
    }

    fn node(&mut self, ty: &Type) -> Result<usize, CodegenError> {
        if let Type::ImportedStruct(imported) = ty {
            return self.node(&Type::Struct(imported.def_id));
        }
        if let Some(&index) = self.offsets.get(ty) {
            return Ok(index);
        }
        if let Type::Struct(id) = ty {
            let record = (self.lookup)(*id)
                .ok_or_else(|| CodegenError::cranelift("missing record layout", self.span))?;
            let variants = &record.variants;
            let index = if variants.is_empty() {
                self.reserve(Layout::Record, record.fields.len(), true)
            } else {
                self.reserve(Layout::Sum, variants.len(), true)
            };
            self.offsets.insert(ty.clone(), index);
            if variants.is_empty() {
                self.fields(index, &record.fields)?;
            } else {
                for (i, variant) in variants.iter().enumerate() {
                    let fields = self.reserve(Layout::Record, variant.fields.len(), true);
                    self.words[index + 2 + i] = fields as u32;
                    self.fields(fields, &variant.fields)?;
                }
            }
            return Ok(index);
        }
        let (kind, children) = match ty {
            Type::Int => (Layout::Int, vec![]),
            Type::Float => (Layout::Float, vec![]),
            Type::Bool => (Layout::Bool, vec![]),
            Type::String => (Layout::String, vec![]),
            Type::Decimal => (Layout::Decimal, vec![]),
            Type::List(item) => (Layout::List, vec![item.as_ref().clone()]),
            Type::Option(item) => (
                if option_uses_wrapper(ty) {
                    Layout::BoxedOption
                } else {
                    Layout::Option
                },
                vec![item.as_ref().clone()],
            ),
            Type::Result(ok, err) => (
                Layout::Result,
                vec![ok.as_ref().clone(), err.as_ref().clone()],
            ),
            Type::Map(_, value) if crate::native_string_map(ty) => {
                (Layout::StringMap, vec![value.as_ref().clone()])
            }
            Type::Function { .. } if crate::native_callback_type(ty) => (Layout::Identity, vec![]),
            _ => {
                return Err(CodegenError::not_supported(
                    format!("native equality for {}", ty.display_name()),
                    self.span,
                ))
            }
        };
        let index = self.reserve(kind, children.len(), false);
        self.offsets.insert(ty.clone(), index);
        for (i, child) in children.iter().enumerate() {
            self.words[index + 1 + i] = self.node(child)? as u32;
        }
        Ok(index)
    }
}

pub fn uses_structural_equality(op: BinaryOp, ty: &Type) -> bool {
    matches!(op, BinaryOp::Eq | BinaryOp::NotEq)
        && matches!(
            ty,
            Type::Struct(_)
                | Type::ImportedStruct(_)
                | Type::List(_)
                | Type::Map(_, _)
                | Type::Option(_)
                | Type::Result(_, _)
        )
}

// A literal such as None or [] can leave its element unspecified. The
// concrete operand supplies that layout. Never reinterpret numeric bits
// between unlike element types.
pub(super) fn comparison_type(left: &Type, right: &Type) -> Option<Type> {
    if left == right || *right == Type::Unknown {
        return Some(left.clone());
    }
    if *left == Type::Unknown {
        return Some(right.clone());
    }
    Some(match (left, right) {
        (Type::ImportedStruct(a), Type::Struct(b)) | (Type::Struct(b), Type::ImportedStruct(a))
            if a.def_id == *b =>
        {
            Type::Struct(*b)
        }
        (Type::List(a), Type::List(b)) => Type::List(Box::new(comparison_type(a, b)?)),
        (Type::Option(a), Type::Option(b)) => Type::Option(Box::new(comparison_type(a, b)?)),
        (Type::Map(k, v), Type::Map(l, w)) => Type::Map(
            Box::new(comparison_type(k, l)?),
            Box::new(comparison_type(v, w)?),
        ),
        (Type::Result(a, e), Type::Result(b, f)) => Type::Result(
            Box::new(comparison_type(a, b)?),
            Box::new(comparison_type(e, f)?),
        ),
        _ => return None,
    })
}

pub fn supports_structural_comparison(
    op: BinaryOp,
    left: &IrExpr,
    right: &IrExpr,
    types: &[corvid_ir::IrType],
) -> bool {
    if !uses_structural_equality(op, &left.ty) {
        return false;
    }
    if compares_to_none(left, right) {
        return true;
    }
    let Some(ty) = comparison_type(&left.ty, &right.ty) else {
        return false;
    };
    LayoutTable {
        lookup: &|id| types.iter().find(|ty| ty.id == id),
        offsets: HashMap::new(),
        words: Vec::new(),
        span: Span::new(0, 0),
    }
    .node(&ty)
    .is_ok()
}

fn compares_to_none(left: &IrExpr, right: &IrExpr) -> bool {
    matches!(left.ty, Type::Option(_))
        && matches!(right.ty, Type::Option(_))
        && (matches!(left.kind, IrExprKind::OptionNone)
            || matches!(right.kind, IrExprKind::OptionNone))
}

pub(super) fn equality_layout(
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    ty: &Type,
    span: Span,
) -> Result<cranelift_module::DataId, CodegenError> {
    if let Some(&id) = runtime.equality_layouts.borrow().get(ty) {
        return Ok(id);
    }
    let mut table = LayoutTable {
        lookup: &|id| runtime.ir_types.get(&id),
        offsets: HashMap::new(),
        words: Vec::new(),
        span,
    };
    table.node(ty)?;
    let mut data = DataDescription::new();
    data.set_align(4);
    data.define(
        table
            .words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let error = |e: cranelift_module::ModuleError| CodegenError::cranelift(e.to_string(), span);
    let id = module.declare_anonymous_data(false, false).map_err(error)?;
    module.define_data(id, &data).map_err(error)?;
    runtime.equality_layouts.borrow_mut().insert(ty.clone(), id);
    Ok(id)
}

pub(super) fn lower_comparison(
    builder: &mut FunctionBuilder,
    op: BinaryOp,
    left: &IrExpr,
    right: &IrExpr,
    current_return_ty: &Type,
    env: &HashMap<LocalId, (Variable, clir::Type)>,
    scope_stack: &Vec<Vec<(LocalId, Variable)>>,
    func_ids_by_def: &HashMap<DefId, FuncId>,
    module: &mut ObjectModule,
    runtime: &RuntimeFuncs,
    span: Span,
) -> Result<ClValue, CodegenError> {
    let ty = comparison_type(&left.ty, &right.ty).ok_or_else(|| {
        CodegenError::not_supported("native comparison requires compatible layouts", span)
    })?;
    let (a, borrowed_a) = lower_container_maybe_borrowed(
        builder,
        left,
        current_return_ty,
        env,
        scope_stack,
        func_ids_by_def,
        module,
        runtime,
    )?;
    if !borrowed_a {
        runtime.return_temporaries.borrow_mut().push(a);
    }
    let (b, borrowed_b) = lower_container_maybe_borrowed(
        builder,
        right,
        current_return_ty,
        env,
        scope_stack,
        func_ids_by_def,
        module,
        runtime,
    )?;
    let equal = if compares_to_none(left, right) {
        builder.ins().icmp(IntCC::Equal, a, b)
    } else {
        let layout = equality_layout(module, runtime, &ty, span)?;
        let data = module.declare_data_in_func(layout, builder.func);
        let address = builder.ins().symbol_value(I64, data);
        let compare = module.declare_func_in_func(runtime.structural_eq, builder.func);
        let call = builder.ins().call(compare, &[address, a, b]);
        builder.inst_results(call)[0]
    };
    if !borrowed_b {
        emit_release(builder, module, runtime, b);
    }
    if !borrowed_a {
        runtime.return_temporaries.borrow_mut().pop();
        emit_release(builder, module, runtime, a);
    }
    Ok(if op == BinaryOp::NotEq {
        builder.ins().icmp_imm(IntCC::Equal, equal, 0)
    } else {
        equal
    })
}
