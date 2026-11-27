use super::*;

pub(in crate::lowering) fn declare_runtime_funcs(
    module: &mut ObjectModule,
    overflow_func_id: FuncId,
) -> Result<RuntimeFuncs, CodegenError> {
    let mut retain_sig = module.make_signature();
    retain_sig.params.push(AbiParam::new(I64));
    let retain_id = module
        .declare_function(RETAIN_SYMBOL, Linkage::Import, &retain_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare retain: {e}"), Span::new(0, 0)))?;

    let mut release_sig = module.make_signature();
    release_sig.params.push(AbiParam::new(I64));
    let release_id = module
        .declare_function(RELEASE_SYMBOL, Linkage::Import, &release_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare release: {e}"), Span::new(0, 0)))?;

    let mut concat_sig = module.make_signature();
    concat_sig.params.push(AbiParam::new(I64));
    concat_sig.params.push(AbiParam::new(I64));
    concat_sig.returns.push(AbiParam::new(I64));
    let concat_id = module
        .declare_function(STRING_CONCAT_SYMBOL, Linkage::Import, &concat_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_concat: {e}"), Span::new(0, 0))
        })?;

    let mut grounded_concat_sig = concat_sig.clone();
    grounded_concat_sig.params.push(AbiParam::new(I8));
    let grounded_string_concat = module
        .declare_function("corvid_grounded_string_concat", Linkage::Import, &grounded_concat_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare grounded string concat: {e}"), Span::new(0, 0)))?;

    let mut declare = |name: &str, params: usize, returns_value: bool| {
        let mut sig = module.make_signature();
        sig.params.resize(params, AbiParam::new(I64));
        if returns_value { sig.returns.push(AbiParam::new(I64)); }
        module.declare_function(name, Linkage::Import, &sig)
            .map_err(|error| CodegenError::cranelift(format!("declare {name}: {error}"), Span::new(0, 0)))
    };
    let list_map_int = declare("corvid_list_map_int", 4, true)?;
    let list_filter_int = declare("corvid_list_filter_int", 4, true)?;
    let list_fold_int = declare("corvid_list_fold_int", 5, true)?;
    let list_snapshot = declare("corvid_list_snapshot", 2, true)?;
    let list_concat = declare("corvid_list_concat", 3, true)?;
    let list_new = declare("corvid_list_new", 2, true)?;
    let list_bounds_error = declare("corvid_list_bounds_error", 2, false)?;
    let list_append = declare("corvid_list_append", 2, false)?;
    let map_new = declare("corvid_ordered_map_new_keyed", 3, true)?;
    let map_set = declare("corvid_ordered_map_set", 3, false)?;
    let map_require_int = declare("corvid_ordered_map_require_int", 2, true)?;
    let map_find = declare("corvid_ordered_map_find", 2, true)?;
    let map_snapshot = declare("corvid_ordered_map_snapshot", 3, true)?;
    let mut test_sig = module.make_signature();
    test_sig.params.resize(5, AbiParam::new(I64));
    test_sig.returns.push(AbiParam::new(I8));
    let list_test_int = module.declare_function("corvid_list_test_int", Linkage::Import, &test_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare list any/all: {e}"), Span::new(0, 0)))?;
    let mut structural_sig = module.make_signature();
    structural_sig.params.resize(3, AbiParam::new(I64));
    structural_sig.returns.push(AbiParam::new(I8));
    let structural_eq = module.declare_function("corvid_structural_eq", Linkage::Import, &structural_sig)
        .map_err(|e| CodegenError::cranelift(e.to_string(), Span::new(0, 0)))?;
    let mut eq_sig = module.make_signature();
    eq_sig.params.push(AbiParam::new(I64));
    eq_sig.params.push(AbiParam::new(I64));
    eq_sig.returns.push(AbiParam::new(I64));
    let eq_id = module
        .declare_function(STRING_EQ_SYMBOL, Linkage::Import, &eq_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare string_eq: {e}"), Span::new(0, 0)))?;

    let mut cmp_sig = module.make_signature();
    cmp_sig.params.push(AbiParam::new(I64));
    cmp_sig.params.push(AbiParam::new(I64));
    cmp_sig.returns.push(AbiParam::new(I64));
    let cmp_id = module
        .declare_function(STRING_CMP_SYMBOL, Linkage::Import, &cmp_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_cmp: {e}"), Span::new(0, 0))
        })?;

    let mut char_len_sig = module.make_signature();
    char_len_sig.params.push(AbiParam::new(I64));
    char_len_sig.returns.push(AbiParam::new(I64));
    let string_char_len_id = module
        .declare_function(STRING_CHAR_LEN_SYMBOL, Linkage::Import, &char_len_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_char_len: {e}"), Span::new(0, 0))
        })?;

    let mut substring_sig = char_len_sig.clone();
    substring_sig.params.extend([AbiParam::new(I64), AbiParam::new(I64)]);
    let mut declare_string_method = |name: &str, sig: &cranelift_codegen::ir::Signature| {
        module.declare_function(name, Linkage::Import, sig)
            .map_err(|e| CodegenError::cranelift(format!("declare {name}: {e}"), Span::new(0, 0)))
    };
    let string_to_upper = declare_string_method("corvid_string_to_upper", &char_len_sig)?;
    let string_to_lower = declare_string_method("corvid_string_to_lower", &char_len_sig)?;
    let string_substring = declare_string_method("corvid_string_substring", &substring_sig)?;
    let string_trim = declare_string_method("corvid_string_trim", &char_len_sig)?;
    let mut search_sig = concat_sig.clone();
    search_sig.returns[0] = AbiParam::new(I8);
    let string_contains = declare_string_method("corvid_string_contains", &search_sig)?;
    let string_starts_with = declare_string_method("corvid_string_starts_with", &search_sig)?;
    let string_ends_with = declare_string_method("corvid_string_ends_with", &search_sig)?;
    let string_replace = declare_string_method("corvid_string_replace", &substring_sig)?;
    let string_split = declare_string_method("corvid_string_split", &substring_sig)?;
    let mut char_at_sig = module.make_signature();
    char_at_sig.params.push(AbiParam::new(I64));
    char_at_sig.params.push(AbiParam::new(I64));
    char_at_sig.returns.push(AbiParam::new(I64));
    let string_parse_int_id = module
        .declare_function("corvid_string_parse_int", Linkage::Import, &char_at_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare string_parse_int: {e}"), Span::new(0, 0)))?;
    // B03 text methods: encoders take the text and return a String;
    // decoders also take a Result payload slot and return its tag,
    // the `corvid_string_parse_int` shape.
    let mut regex_find_sig = module.make_signature();
    for _ in 0..5 {
        regex_find_sig.params.push(AbiParam::new(I64));
    }
    regex_find_sig.returns.push(AbiParam::new(I64));
    let mut text_methods = Vec::new();
    for kind in crate::text_methods::NATIVE_TEXT_METHODS {
        let Some((symbol, shape)) = crate::text_methods::native_text_symbol(kind) else { continue };
        use crate::text_methods::NativeTextShape;
        let sig = match shape {
            NativeTextShape::Encoder => &char_len_sig,
            NativeTextShape::Decoder => &char_at_sig,
            NativeTextShape::RegexSearch => &search_sig,
            NativeTextShape::RegexReplace | NativeTextShape::RegexSplit => &substring_sig,
            NativeTextShape::RegexFind => &regex_find_sig,
        };
        let id = module
            .declare_function(symbol, Linkage::Import, sig)
            .map_err(|e| CodegenError::cranelift(format!("declare {symbol}: {e}"), Span::new(0, 0)))?;
        text_methods.push((kind, id));
    }
    // B06 `Decimal`: one import per operation, shaped by its table entry.
    let mut decimal_operations = HashMap::new();
    for operation in crate::decimal_methods::DecimalOperation::all() {
        use crate::decimal_methods::DecimalSlot;
        let width = |slot: DecimalSlot| match slot {
            DecimalSlot::Word => I64,
            DecimalSlot::Float => F64,
        };
        let (params, returns) = operation.signature();
        let mut sig = module.make_signature();
        sig.params.extend(params.iter().map(|slot| AbiParam::new(width(*slot))));
        sig.returns.push(AbiParam::new(width(returns)));
        let symbol = operation.symbol();
        let id = module
            .declare_function(&symbol, Linkage::Import, &sig)
            .map_err(|e| CodegenError::cranelift(format!("declare {symbol}: {e}"), Span::new(0, 0)))?;
        decimal_operations.insert(operation, id);
    }
    // B06 time and currency types: one import per operation, every
    // parameter and the return one word.
    let mut declare_word_import = |symbol: &str, arity: usize| {
        let mut sig = module.make_signature();
        sig.params.extend((0..arity).map(|_| AbiParam::new(I64)));
        sig.returns.push(AbiParam::new(I64));
        module
            .declare_function(symbol, Linkage::Import, &sig)
            .map_err(|e| CodegenError::cranelift(format!("declare {symbol}: {e}"), Span::new(0, 0)))
    };
    let mut time_operations = HashMap::new();
    for operation in crate::time_methods::TimeOperation::all() {
        time_operations.insert(operation, declare_word_import(&operation.symbol(), operation.arity())?);
    }
    let mut currency_operations = HashMap::new();
    for operation in crate::currency_methods::CurrencyOperation::all() {
        currency_operations.insert(operation, declare_word_import(operation.symbol(), operation.arity())?);
    }
    // B04 `Bytes`: the same one-word shape.
    let mut bytes_operations = HashMap::new();
    for operation in crate::bytes_methods::BytesOperation::all() {
        bytes_operations.insert(operation, declare_word_import(operation.symbol(), operation.arity())?);
    }
    // corvid_json_decode_typed(text, plan, plan_len, typeinfos, strict, payload) -> tag
    let mut json_decode_sig = module.make_signature();
    for _ in 0..6 {
        json_decode_sig.params.push(AbiParam::new(I64));
    }
    json_decode_sig.returns.push(AbiParam::new(I64));
    let json_decode_typed_id = module
        .declare_function("corvid_json_decode_typed", Linkage::Import, &json_decode_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare json_decode_typed: {e}"), Span::new(0, 0)))?;
    // corvid_runtime_call(name, args, arg_count, plan, plan_len, typeinfos, payload) -> tag
    // corvid_runtime_call(name, args, count, plan, plan_len, typeinfos,
    // payload, revealed) -> tag; `revealed` is null unless the tool takes
    // a secret (native dispatch N3c).
    let mut runtime_call_sig = module.make_signature();
    for _ in 0..8 {
        runtime_call_sig.params.push(AbiParam::new(I64));
    }
    runtime_call_sig.returns.push(AbiParam::new(I64));
    let runtime_call_id = module
        .declare_function("corvid_runtime_call", Linkage::Import, &runtime_call_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare runtime_call: {e}"), Span::new(0, 0)))?;
    // corvid_json_encode_tool_argument(name, value, plan, plan_len,
    // typeinfos, revealed) -> String (native dispatch N3c).
    let mut tool_argument_sig = module.make_signature();
    for _ in 0..6 {
        tool_argument_sig.params.push(AbiParam::new(I64));
    }
    tool_argument_sig.returns.push(AbiParam::new(I64));
    let json_encode_tool_argument = module
        .declare_function("corvid_json_encode_tool_argument", Linkage::Import, &tool_argument_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare json_encode_tool_argument: {e}"), Span::new(0, 0)))?;
    // corvid_json_encode_recorded(value, plan, plan_len, typeinfos) ->
    // String: a value a trace records, secrets redacted, never failing.
    let mut recorded_sig = module.make_signature();
    for _ in 0..4 {
        recorded_sig.params.push(AbiParam::new(I64));
    }
    recorded_sig.returns.push(AbiParam::new(I64));
    let json_encode_recorded = module
        .declare_function("corvid_json_encode_recorded", Linkage::Import, &recorded_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare json_encode_recorded: {e}"), Span::new(0, 0)))?;
    // corvid_json_encode_typed(value, plan, plan_len, typeinfos) -> String
    let mut json_encode_sig = module.make_signature();
    for _ in 0..4 {
        json_encode_sig.params.push(AbiParam::new(I64));
    }
    json_encode_sig.returns.push(AbiParam::new(I64));
    let json_encode_typed_id = module
        .declare_function("corvid_json_encode_typed", Linkage::Import, &json_encode_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare json_encode_typed: {e}"), Span::new(0, 0)))?;
    let string_char_at_id = module
        .declare_function(STRING_CHAR_AT_SYMBOL, Linkage::Import, &char_at_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_char_at: {e}"), Span::new(0, 0))
        })?;

    // corvid_alloc_typed(size: i64, typeinfo: i64) -> i64
    let mut alloc_typed_sig = module.make_signature();
    alloc_typed_sig.params.push(AbiParam::new(I64));
    alloc_typed_sig.params.push(AbiParam::new(I64));
    alloc_typed_sig.returns.push(AbiParam::new(I64));
    let alloc_typed_id = module
        .declare_function(ALLOC_TYPED_SYMBOL, Linkage::Import, &alloc_typed_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare alloc_typed: {e}"), Span::new(0, 0))
        })?;

    // corvid_destroy_list(payload) -> void — installed in every
    // refcounted-element list's typeinfo, referenced from the data
    // emission via write_function_addr.
    let mut list_destroy_sig = module.make_signature();
    list_destroy_sig.params.push(AbiParam::new(I64));
    let list_destroy_id = module
        .declare_function(LIST_DESTROY_SYMBOL, Linkage::Import, &list_destroy_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare list destroy: {e}"), Span::new(0, 0))
        })?;

    // corvid_trace_list(payload, marker, ctx) -> void — installed in
    // every list's typeinfo (primitive-element included; the fn
    // no-ops when elem_typeinfo is NULL).
    let mut list_trace_sig = module.make_signature();
    list_trace_sig.params.push(AbiParam::new(I64));
    list_trace_sig.params.push(AbiParam::new(I64));
    list_trace_sig.params.push(AbiParam::new(I64));
    let list_trace_id = module
        .declare_function(LIST_TRACE_SYMBOL, Linkage::Import, &list_trace_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare list trace: {e}"), Span::new(0, 0))
        })?;

    let mut weak_unary_sig = module.make_signature();
    weak_unary_sig.params.push(AbiParam::new(I64));
    weak_unary_sig.returns.push(AbiParam::new(I64));
    let weak_new_id = module
        .declare_function(WEAK_NEW_SYMBOL, Linkage::Import, &weak_unary_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare weak_new: {e}"), Span::new(0, 0)))?;
    let weak_upgrade_id = module
        .declare_function(WEAK_UPGRADE_SYMBOL, Linkage::Import, &weak_unary_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare weak_upgrade: {e}"), Span::new(0, 0))
        })?;

    let mut weak_clear_sig = module.make_signature();
    weak_clear_sig.params.push(AbiParam::new(I64));
    let weak_clear_self_id = module
        .declare_function(WEAK_CLEAR_SELF_SYMBOL, Linkage::Import, &weak_clear_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare weak_clear_self: {e}"), Span::new(0, 0))
        })?;

    // corvid_typeinfo_String — runtime-provided data symbol. Declared
    // here so codegen can reference it from static string literal
    // descriptors and from List<String>'s elem_typeinfo slot.
    let string_typeinfo_id = module
        .declare_data(STRING_TYPEINFO_SYMBOL, Linkage::Import, false, false)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare String typeinfo: {e}"), Span::new(0, 0))
        })?;
    // Native dispatch N3 — the runtime's `Secret` object, for a list or
    // record slot that holds one.
    let secret_typeinfo = module
        .declare_data("corvid_typeinfo_Secret", Linkage::Import, false, false)
        .map_err(|e| CodegenError::cranelift(format!("declare Secret typeinfo: {e}"), Span::new(0, 0)))?;
    let map_typeinfo = module
        .declare_data("corvid_typeinfo_OrderedMap", Linkage::Import, false, false)
        .map_err(|e| CodegenError::cranelift(format!("declare Map typeinfo: {e}"), Span::new(0, 0)))?;
    let weak_box_typeinfo_id = module
        .declare_data(WEAK_BOX_TYPEINFO_SYMBOL, Linkage::Import, false, false)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare WeakBox typeinfo: {e}"), Span::new(0, 0))
        })?;

    // ---- native entry helpers ----
    let void_void_sig = module.make_signature();
    let entry_init_id = module
        .declare_function(ENTRY_INIT_SYMBOL, Linkage::Import, &void_void_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare corvid_init: {e}"), Span::new(0, 0))
        })?;

    let mut arity_sig = module.make_signature();
    arity_sig.params.push(AbiParam::new(I64));
    arity_sig.params.push(AbiParam::new(I64));
    let arity_id = module
        .declare_function(ENTRY_ARITY_MISMATCH_SYMBOL, Linkage::Import, &arity_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare arity_mismatch: {e}"), Span::new(0, 0))
        })?;

    // parse helpers: (cstr_ptr, argv_index) -> typed value
    let mut parse_i64_sig = module.make_signature();
    parse_i64_sig.params.push(AbiParam::new(I64));
    parse_i64_sig.params.push(AbiParam::new(I64));
    parse_i64_sig.returns.push(AbiParam::new(I64));
    let parse_i64_id = module
        .declare_function(PARSE_I64_SYMBOL, Linkage::Import, &parse_i64_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare parse_i64: {e}"), Span::new(0, 0)))?;

    let mut parse_f64_sig = module.make_signature();
    parse_f64_sig.params.push(AbiParam::new(I64));
    parse_f64_sig.params.push(AbiParam::new(I64));
    parse_f64_sig.returns.push(AbiParam::new(F64));
    let parse_f64_id = module
        .declare_function(PARSE_F64_SYMBOL, Linkage::Import, &parse_f64_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare parse_f64: {e}"), Span::new(0, 0)))?;

    let mut parse_bool_sig = module.make_signature();
    parse_bool_sig.params.push(AbiParam::new(I64));
    parse_bool_sig.params.push(AbiParam::new(I64));
    parse_bool_sig.returns.push(AbiParam::new(I8));
    let parse_bool_id = module
        .declare_function(PARSE_BOOL_SYMBOL, Linkage::Import, &parse_bool_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare parse_bool: {e}"), Span::new(0, 0))
        })?;

    // corvid_string_from_cstr(cstr_ptr) -> descriptor
    let mut from_cstr_sig = module.make_signature();
    from_cstr_sig.params.push(AbiParam::new(I64));
    from_cstr_sig.returns.push(AbiParam::new(I64));
    let from_cstr_id = module
        .declare_function(STRING_FROM_CSTR_SYMBOL, Linkage::Import, &from_cstr_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_from_cstr: {e}"), Span::new(0, 0))
        })?;

    // print helpers
    let mut print_i64_sig = module.make_signature();
    print_i64_sig.params.push(AbiParam::new(I64));
    let print_i64_id = module
        .declare_function(PRINT_I64_SYMBOL, Linkage::Import, &print_i64_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare print_i64: {e}"), Span::new(0, 0)))?;

    let mut print_bool_sig = module.make_signature();
    print_bool_sig.params.push(AbiParam::new(I64));
    let print_bool_id = module
        .declare_function(PRINT_BOOL_SYMBOL, Linkage::Import, &print_bool_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare print_bool: {e}"), Span::new(0, 0))
        })?;

    let mut print_f64_sig = module.make_signature();
    print_f64_sig.params.push(AbiParam::new(F64));
    let print_f64_id = module
        .declare_function(PRINT_F64_SYMBOL, Linkage::Import, &print_f64_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare print_f64: {e}"), Span::new(0, 0)))?;

    let mut print_string_sig = module.make_signature();
    print_string_sig.params.push(AbiParam::new(I64));
    let print_string_id = module
        .declare_function(PRINT_STRING_SYMBOL, Linkage::Import, &print_string_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare print_string: {e}"), Span::new(0, 0))
        })?;
    let mut bench_enabled_sig = module.make_signature();
    bench_enabled_sig.returns.push(AbiParam::new(I64));
    let bench_server_enabled_id = module
        .declare_function(
            BENCH_SERVER_ENABLED_SYMBOL,
            Linkage::Import,
            &bench_enabled_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare bench_server_enabled: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut bench_next_sig = module.make_signature();
    bench_next_sig.returns.push(AbiParam::new(I64));
    let bench_next_trial_id = module
        .declare_function(BENCH_NEXT_TRIAL_SYMBOL, Linkage::Import, &bench_next_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare bench_next_trial: {e}"), Span::new(0, 0))
        })?;

    let mut bench_finish_sig = module.make_signature();
    bench_finish_sig.params.push(AbiParam::new(I64));
    let bench_finish_trial_id = module
        .declare_function(
            BENCH_FINISH_TRIAL_SYMBOL,
            Linkage::Import,
            &bench_finish_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare bench_finish_trial: {e}"), Span::new(0, 0))
        })?;

    let mut runtime_init_sig = module.make_signature();
    runtime_init_sig.returns.push(AbiParam::new(I32));
    let runtime_init_id = module
        .declare_function(RUNTIME_INIT_SYMBOL, Linkage::Import, &runtime_init_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare runtime_init: {e}"), Span::new(0, 0))
        })?;

    let mut register_connectors_sig = module.make_signature();
    register_connectors_sig.params.push(AbiParam::new(I64));
    let register_connectors_id = module
        .declare_function("corvid_register_connectors", Linkage::Import, &register_connectors_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare register_connectors: {e}"), Span::new(0, 0)))?;

    // `int.to_string()`, `float.to_string()`, `bool.to_string()`, in the
    // interpreter's text.
    let mut int_text_sig = module.make_signature();
    int_text_sig.params.push(AbiParam::new(I64));
    int_text_sig.returns.push(AbiParam::new(I64));
    let int_to_string = module
        .declare_function("corvid_int_to_string", Linkage::Import, &int_text_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare int_to_string: {e}"), Span::new(0, 0)))?;
    let bool_to_string = module
        .declare_function("corvid_bool_to_string", Linkage::Import, &int_text_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare bool_to_string: {e}"), Span::new(0, 0)))?;
    let mut float_text_sig = module.make_signature();
    float_text_sig.params.push(AbiParam::new(F64));
    float_text_sig.returns.push(AbiParam::new(I64));
    let float_to_string = module
        .declare_function("corvid_float_to_string", Linkage::Import, &float_text_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare float_to_string: {e}"), Span::new(0, 0)))?;

    // Native dispatch N4 — `response.header(name)` as an Option<String>.
    let mut response_header_sig = module.make_signature();
    response_header_sig.params.push(AbiParam::new(I64));
    response_header_sig.params.push(AbiParam::new(I64));
    response_header_sig.returns.push(AbiParam::new(I64));
    let response_header = module
        .declare_function("corvid_response_header", Linkage::Import, &response_header_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare response_header: {e}"), Span::new(0, 0)))?;

    // Native dispatch N3 — `a == b` on two secrets, in constant time.
    let mut secret_eq_sig = module.make_signature();
    secret_eq_sig.params.push(AbiParam::new(I64));
    secret_eq_sig.params.push(AbiParam::new(I64));
    secret_eq_sig.returns.push(AbiParam::new(I64));
    let secret_eq = module
        .declare_function("corvid_secret_eq", Linkage::Import, &secret_eq_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare secret_eq: {e}"), Span::new(0, 0)))?;

    let runtime_shutdown_sig = module.make_signature();
    let runtime_shutdown_id = module
        .declare_function(
            RUNTIME_SHUTDOWN_SYMBOL,
            Linkage::Import,
            &runtime_shutdown_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare runtime_shutdown: {e}"), Span::new(0, 0))
        })?;

    let embed_init_sig = module.make_signature();
    let embed_init_id = module
        .declare_function(RUNTIME_EMBED_INIT_SYMBOL, Linkage::Import, &embed_init_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare runtime_embed_init: {e}"), Span::new(0, 0))
        })?;

    let mut string_into_cstr_sig = module.make_signature();
    string_into_cstr_sig.params.push(AbiParam::new(I64));
    string_into_cstr_sig.returns.push(AbiParam::new(I64));
    let string_into_cstr_id = module
        .declare_function(
            STRING_INTO_CSTR_SYMBOL,
            Linkage::Import,
            &string_into_cstr_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_into_cstr: {e}"), Span::new(0, 0))
        })?;

    let mut begin_direct_observation_sig = module.make_signature();
    begin_direct_observation_sig.params.push(AbiParam::new(F64));
    let begin_direct_observation_id = module
        .declare_function(
            BEGIN_DIRECT_OBSERVATION_SYMBOL,
            Linkage::Import,
            &begin_direct_observation_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare begin_direct_observation: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut finish_direct_observation_sig = module.make_signature();
    finish_direct_observation_sig
        .params
        .push(AbiParam::new(I64));
    let finish_direct_observation_id = module
        .declare_function(
            FINISH_DIRECT_OBSERVATION_SYMBOL,
            Linkage::Import,
            &finish_direct_observation_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare finish_direct_observation: {e}"),
                Span::new(0, 0),
            )
        })?;

    let grounded_capture_scalar_sig = module.make_signature();
    let grounded_capture_scalar_handle_id = module
        .declare_function(
            GROUNDED_CAPTURE_SCALAR_HANDLE_SYMBOL,
            Linkage::Import,
            &grounded_capture_scalar_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_capture_scalar_handle: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut grounded_capture_string_sig = module.make_signature();
    grounded_capture_string_sig.params.push(AbiParam::new(I64));
    grounded_capture_string_sig.returns.push(AbiParam::new(I64));
    let grounded_capture_string_handle_id = module
        .declare_function(
            GROUNDED_CAPTURE_STRING_HANDLE_SYMBOL,
            Linkage::Import,
            &grounded_capture_string_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_capture_string_handle: {e}"),
                Span::new(0, 0),
            )
        })?;

    let make_grounded_attest_sig =
        |module: &mut ObjectModule, value_ty: cranelift_codegen::ir::Type| {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(value_ty));
            sig.params.push(AbiParam::new(I64));
            sig.params.push(AbiParam::new(F64));
            sig.returns.push(AbiParam::new(value_ty));
            sig
        };
    let grounded_attest_int_sig = make_grounded_attest_sig(module, I64);
    let grounded_attest_int_id = module
        .declare_function(
            GROUNDED_ATTEST_INT_SYMBOL,
            Linkage::Import,
            &grounded_attest_int_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare grounded_attest_int: {e}"), Span::new(0, 0))
        })?;
    let grounded_attest_bool_sig = make_grounded_attest_sig(module, I8);
    let grounded_attest_bool_id = module
        .declare_function(
            GROUNDED_ATTEST_BOOL_SYMBOL,
            Linkage::Import,
            &grounded_attest_bool_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_attest_bool: {e}"),
                Span::new(0, 0),
            )
        })?;
    let grounded_attest_float_sig = make_grounded_attest_sig(module, F64);
    let grounded_attest_float_id = module
        .declare_function(
            GROUNDED_ATTEST_FLOAT_SYMBOL,
            Linkage::Import,
            &grounded_attest_float_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_attest_float: {e}"),
                Span::new(0, 0),
            )
        })?;
    let grounded_attest_string_sig = make_grounded_attest_sig(module, I64);
    let grounded_attest_string_id = module
        .declare_function(
            GROUNDED_ATTEST_STRING_SYMBOL,
            Linkage::Import,
            &grounded_attest_string_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_attest_string: {e}"),
                Span::new(0, 0),
            )
        })?;

    // `corvid_grounded_attest_struct(value: i64, source: CorvidString,
    // confidence: f64) -> i64` — same shape as the string variant.
    let grounded_attest_struct_sig = make_grounded_attest_sig(module, I64);
    let grounded_attest_struct_id = module
        .declare_function(
            GROUNDED_ATTEST_STRUCT_SYMBOL,
            Linkage::Import,
            &grounded_attest_struct_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare grounded_attest_struct: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut sleep_ms_sig = module.make_signature();
    sleep_ms_sig.params.push(AbiParam::new(I64));
    let sleep_ms_id = module
        .declare_function(SLEEP_MS_SYMBOL, Linkage::Import, &sleep_ms_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare sleep_ms: {e}"), Span::new(0, 0)))?;

    // Stringification helpers. Each takes a typed scalar
    // and returns a Corvid String descriptor pointer (i64).
    let mut sfi_sig = module.make_signature();
    sfi_sig.params.push(AbiParam::new(I64));
    sfi_sig.returns.push(AbiParam::new(I64));
    // A `String` prompt argument renders as its JSON form, quoted and
    // escaped, as the interpreter renders it.
    let string_from_int_id = module
        .declare_function(STRING_FROM_INT_SYMBOL, Linkage::Import, &sfi_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_from_int: {e}"), Span::new(0, 0))
        })?;

    let mut sfb_sig = module.make_signature();
    sfb_sig.params.push(AbiParam::new(I8));
    sfb_sig.returns.push(AbiParam::new(I64));
    let string_from_bool_id = module
        .declare_function(STRING_FROM_BOOL_SYMBOL, Linkage::Import, &sfb_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_from_bool: {e}"), Span::new(0, 0))
        })?;

    let mut sff_sig = module.make_signature();
    sff_sig.params.push(AbiParam::new(F64));
    sff_sig.returns.push(AbiParam::new(I64));
    let string_from_float_id = module
        .declare_function(STRING_FROM_FLOAT_SYMBOL, Linkage::Import, &sff_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare string_from_float: {e}"), Span::new(0, 0))
        })?;

    let mut approve_sig = module.make_signature();
    approve_sig.params.push(AbiParam::new(I64));
    approve_sig.params.push(AbiParam::new(I64));
    approve_sig.params.push(AbiParam::new(I64));
    approve_sig.params.push(AbiParam::new(I64));
    approve_sig.returns.push(AbiParam::new(I8));
    let approve_sync_id = module
        .declare_function(APPROVE_SYNC_SYMBOL, Linkage::Import, &approve_sig)
        .map_err(|e| {
            CodegenError::cranelift(format!("declare approve_sync: {e}"), Span::new(0, 0))
        })?;

    // The prompt bridge every native prompt call uses (B1b).
    let mut prompt_typed_sig = module.make_signature();
    for _ in 0..12 {
        prompt_typed_sig.params.push(AbiParam::new(I64));
    }
    prompt_typed_sig.returns.push(AbiParam::new(I64));
    let prompt_call_typed_id = module
        .declare_function(PROMPT_CALL_TYPED_SYMBOL, Linkage::Import, &prompt_typed_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare prompt_call_typed: {e}"), Span::new(0, 0)))?;


    let mut citation_verify_sig = module.make_signature();
    citation_verify_sig.params.push(AbiParam::new(I64));
    citation_verify_sig.params.push(AbiParam::new(I64));
    citation_verify_sig.params.push(AbiParam::new(I64));
    citation_verify_sig.returns.push(AbiParam::new(I8));
    let citation_verify_or_panic_id = module
        .declare_function(
            CITATION_VERIFY_OR_PANIC_SYMBOL,
            Linkage::Import,
            &citation_verify_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(
                format!("declare citation_verify_or_panic: {e}"),
                Span::new(0, 0),
            )
        })?;

    let mut trace_run_started_sig = module.make_signature();
    trace_run_started_sig.params.push(AbiParam::new(I64));
    trace_run_started_sig.params.push(AbiParam::new(I64));
    trace_run_started_sig.params.push(AbiParam::new(I64));
    trace_run_started_sig.params.push(AbiParam::new(I64));
    let trace_run_started_id = module
        .declare_function(
            TRACE_RUN_STARTED_SYMBOL,
            Linkage::Import,
            &trace_run_started_sig,
        )
        .map_err(|e| {
            CodegenError::cranelift(format!("declare trace_run_started: {e}"), Span::new(0, 0))
        })?;

    // Native outage unwinding (recoverable failures TE-3): the pending
    // slot's test, clear and stop, and the switch that turns holding on.
    let outage_fn = |module: &mut ObjectModule, symbol: &str, returns: bool| {
        let mut sig = module.make_signature();
        if returns {
            sig.returns.push(AbiParam::new(I64));
        }
        module
            .declare_function(symbol, Linkage::Import, &sig)
            .map_err(|e| CodegenError::cranelift(format!("declare {symbol}: {e}"), Span::new(0, 0)))
    };
    let outage_pending_id = outage_fn(module, "corvid_outage_pending", true)?;
    let outage_clear_id = outage_fn(module, "corvid_outage_clear", false)?;
    let outage_stop_if_pending_id = outage_fn(module, "corvid_outage_stop_if_pending", false)?;
    let outage_catching_id = outage_fn(module, "corvid_outage_catching", false)?;

    // A run's result as its public JSON text (one mapping for every type).
    let mut trace_run_completed_sig = module.make_signature();
    trace_run_completed_sig.params.push(AbiParam::new(I64));
    let trace_run_completed_id = module
        .declare_function(TRACE_RUN_COMPLETED_JSON_SYMBOL, Linkage::Import, &trace_run_completed_sig)
        .map_err(|e| CodegenError::cranelift(format!("declare trace_run_completed_json: {e}"), Span::new(0, 0)))?;

    Ok(RuntimeFuncs {
        overflow: overflow_func_id,
        retain: retain_id,
        release: release_id,
        structural_eq, equality_layouts: Default::default(), json_decode_plans: Default::default(),
        list_map_int, list_filter_int, list_fold_int, list_test_int,
        list_snapshot, list_concat, list_new, list_append, list_bounds_error, return_temporaries: Default::default(),
        map_new, map_set, map_find, map_require_int, map_snapshot,
        string_concat: concat_id,
        grounded_string_concat,
        string_eq: eq_id,
        string_cmp: cmp_id,
        string_to_upper,
        string_to_lower,
        string_substring,
        string_trim,
        string_contains,
        string_starts_with,
        string_ends_with,
        string_replace,
        string_split,
        string_parse_int: string_parse_int_id,
        text_methods,
        decimal_operations,
        time_operations,
        currency_operations,
        bytes_operations,
        json_decode_typed: json_decode_typed_id,
        runtime_call: runtime_call_id,
        json_encode_typed: json_encode_typed_id,
        json_encode_tool_argument,
        json_encode_recorded,
        string_char_len: string_char_len_id,
        string_char_at: string_char_at_id,
        alloc_typed: alloc_typed_id,
        list_destroy: list_destroy_id,
        list_trace: list_trace_id,
        weak_new: weak_new_id,
        weak_upgrade: weak_upgrade_id,
        weak_clear_self: weak_clear_self_id,
        string_typeinfo: string_typeinfo_id,
        secret_typeinfo,
        map_typeinfo,
        weak_box_typeinfo: weak_box_typeinfo_id,
        entry_init: entry_init_id,
        entry_arity_mismatch: arity_id,
        parse_i64: parse_i64_id,
        parse_f64: parse_f64_id,
        parse_bool: parse_bool_id,
        string_from_cstr: from_cstr_id,
        print_i64: print_i64_id,
        print_bool: print_bool_id,
        print_f64: print_f64_id,
        print_string: print_string_id,
        bench_server_enabled: bench_server_enabled_id,
        bench_next_trial: bench_next_trial_id,
        bench_finish_trial: bench_finish_trial_id,
        runtime_init: runtime_init_id,
        register_connectors: register_connectors_id,
        secret_eq,
        int_to_string,
        float_to_string,
        bool_to_string,
        response_header,
        runtime_shutdown: runtime_shutdown_id,
        runtime_embed_init: embed_init_id,
        sleep_ms: sleep_ms_id,
        string_into_cstr: string_into_cstr_id,
        begin_direct_observation: begin_direct_observation_id,
        finish_direct_observation: finish_direct_observation_id,
        grounded_capture_scalar_handle: grounded_capture_scalar_handle_id,
        grounded_capture_string_handle: grounded_capture_string_handle_id,
        grounded_attest_int: grounded_attest_int_id,
        grounded_attest_bool: grounded_attest_bool_id,
        grounded_attest_float: grounded_attest_float_id,
        grounded_attest_string: grounded_attest_string_id,
        grounded_attest_struct: grounded_attest_struct_id,
        string_from_int: string_from_int_id,
        string_from_bool: string_from_bool_id,
        string_from_float: string_from_float_id,
        approve_sync: approve_sync_id,
        prompt_call_typed: prompt_call_typed_id,
        citation_verify_or_panic: citation_verify_or_panic_id,
        trace_run_started: trace_run_started_id,
        trace_run_completed: trace_run_completed_id,
        outage_pending: outage_pending_id,
        outage_clear: outage_clear_id,
        outage_stop_if_pending: outage_stop_if_pending_id,
        outage_catching: outage_catching_id,
        outage_plan: Default::default(),
        outage_checked: Default::default(),
        literal_counter: std::cell::Cell::new(0),
        // The unified ownership pass is the default. It produces
        // refcount-correct code
        // across all 106 parity fixtures + 9 verifier-audit classes,
        // with systematically lower RC op counts than the peephole-
        // optimized predecessor.
        //
        // Set `CORVID_DUP_DROP_PASS=0` to fall back to the legacy
        // scattered-emit codegen for A/B comparison. That fallback
        // path will be removed once the unified pass has fully
        // stabilized in production.
        in_match_operand: Default::default(),
        next_variable: Default::default(),
        dup_drop_enabled: crate::dup_drop_pass_enabled(),
        // Default to the native (link-time wrapper) dispatch; `lower_file`
        // sets this true for library targets (cdylib/staticlib).
        tools_via_registry: false,
        struct_destructors: HashMap::new(),
        struct_traces: HashMap::new(),
        callback_typeinfo: emit_callback_typeinfo(module)?,
        variant_typeinfos: HashMap::new(),
        struct_typeinfos: HashMap::new(),
        list_typeinfos: HashMap::new(),
        result_destructors: HashMap::new(),
        result_traces: HashMap::new(),
        result_typeinfos: HashMap::new(),
        option_typeinfos: HashMap::new(),
        ir_types: HashMap::new(),
        runtime_call_ops: std::collections::HashSet::new(),
        ir_tools: HashMap::new(),
        tool_wrapper_ids: std::cell::RefCell::new(HashMap::new()),
        ir_prompts: HashMap::new(),
        prompt_pins: HashMap::new(),
        agent_borrow_sigs: HashMap::new(),
        stack_maps: std::cell::RefCell::new(HashMap::new()),
    })
}
