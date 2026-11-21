//! Decides whether an IR file can run through the native AOT tier.
//!
//! The native path currently supports prompt calls and conditionally
//! supports tool calls when the caller supplies a companion tools
//! staticlib via `--with-tools-lib`. The scan produces a structured
//! reason so the CLI can explain why a program falls back to the
//! interpreter.
//!
//! Rationale for a pre-flight IR scan (vs. "try compile, catch
//! NotSupported"): (a) names the native-ability rule explicitly so it's
//! testable and documentable; (b) yields a driver-level error message
//! rather than a codegen-internal one; (c) cheap - O(IR nodes) walk
//! with early exit.

use corvid_ir::{IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrImportSource, IrStmt};
use corvid_types::Type;

/// Why a program can't run via the native tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotNativeReason {
    PythonImport {
        module: String,
    },
    /// User-declared tool called from compiled code. This is supported
    /// via typed-ABI direct calls, but only when the caller
    /// supplies a tools staticlib (`--with-tools-lib`). Without one,
    /// the scan reports this reason and the dispatcher falls back.
    ToolCall {
        name: String,
    },
    /// Wider tagged unions and retry bodies outside the supported
    /// native `Result` / `Option` subset still route to the
    /// interpreter. Nullable-pointer `Option<T>` with a
    /// refcounted payload, wide scalar `Option<Int|Bool|Float>`, and
    /// the compositional native `Result<T, E>` subset are the
    /// supported native forms today.
    TaggedUnionRetryNotNative,
    StreamLoweringNotImplemented,
    TestFixtureNotNative {
        name: String,
    },
    /// `replay <trace>: when ... else ...` expressions need the
    /// trace-dispatch runtime primitive (Phase 21 slice
    /// 21-inv-E-runtime) plus its native-tier lowering follow-up.
    /// Until both land, any program containing a replay expression
    /// routes to the interpreter tier.
    ReplayPrimitiveNotNative,
    HumanBoundaryNotNative,
    /// Place assignment (`x.field = v`, `xs[i] = v`, compound `op=`)
    /// is interpreter-only in slice 45b; native lowering is filed
    /// with the 47c backend-parity work.
    PlaceAssignmentNotNative,
    UnsupportedEquality,
    CallbackNotNative,
    PromptToolsRequireInterpreter,
    UnsupportedMap,
    UnsupportedMatch,
    /// Builtin methods on built-in receiver types (`String.length()`
    /// and the 45d/45e/45f batches) are interpreter-only in 45c.
    BuiltinMethodNotNative,
}

impl std::fmt::Display for NotNativeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PythonImport { module } => write!(
                f,
                "program imports Python module `{module}` - native Python FFI is not implemented yet"
            ),
            Self::ToolCall { name } => write!(
                f,
                "program calls tool `{name}` - pass `--with-tools-lib <path>` pointing at your compiled `#[tool]` staticlib, or let auto-dispatch fall back to the interpreter"
            ),
            Self::TaggedUnionRetryNotNative => write!(
                f,
                "program uses a tagged-union or retry shape outside the current native subset - native AOT supports nullable-pointer `Option<T>`, wide scalar `Option<Int|Bool|Float>`, compositional native `Result<T, E>`, postfix `?`, and `try ... retry` over native `Result<T, E>` and `Option<T>` bodies; wider shapes still run in the interpreter"
            ),
            Self::StreamLoweringNotImplemented => {
                write!(f, "program uses `Stream<T>` - Stream lowering is not yet implemented")
            }
            Self::TestFixtureNotNative { name } => write!(
                f,
                "program calls test fixture `{name}` - fixtures are interpreter-only test declarations and are not part of the native production tier"
            ),
            Self::ReplayPrimitiveNotNative => {
                write!(f, "program uses `replay <trace>: when ... else ...` - native lowering of the replay language primitive lands in a follow-up to Phase 21 slice 21-inv-E-runtime; until then the interpreter tier runs it")
            }
            Self::HumanBoundaryNotNative => {
                write!(f, "program uses `ask` or `choose` - human input boundaries are interpreter-only in this slice")
            }
            Self::BuiltinMethodNotNative => {
                write!(f, "program uses a builtin method (e.g. `String.length()`) - interpreter-only in 45c; the auto-dispatcher runs the interpreter tier")
            }
            Self::UnsupportedMap => f.write_str(corvid_codegen_cl::NATIVE_MAP_REQUIREMENTS),
            Self::UnsupportedMatch => write!(f, "this match pattern or arm requires the interpreter"),
            Self::UnsupportedEquality => write!(f, "comparison of these value types requires the interpreter"),
            Self::PromptToolsRequireInterpreter => write!(f, "prompt function arguments require the interpreter"),
            Self::CallbackNotNative => write!(f, "this callback signature or body requires the interpreter"),
            Self::PlaceAssignmentNotNative => {
                write!(f, "program uses place assignment (`x.field = v` / `xs[i] = v` / compound `op=`) - interpreter-only in 45b; the auto-dispatcher runs the interpreter tier")
            }
        }
    }
}

fn is_refcounted_type(ty: &Type) -> bool {
    match ty {
        Type::String
        | Type::Struct(_)
        | Type::ImportedStruct(_)
        | Type::List(_)
        | Type::Weak(_, _)
        | Type::Result(_, _)
        | Type::Partial(_)
        | Type::ResumeToken(_) => true,
        Type::Map(_, _) => corvid_codegen_cl::native_string_map(ty),
        Type::Option(inner) => is_native_wide_option_type(ty) || is_refcounted_type(inner),
        _ => false,
    }
}

fn is_native_value_type(ty: &Type) -> bool {
    match ty {
        Type::Map(_, _) => corvid_codegen_cl::native_string_map(ty),
        Type::Int | Type::Bool | Type::Float | Type::String => true,
        Type::Struct(_) | Type::ImportedStruct(_) | Type::List(_) | Type::Weak(_, _) => true,
        Type::Option(_) => is_native_option_type(ty),
        Type::Result(ok, err) => is_native_value_type(ok) && is_native_value_type(err),
        Type::Grounded(inner) => is_native_value_type(inner),
        Type::Tainted(inner) => is_native_value_type(inner),
        // TraceId requires replay-runtime support on the native
        // tier (Phase 21 slice 21-inv-E-4 + E-runtime). Until then,
        // a program using replay routes to the interpreter tier.
        Type::TraceId => false,
        // Phase 33S3a — `DbHandle` is interpreter-tier only; the
        // driver's tier-picker checks this function and routes any
        // program mentioning `DbHandle` (including transitively
        // through agent signatures) to the interpreter so the user
        // never sees a confusing native-codegen error mid-build.
        Type::DbHandle => false,
        // Phase 33R5b-a — same rationale for JsonValue and
        // JsonBuilder. Interpreter-tier only until the cdylib
        // bridging slice ships.
        Type::JsonValue | Type::JsonBuilder => false,
        // Slice 51f — `Upload` / `Page` are HTTP-boundary types served
        // by `corvid serve`; the tier-picker routes any program
        // mentioning them to the interpreter tier, mirroring `DbHandle`.
        Type::Upload(_) | Type::Page(_) => false,
        Type::Function { .. } => corvid_codegen_cl::native_callback_type(ty),
        Type::Nothing
        | Type::Stream(_)
        | Type::Partial(_)
        | Type::ResumeToken(_)
        | Type::RouteParams(_)
        | Type::Unknown => false,
    }
}

fn is_native_wide_option_type(ty: &Type) -> bool {
    matches!(ty, Type::Option(inner) if matches!(&**inner, Type::Int | Type::Bool | Type::Float))
}

fn is_native_option_type(ty: &Type) -> bool {
    match ty {
        Type::Option(inner) => is_refcounted_type(inner) || is_native_wide_option_type(ty),
        _ => false,
    }
}

fn is_native_option_expr_type(ty: &Type) -> bool {
    matches!(ty, Type::Option(inner) if matches!(**inner, Type::Unknown))
        || is_native_option_type(ty)
}

fn is_native_result_type(ty: &Type) -> bool {
    matches!(ty, Type::Result(ok, err) if is_native_value_type(ok) && is_native_value_type(err))
}

/// Walk the IR and return `Ok(())` if every construct is native-able,
/// else the first reason found (early exit - one reason is enough to
/// route the caller to the interpreter tier).
pub fn native_ability(ir: &IrFile) -> Result<(), NotNativeReason> {
    if let Some((module, _)) = ir.tools.iter().find_map(|tool| tool.python.as_ref()) {
        return Err(NotNativeReason::PythonImport { module: module.clone() });
    }
    for import in &ir.imports {
        match import.source {
            IrImportSource::Python => {
                return Err(NotNativeReason::PythonImport {
                    module: import.module.clone(),
                });
            }
            IrImportSource::Corvid
            | IrImportSource::RemoteCorvid
            | IrImportSource::PackageCorvid => {}
        }
    }
    for agent in &ir.agents {
        if agent.params.iter().map(|p| &p.ty).chain(std::iter::once(&agent.return_ty))
            .any(|ty| matches!(ty, Type::Map(_, _)) && !corvid_codegen_cl::native_string_map(ty)) {
            return Err(NotNativeReason::UnsupportedMap);
        }
        if matches!(agent.return_ty, Type::Stream(_)) {
            return Err(NotNativeReason::StreamLoweringNotImplemented);
        }
        scan_block(&agent.body, &agent.return_ty, &ir.types)?;
    }
    Ok(())
}

fn scan_block(block: &IrBlock, current_return_ty: &Type, types: &[corvid_ir::IrType]) -> Result<(), NotNativeReason> {
    for stmt in &block.stmts {
        scan_stmt(stmt, current_return_ty, types)?;
    }
    Ok(())
}

fn scan_stmt(stmt: &IrStmt, current_return_ty: &Type, types: &[corvid_ir::IrType]) -> Result<(), NotNativeReason> {
    if let Some((_, keys, value, _)) = corvid_codegen_cl::native_indexed_assignment(stmt) {
        for key in keys { scan_expr(key, current_return_ty, types)?; }
        return scan_expr(value, current_return_ty, types);
    }
    match stmt {
        IrStmt::Let { value, .. } => scan_expr(value, current_return_ty, types),
        IrStmt::Assign { value, .. } if corvid_codegen_cl::native_record_assignment(stmt) || corvid_codegen_cl::native_local_compound(stmt) => scan_expr(value, current_return_ty, types),
        IrStmt::Assign { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        IrStmt::Yield { .. } => Err(NotNativeReason::StreamLoweringNotImplemented),
        IrStmt::Return { value: Some(v), .. } => scan_expr(v, current_return_ty, types),
        IrStmt::Return { value: None, .. } => Ok(()),
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            scan_expr(cond, current_return_ty, types)?;
            scan_block(then_block, current_return_ty, types)?;
            if let Some(b) = else_block {
                scan_block(b, current_return_ty, types)?;
            }
            Ok(())
        }
        IrStmt::For { iter, body, .. } => {
            scan_expr(iter, current_return_ty, types)?;
            scan_block(body, current_return_ty, types)
        }
        // Destructuring is interpreter-only in 45n.
        IrStmt::Destructure { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        // Parallel blocks are interpreter-only in 46e.
        IrStmt::Parallel { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        // `while` (45k) lowers natively like `for` — walk cond + body.
        IrStmt::While { cond, body, .. } => {
            scan_expr(cond, current_return_ty, types)?;
            scan_block(body, current_return_ty, types)
        }
        IrStmt::Approve { args, .. } => {
            // `approve` compiles to a no-op in generated native code.
            // Still walk the arg expressions so any tool/prompt call
            // buried in an approve arg is reported.
            for a in args {
                scan_expr(a, current_return_ty, types)?;
            }
            Ok(())
        }
        IrStmt::Expr { expr, .. } => scan_expr(expr, current_return_ty, types),
        IrStmt::Break { .. } | IrStmt::Continue { .. } | IrStmt::Pass { .. } => Ok(()),
        // Ownership ops contain no user expressions; they don't change
        // whether this agent can run natively.
        IrStmt::Dup { .. } | IrStmt::Drop { .. } => Ok(()),
    }
}

fn scan_expr(expr: &IrExpr, current_return_ty: &Type, types: &[corvid_ir::IrType]) -> Result<(), NotNativeReason> {
    if let Some((receiver, key)) = corvid_codegen_cl::native_string_map_lookup(expr) {
        scan_expr(receiver, current_return_ty, types)?;
        return scan_expr(key, current_return_ty, types);
    }
    if matches!(expr.ty, Type::Map(_, _)) && !corvid_codegen_cl::native_string_map(&expr.ty) {
        return Err(NotNativeReason::UnsupportedMap);
    }
    if matches!(expr.ty, Type::Stream(_)) {
        return Err(NotNativeReason::StreamLoweringNotImplemented);
    }
    match &expr.kind {
        // Map is interpreter-only in 45g; a MapLiteral anywhere routes
        // the program to the interpreter tier.
        IrExprKind::MapLiteral { keys, values } if corvid_codegen_cl::native_string_map(&expr.ty) => {
            keys.iter().chain(values).try_for_each(|item| scan_expr(item, current_return_ty, types))
        }
        IrExprKind::MapLiteral { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        IrExprKind::Match { scrutinee, arms }
            if (is_native_option_type(&scrutinee.ty) || is_native_result_type(&scrutinee.ty) || matches!(scrutinee.ty, Type::Struct(_) | Type::ImportedStruct(_)))
                && corvid_codegen_cl::result_match::supports_match(&scrutinee.ty, arms) =>
        {
            scan_expr(scrutinee, current_return_ty, types)?;
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    scan_expr(guard, current_return_ty, types)?;
                }
                scan_expr(&arm.body, current_return_ty, types)?;
            }
            Ok(())
        }
        IrExprKind::Match { .. } => Err(NotNativeReason::UnsupportedMatch),
        // Lambdas are interpreter-only in 45j.
        IrExprKind::Lambda { body, .. } if corvid_codegen_cl::native_callback_type(&expr.ty)
            && corvid_codegen_cl::native_lambda_locals(body).is_some() => Ok(()),
        IrExprKind::Lambda { .. } => Err(NotNativeReason::CallbackNotNative),
        // Named struct literals are interpreter-only in 45n.
        IrExprKind::StructLiteral { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        // `Page<Item>` responses are interpreter-only in 52c-2.
        IrExprKind::PageNew { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        IrExprKind::BuiltinMethod {
            kind: corvid_types::BuiltinMethodKind::ListFirst | corvid_types::BuiltinMethodKind::ListLast,
            receiver,
            ..
        } if is_native_option_type(&expr.ty) => scan_expr(receiver, current_return_ty, types),
        IrExprKind::BuiltinMethod {
            kind: corvid_types::BuiltinMethodKind::ListLength, receiver, ..
        } => scan_expr(receiver, current_return_ty, types),
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if corvid_codegen_cl::native_string_map(&receiver.ty) && matches!(kind,
                corvid_types::BuiltinMethodKind::MapKeys | corvid_types::BuiltinMethodKind::MapValues |
                corvid_types::BuiltinMethodKind::MapLength | corvid_types::BuiltinMethodKind::MapContainsKey) => {
            scan_expr(receiver, current_return_ty, types)?;
            args.iter().try_for_each(|arg| scan_expr(arg, current_return_ty, types))
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::ListAppend, receiver, args }
            if args.len() == 1 && corvid_codegen_cl::native_list_append(&receiver.ty, &args[0].ty) => {
            scan_expr(receiver, current_return_ty, types)?;
            scan_expr(&args[0], current_return_ty, types)
        }
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if corvid_codegen_cl::native_integer_list_transform(*kind, &receiver.ty, args) || corvid_codegen_cl::native_numeric_conversion(*kind)
                || corvid_codegen_cl::native_string_method(*kind)
                || matches!(kind, corvid_types::BuiltinMethodKind::StringParseInt | corvid_types::BuiltinMethodKind::ResultIsOk | corvid_types::BuiltinMethodKind::ResultIsErr) => {
            scan_expr(receiver, current_return_ty, types)?;
            args.iter().try_for_each(|arg| scan_expr(arg, current_return_ty, types))
        }
        IrExprKind::BuiltinMethod { .. } => Err(NotNativeReason::BuiltinMethodNotNative),
        IrExprKind::Literal(_) | IrExprKind::Local { .. } | IrExprKind::Decl { .. } => Ok(()),
        IrExprKind::Call {
            kind,
            callee_name,
            args,
        } => {
            match kind {
                IrCallKind::EnumConstructor { .. } => {}
                IrCallKind::ClosureLocal { signature, .. } if corvid_codegen_cl::native_callback_type(signature) => {}
                IrCallKind::ClosureLocal { .. } => {
                    return Err(NotNativeReason::CallbackNotNative)
                }
                IrCallKind::Tool { .. } => {
                    return Err(NotNativeReason::ToolCall {
                        name: callee_name.clone(),
                    })
                }
                IrCallKind::Prompt { .. } if args.iter().any(|arg| matches!(arg.ty, Type::Function { .. })) => {
                    return Err(NotNativeReason::PromptToolsRequireInterpreter);
                }
                IrCallKind::Prompt { .. } => {
                    // Prompt calls compile and run natively. No extra
                    // user-provided lib is needed because corvid-runtime
                    // ships the LLM adapters built in. Runtime errors
                    // surface if no provider is configured.
                }
                IrCallKind::Agent { .. }
                | IrCallKind::Fixture { .. }
                | IrCallKind::StructConstructor { .. }
                | IrCallKind::Unknown => {}
            }
            if matches!(kind, IrCallKind::Fixture { .. }) {
                return Err(NotNativeReason::TestFixtureNotNative {
                    name: callee_name.clone(),
                });
            }
            for a in args {
                scan_expr(a, current_return_ty, types)?;
            }
            Ok(())
        }
        IrExprKind::FieldAccess { target, .. } => scan_expr(target, current_return_ty, types),
        IrExprKind::UnwrapGrounded { value } => scan_expr(value, current_return_ty, types),
        IrExprKind::Index { target, index } => {
            scan_expr(target, current_return_ty, types)?;
            scan_expr(index, current_return_ty, types)
        }
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right } => {
            if corvid_codegen_cl::lowering::uses_structural_equality(*op, &left.ty)
                && !corvid_codegen_cl::lowering::supports_structural_comparison(*op, left, right, types) {
                return Err(NotNativeReason::UnsupportedEquality);
            }
            scan_expr(left, current_return_ty, types)?;
            scan_expr(right, current_return_ty, types)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            scan_expr(operand, current_return_ty, types)
        }
        IrExprKind::List { items } => {
            for it in items {
                scan_expr(it, current_return_ty, types)?;
            }
            Ok(())
        }
        IrExprKind::WeakNew { strong } => scan_expr(strong, current_return_ty, types),
        IrExprKind::WeakUpgrade { weak } => scan_expr(weak, current_return_ty, types),
        IrExprKind::StreamSplitBy { stream, .. } => {
            scan_expr(stream, current_return_ty, types)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamMerge { groups, .. } => {
            scan_expr(groups, current_return_ty, types)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamOrderedBy { stream, .. } => {
            scan_expr(stream, current_return_ty, types)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamResumeToken { stream } => {
            scan_expr(stream, current_return_ty, types)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::ResumeStream { token, .. } => {
            scan_expr(token, current_return_ty, types)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        // Tagged-union/retry nodes are accepted only for the current
        // native subset. Recurse into sub-expressions first so any
        // nested tool/prompt calls still get reported correctly.
        IrExprKind::OptionSome { inner } => {
            scan_expr(inner, current_return_ty, types)?;
            if is_native_option_expr_type(&expr.ty) {
                Ok(())
            } else {
                Err(NotNativeReason::TaggedUnionRetryNotNative)
            }
        }
        IrExprKind::ResultOk { inner } | IrExprKind::ResultErr { inner } => {
            scan_expr(inner, current_return_ty, types)?;
            if is_native_result_type(&expr.ty) {
                Ok(())
            } else {
                Err(NotNativeReason::TaggedUnionRetryNotNative)
            }
        }
        IrExprKind::OptionNone => {
            if is_native_option_expr_type(&expr.ty) {
                Ok(())
            } else {
                Err(NotNativeReason::TaggedUnionRetryNotNative)
            }
        }
        IrExprKind::TryPropagate { inner } => {
            scan_expr(inner, current_return_ty, types)?;
            match &inner.ty {
                Type::Option(_) => {
                    if is_native_option_expr_type(&inner.ty)
                        && is_native_option_expr_type(current_return_ty)
                    {
                        Ok(())
                    } else {
                        Err(NotNativeReason::TaggedUnionRetryNotNative)
                    }
                }
                Type::Result(_, _) => {
                    if is_native_result_type(&inner.ty) {
                        if let Type::Result(_, outer_err) = current_return_ty {
                            if is_native_result_type(current_return_ty) {
                                if let Type::Result(_, inner_err) = &inner.ty {
                                    if &**outer_err == &**inner_err {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                    }
                    Err(NotNativeReason::TaggedUnionRetryNotNative)
                }
                _ => Err(NotNativeReason::TaggedUnionRetryNotNative),
            }
        }
        IrExprKind::TrustBoundary { inner } => scan_expr(inner, current_return_ty, types),
        IrExprKind::TryRetry { body, .. } => {
            scan_expr(body, current_return_ty, types)?;
            if &body.ty == &expr.ty
                && (is_native_result_type(&body.ty) || is_native_option_expr_type(&body.ty))
            {
                Ok(())
            } else {
                Err(NotNativeReason::TaggedUnionRetryNotNative)
            }
        }
        IrExprKind::Replay { .. } => Err(NotNativeReason::ReplayPrimitiveNotNative),
        IrExprKind::Ask { .. } | IrExprKind::Choose { .. } => {
            Err(NotNativeReason::HumanBoundaryNotNative)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{native_ability, NotNativeReason};
    use corvid_ast::Span;
    use corvid_ir::{IrAgent, IrBlock, IrFile};
    use corvid_resolve::DefId;
    use corvid_types::Type;

    #[test]
    fn stream_return_type_is_not_native() {
        let span = Span::new(0, 0);
        let ir = IrFile {
            import_fingerprint: None,
            imports: vec![],
            types: vec![],
            tools: vec![],
            prompts: vec![],
            agents: vec![IrAgent {
                id: DefId(0),
                name: "streamer".into(),
                extern_abi: None,
                params: vec![],
                return_ty: Type::Stream(Box::new(Type::String)),
                cost_budget: None,
                wrapping_arithmetic: false,
                is_replayable: false,
                pure_fn: false,
                retry_max_attempts: None,
                retry_backoff_ms: None,
                idempotency_key_param: None,
                body: IrBlock {
                    stmts: vec![],
                    span,
                },
                span,
                borrow_sig: Some(vec![]),
            }],
            evals: vec![],
            tests: vec![],
            fixtures: vec![],
            mocks: vec![],
            servers: vec![],
            models: Vec::new(),
            connectors: vec![],
        };

        assert!(matches!(
            native_ability(&ir),
            Err(NotNativeReason::StreamLoweringNotImplemented)
        ));
    }
}
