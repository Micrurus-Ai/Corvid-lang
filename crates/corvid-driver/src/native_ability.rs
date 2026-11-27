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

use corvid_codegen_cl::NativeTypeQuestion;
use corvid_ir::{IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrImportSource, IrStmt};
use corvid_types::Type;

/// Why a program can't run via the native tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotNativeReason {
    /// A table operation (B09). Native tables are slice P6.
    TableOperation {
        table: String,
    },
    /// An `atomic:` block (B09 P7a), which groups table writes.
    AtomicBlock,
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
    /// `--mode mock` on a program with connectors: a mock payload is a
    /// Corvid expression the interpreter evaluates.
    ConnectorMockMode,
    /// The entry agent takes or returns a type the native command line
    /// cannot carry; the reason is the code generator's own.
    EntryBoundary { reason: String },
    /// A connector operation that native code cannot call yet; the
    /// reason names what (native dispatch N1 covers JSON-shaped calls).
    ConnectorNotNative {
        operation: String,
        reason: &'static str,
    },
    /// A tool the runtime implements whose declared types have no
    /// native JSON form.
    RuntimeToolNotNative {
        name: String,
    },
    /// Wider tagged unions and retry bodies outside the supported
    /// native `Result` / `Option` subset still route to the
    /// interpreter. Nullable-pointer `Option<T>` with a
    /// refcounted payload, wide scalar `Option<Int|Bool|Float>`, and
    /// the compositional native `Result<T, E>` subset are the
    /// supported native forms today.
    TaggedUnionRetryNotNative,
    /// `try ... else` (recoverable failures TE-2) runs on the interpreter
    /// until native code catches outages (TE-3).
    TryElseNotNative,
    /// An agent takes a `Caller`. The interpreter checks a called
    /// action's `requires` against the actor passed in (B10); native code
    /// does not yet, so it refuses rather than skip the check.
    CallerNotNative { agent: String },
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
    /// A builtin method native code does not run yet; `method` names it
    /// with its receiver type.
    BuiltinMethodNotNative { method: String },
    /// A tool of the program's own whose argument or answer type has no
    /// native JSON form.
    ToolTypeNotNative {
        name: String,
    },
    /// A prompt whose argument or reply type has no native JSON form
    /// (a function, a stream, a map with non-text keys, `JsonValue`).
    PromptTypeNotNative {
        name: String,
        type_name: String,
    },
}

impl std::fmt::Display for NotNativeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PythonImport { module } => write!(
                f,
                "program imports Python module `{module}` - native Python FFI is not implemented yet"
            ),
            Self::EntryBoundary { reason } => write!(f, "{reason}"),
            Self::ConnectorMockMode => write!(
                f,
                "connectors run in `mock` mode, whose payloads are Corvid expressions the interpreter evaluates"
            ),
            Self::RuntimeToolNotNative { name } => write!(
                f,
                "program calls `{name}`, a tool the runtime implements, with a type that has no native JSON form - it runs on the interpreter"
            ),
            Self::ConnectorNotNative { operation, reason } => write!(
                f,
                "program calls connector operation `{operation}`, which {reason} - it runs on the interpreter until native dispatch covers it"
            ),
            Self::ToolCall { name } => write!(
                f,
                "program calls tool `{name}` - pass `--with-tools-lib <path>` pointing at your compiled `#[tool]` staticlib, or let auto-dispatch fall back to the interpreter"
            ),
            Self::CallerNotNative { agent } => write!(
                f,
                "`{agent}` takes a `Caller`, whose action policy the interpreter checks on every call; native code does not yet (shared actions B10, on the roadmap)"
            ),
            Self::TryElseNotNative => write!(
                f,
                "program uses `try ... else`, which runs on the interpreter until native code can catch outages (recoverable failures TE-3, on the roadmap)"
            ),
            Self::TaggedUnionRetryNotNative => write!(
                f,
                "program uses a tagged-union or retry shape outside the current native subset - native AOT supports nullable-pointer `Option<T>`, wide scalar `Option<Int|Bool|Float>`, compositional native `Result<T, E>`, postfix `?`, and `try ... retry` over native `Result<T, E>` and `Option<T>` bodies; wider shapes still run in the interpreter"
            ),
            Self::TableOperation { table } => write!(
                f,
                "program reads or writes table `{table}` - table operations run on the interpreter until native tables land (B09 P6)"
            ),
            Self::AtomicBlock => write!(
                f,
                "program uses an `atomic:` block - atomic table writes run on the interpreter until native tables land (B09 P6)"
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
            Self::BuiltinMethodNotNative { method } => {
                write!(f, "program calls {method}, which native code does not run yet")
            }
            Self::UnsupportedMap => f.write_str(corvid_codegen_cl::NATIVE_MAP_REQUIREMENTS),
            Self::ToolTypeNotNative { name } => write!(
                f,
                "tool `{name}` sends or reads a type with no native JSON form; the auto-dispatcher runs the interpreter tier"
            ),
            Self::UnsupportedMatch => write!(f, "this match pattern or arm requires the interpreter"),
            Self::UnsupportedEquality => write!(f, "comparison of these value types requires the interpreter"),
            Self::PromptToolsRequireInterpreter => write!(f, "prompt function arguments require the interpreter"),
            Self::PromptTypeNotNative { name, type_name } => write!(
                f,
                "prompt `{name}` sends or reads a `{type_name}`, which has no native JSON form; the auto-dispatcher runs the interpreter tier"
            ),
            Self::CallbackNotNative => write!(f, "this callback signature or body requires the interpreter"),
            Self::PlaceAssignmentNotNative => {
                write!(f, "program uses place assignment (`x.field = v` / `xs[i] = v` / compound `op=`) - runs on the interpreter only; the auto-dispatcher runs the interpreter tier")
            }
        }
    }
}

/// The driver asks the code generator's own native type checks, as they
/// apply to a whole program before lowering (see `NativeTypeQuestion`), so
/// the two can never disagree about which types run natively.
fn is_native_value_type(ty: &Type) -> bool {
    corvid_codegen_cl::is_native_value_type_for(ty, NativeTypeQuestion::Program)
}

fn is_native_option_type(ty: &Type) -> bool {
    corvid_codegen_cl::is_native_option_type_for(ty, NativeTypeQuestion::Program)
}

fn is_native_option_expr_type(ty: &Type) -> bool {
    matches!(ty, Type::Option(inner) if matches!(**inner, Type::Unknown))
        || is_native_option_type(ty)
}

fn is_native_result_type(ty: &Type) -> bool {
    matches!(ty, Type::Result(ok, err) if is_native_value_type(ok) && is_native_value_type(err))
}

/// Whether native code can call a tool of the program's own: every
/// parameter and the answer have a native JSON form, a grounded answer
/// as its plain type (the call grounds it afterwards).
fn user_tool_is_native(tool_id: corvid_resolve::DefId, context: ScanContext<'_>) -> bool {
    context.tools.iter().find(|t| t.id == tool_id).is_some_and(|tool| {
        let mut plain = tool.clone();
        if let Type::Grounded(inner) = &tool.return_ty {
            plain.return_ty = (**inner).clone();
        }
        corvid_codegen_cl::native_runtime_call_types(&plain, context.types)
    })
}

/// Walk the IR and return `Ok(())` if every construct is native-able,
/// else the first reason found (early exit - one reason is enough to
/// route the caller to the interpreter tier).
pub fn native_ability(ir: &IrFile) -> Result<(), NotNativeReason> {
    native_ability_with_tools(ir, false)
}

#[derive(Clone, Copy)]
struct ScanContext<'a> {
    types: &'a [corvid_ir::IrType],
    has_tools_lib: bool,
    connectors: &'a [corvid_ir::IrConnector],
    tools: &'a [corvid_ir::IrTool],
}

/// Why native code cannot call connector operation `tool_id` yet, or
/// `Ok` when it can: through `corvid_runtime_call` (native dispatch N1),
/// for an operation whose arguments and answer have a native JSON form.
/// `None` when `tool_id` is not a connector operation.
fn connector_native_problem(tool_id: corvid_resolve::DefId, context: ScanContext<'_>) -> Option<Result<(), &'static str>> {
    let (connector, op) = context
        .connectors
        .iter()
        .find_map(|c| c.operations.iter().find(|op| op.tool_id == tool_id).map(|op| (c, op)))?;
    let tool = context.tools.iter().find(|t| t.id == tool_id)?;
    let problem = if connector.circuit_breaker.is_some() {
        Some("belongs to a connector with a `circuit_breaker`, whose count spans one interpreter run")
    } else if op.protocol.is_some() {
        Some("runs a provider protocol, whose durable lifecycle the interpreter drives")
    } else if op.streams.is_some() {
        Some("streams its response")
    } else if tool.params.iter().any(|p| matches!(p.ty, Type::Bytes | Type::Upload(_)))
        || op.body.as_ref().is_some_and(|b| b.encoding == corvid_ast::BodyEncoding::Multipart)
    {
        Some("sends a file or multipart body")
    } else if !corvid_codegen_cl::native_runtime_call_types(tool, context.types) {
        Some("sends or reads a type with no native JSON form")
    } else {
        None
    };
    Some(match problem {
        Some(reason) => Err(reason),
        None => Ok(()),
    })
}

/// `None` when the tool is not one the runtime implements; otherwise
/// whether its declared types have a native JSON form.
fn runtime_tool_is_native(tool_id: corvid_resolve::DefId, context: ScanContext<'_>) -> Option<bool> {
    let tool = context.tools.iter().find(|t| t.id == tool_id)?;
    if !corvid_runtime::runtime::is_native_runtime_tool(&tool.name) {
        return None;
    }
    Some(corvid_codegen_cl::native_runtime_call_types(tool, context.types))
}

pub(crate) fn native_ability_with_tools(ir: &IrFile, has_tools_lib: bool) -> Result<(), NotNativeReason> {
    let context = ScanContext { types: &ir.types, has_tools_lib, connectors: &ir.connectors, tools: &ir.tools };
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
        if agent.params.iter().any(|p| corvid_types::action_input::is_caller(&p.ty)) {
            return Err(NotNativeReason::CallerNotNative { agent: agent.name.clone() });
        }
        if agent.params.iter().map(|p| &p.ty).chain(std::iter::once(&agent.return_ty))
            .any(|ty| matches!(ty, Type::Map(_, _)) && !corvid_codegen_cl::native_string_map(ty)) {
            return Err(NotNativeReason::UnsupportedMap);
        }
        if matches!(agent.return_ty, Type::Stream(_)) {
            return Err(NotNativeReason::StreamLoweringNotImplemented);
        }
        scan_block(&agent.body, &agent.return_ty, context)?;
    }
    Ok(())
}

fn scan_block(block: &IrBlock, current_return_ty: &Type, context: ScanContext<'_>) -> Result<(), NotNativeReason> {
    for stmt in &block.stmts {
        scan_stmt(stmt, current_return_ty, context)?;
    }
    Ok(())
}

fn scan_stmt(stmt: &IrStmt, current_return_ty: &Type, context: ScanContext<'_>) -> Result<(), NotNativeReason> {
    if let Some((_, keys, value, _)) = corvid_codegen_cl::native_indexed_assignment(stmt) {
        for key in keys { scan_expr(key, current_return_ty, context)?; }
        return scan_expr(value, current_return_ty, context);
    }
    match stmt {
        IrStmt::Let { value, .. } => scan_expr(value, current_return_ty, context),
        IrStmt::Assign { value, .. }
            if corvid_codegen_cl::native_record_assignment(stmt)
                || corvid_codegen_cl::native_local_compound(stmt)
                || corvid_codegen_cl::native_scalar_compound(stmt) =>
        {
            scan_expr(value, current_return_ty, context)
        }
        IrStmt::Assign { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        IrStmt::Yield { .. } => Err(NotNativeReason::StreamLoweringNotImplemented),
        IrStmt::Return { value: Some(v), .. } => scan_expr(v, current_return_ty, context),
        IrStmt::Return { value: None, .. } => Ok(()),
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            scan_expr(cond, current_return_ty, context)?;
            scan_block(then_block, current_return_ty, context)?;
            if let Some(b) = else_block {
                scan_block(b, current_return_ty, context)?;
            }
            Ok(())
        }
        IrStmt::For { iter, body, .. } => {
            scan_expr(iter, current_return_ty, context)?;
            scan_block(body, current_return_ty, context)
        }
        // Destructuring is interpreter-only in 45n.
        IrStmt::Destructure { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        // Parallel blocks are interpreter-only in 46e.
        IrStmt::Parallel { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        // `while` (45k) lowers natively like `for` — walk cond + body.
        // Native tables arrive with B09 P6; an atomic block groups table writes.
        IrStmt::Atomic { .. } => Err(NotNativeReason::AtomicBlock),
        IrStmt::While { cond, body, .. } => {
            scan_expr(cond, current_return_ty, context)?;
            scan_block(body, current_return_ty, context)
        }
        IrStmt::Approve { args, .. } => {
            // Native `approve` calls the runtime's approval gate
            // (`corvid_approve_sync`), as the interpreter does. Walk the
            // arg expressions so any tool/prompt call buried in an
            // approve arg is reported.
            for a in args {
                scan_expr(a, current_return_ty, context)?;
            }
            Ok(())
        }
        IrStmt::Expr { expr, .. } => scan_expr(expr, current_return_ty, context),
        IrStmt::Break { .. } | IrStmt::Continue { .. } | IrStmt::Pass { .. } => Ok(()),
        // Ownership ops contain no user expressions; they don't change
        // whether this agent can run natively.
        IrStmt::Dup { .. } | IrStmt::Drop { .. } => Ok(()),
    }
}


fn scan_expr(expr: &IrExpr, current_return_ty: &Type, context: ScanContext<'_>) -> Result<(), NotNativeReason> {
    if let Some((receiver, key)) = corvid_codegen_cl::native_string_map_lookup(expr) {
        scan_expr(receiver, current_return_ty, context)?;
        return scan_expr(key, current_return_ty, context);
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
            keys.iter().chain(values).try_for_each(|item| scan_expr(item, current_return_ty, context))
        }
        IrExprKind::MapLiteral { .. } => Err(NotNativeReason::PlaceAssignmentNotNative),
        IrExprKind::Match { scrutinee, arms }
            if (is_native_option_type(&scrutinee.ty) || is_native_result_type(&scrutinee.ty) || matches!(scrutinee.ty, Type::Struct(_) | Type::ImportedStruct(_)))
                && corvid_codegen_cl::result_match::supports_match(&scrutinee.ty, arms) =>
        {
            scan_expr(scrutinee, current_return_ty, context)?;
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    scan_expr(guard, current_return_ty, context)?;
                }
                scan_expr(&arm.body, current_return_ty, context)?;
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
        IrExprKind::TableOp { table, .. } | IrExprKind::TableQuery { table, .. } => Err(NotNativeReason::TableOperation { table: table.name.clone() }),
        IrExprKind::BuiltinMethod {
            kind: corvid_types::BuiltinMethodKind::ListFirst | corvid_types::BuiltinMethodKind::ListLast,
            receiver,
            ..
        } if is_native_option_type(&expr.ty) => scan_expr(receiver, current_return_ty, context),
        IrExprKind::BuiltinMethod {
            kind: corvid_types::BuiltinMethodKind::ListLength, receiver, ..
        } => scan_expr(receiver, current_return_ty, context),
        // Native dispatch N4 — a `Response`'s accessors.
        IrExprKind::BuiltinMethod {
            kind:
                corvid_types::BuiltinMethodKind::ResponseStatus
                | corvid_types::BuiltinMethodKind::ResponseBody
                | corvid_types::BuiltinMethodKind::ResponseHeader,
            receiver,
            args,
        } => {
            scan_expr(receiver, current_return_ty, context)?;
            args.iter().try_for_each(|arg| scan_expr(arg, current_return_ty, context))
        }
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if corvid_codegen_cl::native_string_map(&receiver.ty) && matches!(kind,
                corvid_types::BuiltinMethodKind::MapKeys | corvid_types::BuiltinMethodKind::MapValues |
                corvid_types::BuiltinMethodKind::MapLength | corvid_types::BuiltinMethodKind::MapContainsKey) => {
            scan_expr(receiver, current_return_ty, context)?;
            args.iter().try_for_each(|arg| scan_expr(arg, current_return_ty, context))
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::ListAppend, receiver, args }
            if args.len() == 1 && corvid_codegen_cl::native_list_append(&receiver.ty, &args[0].ty) => {
            scan_expr(receiver, current_return_ty, context)?;
            scan_expr(&args[0], current_return_ty, context)
        }
        IrExprKind::BuiltinMethod { kind, receiver, args }
            if corvid_codegen_cl::native_integer_list_transform(*kind, &receiver.ty, args) || corvid_codegen_cl::native_numeric_conversion(*kind)
                || corvid_codegen_cl::native_scalar_to_string(*kind)
                || corvid_codegen_cl::native_string_method(*kind)
                || corvid_codegen_cl::native_text_method(*kind)
                || corvid_codegen_cl::is_native_decimal_method(*kind)
                || corvid_codegen_cl::is_native_time_method(*kind)
                || corvid_codegen_cl::is_native_currency_method(*kind)
                || corvid_codegen_cl::is_native_bytes_method(*kind)
                || matches!(kind, corvid_types::BuiltinMethodKind::StringParseInt | corvid_types::BuiltinMethodKind::ResultIsOk | corvid_types::BuiltinMethodKind::ResultIsErr) => {
            scan_expr(receiver, current_return_ty, context)?;
            args.iter().try_for_each(|arg| scan_expr(arg, current_return_ty, context))
        }
        IrExprKind::BuiltinMethod {
            kind: corvid_types::BuiltinMethodKind::JsonDecode | corvid_types::BuiltinMethodKind::JsonDecodeStrict,
            receiver,
            ..
        } if matches!(&expr.ty, Type::Result(target, _) if corvid_codegen_cl::native_public_json_type(target, context.types)) => {
            scan_expr(receiver, current_return_ty, context)
        }
        IrExprKind::BuiltinMethod { kind: corvid_types::BuiltinMethodKind::JsonEncode, receiver, .. }
            if corvid_codegen_cl::native_public_json_type(&receiver.ty, context.types) =>
        {
            scan_expr(receiver, current_return_ty, context)
        }
        IrExprKind::BuiltinMethod { kind, receiver, .. } => Err(NotNativeReason::BuiltinMethodNotNative {
            method: format!(
                "`{}` on `{}`",
                corvid_types::builtin_methods::builtin_method_name(&receiver.ty, *kind).unwrap_or("a builtin method"),
                receiver.ty.display_name()
            ),
        }),
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
                // A connector operation is a runtime-owned call; it needs
                // no tools library, only a native JSON shape.
                IrCallKind::Tool { def_id, .. } if connector_native_problem(*def_id, context).is_some() => {
                    if let Some(Err(reason)) = connector_native_problem(*def_id, context) {
                        return Err(NotNativeReason::ConnectorNotNative { operation: callee_name.clone(), reason });
                    }
                    for a in args {
                        scan_expr(a, current_return_ty, context)?;
                    }
                    return Ok(());
                }
                // A tool the runtime implements is a runtime-owned call
                // too (N2): no tools library, and its text-scalar types
                // (an `Instant`) cross as public JSON.
                IrCallKind::Tool { def_id, .. } if runtime_tool_is_native(*def_id, context).is_some() => {
                    if runtime_tool_is_native(*def_id, context) == Some(false) {
                        return Err(NotNativeReason::RuntimeToolNotNative { name: callee_name.clone() });
                    }
                    for a in args {
                        scan_expr(a, current_return_ty, context)?;
                    }
                    return Ok(());
                }
                IrCallKind::Tool { .. } if !context.has_tools_lib => {
                    return Err(NotNativeReason::ToolCall {
                        name: callee_name.clone(),
                    })
                }
                // A tool of the program's own is a runtime-owned call too
                // (native scalar boundaries B2b): its arguments and answer
                // cross in the public JSON form, secrets redacted and
                // revealed and text-held scalars checked, so it needs only
                // a native JSON form for each.
                IrCallKind::Tool { def_id, .. } if !user_tool_is_native(*def_id, context) => {
                    return Err(NotNativeReason::ToolTypeNotNative { name: callee_name.clone() })
                }
                IrCallKind::Prompt { .. } if args.iter().any(|arg| matches!(arg.ty, Type::Function { .. })) => {
                    return Err(NotNativeReason::PromptToolsRequireInterpreter);
                }
                IrCallKind::Prompt { .. } => {
                    // Prompt calls compile and run natively: arguments and
                    // replies cross in the public JSON form through the
                    // embedded plan (B1b), so any type with a native JSON
                    // form may cross. The runtime ships the LLM adapters.
                    let shown = |ty: &Type| match ty {
                        Type::Grounded(inner) => (**inner).clone(),
                        other => other.clone(),
                    };
                    let crossing = std::iter::once(shown(&expr.ty)).chain(args.iter().map(|arg| shown(&arg.ty)));
                    for ty in crossing {
                        if !corvid_codegen_cl::native_public_json_type(&ty, context.types) {
                            return Err(NotNativeReason::PromptTypeNotNative {
                                name: callee_name.clone(),
                                type_name: ty.display_name(),
                            });
                        }
                    }
                }
                IrCallKind::Tool { .. }
                | IrCallKind::Agent { .. }
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
                scan_expr(a, current_return_ty, context)?;
            }
            Ok(())
        }
        IrExprKind::FieldAccess { target, .. } => scan_expr(target, current_return_ty, context),
        IrExprKind::UnwrapGrounded { value } => scan_expr(value, current_return_ty, context),
        IrExprKind::Index { target, index } => {
            scan_expr(target, current_return_ty, context)?;
            scan_expr(index, current_return_ty, context)
        }
        IrExprKind::BinOp { op, left, right } | IrExprKind::WrappingBinOp { op, left, right } => {
            if corvid_codegen_cl::lowering::uses_structural_equality(*op, &left.ty)
                && !corvid_codegen_cl::lowering::supports_structural_comparison(*op, left, right, context.types) {
                return Err(NotNativeReason::UnsupportedEquality);
            }
            scan_expr(left, current_return_ty, context)?;
            scan_expr(right, current_return_ty, context)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            scan_expr(operand, current_return_ty, context)
        }
        IrExprKind::List { items } => {
            for it in items {
                scan_expr(it, current_return_ty, context)?;
            }
            Ok(())
        }
        IrExprKind::WeakNew { strong } => scan_expr(strong, current_return_ty, context),
        IrExprKind::WeakUpgrade { weak } => scan_expr(weak, current_return_ty, context),
        IrExprKind::StreamSplitBy { stream, .. } => {
            scan_expr(stream, current_return_ty, context)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamMerge { groups, .. } => {
            scan_expr(groups, current_return_ty, context)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamOrderedBy { stream, .. } => {
            scan_expr(stream, current_return_ty, context)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::StreamResumeToken { stream } => {
            scan_expr(stream, current_return_ty, context)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        IrExprKind::ResumeStream { token, .. } => {
            scan_expr(token, current_return_ty, context)?;
            Err(NotNativeReason::StreamLoweringNotImplemented)
        }
        // Tagged-union/retry nodes are accepted only for the current
        // native subset. Recurse into sub-expressions first so any
        // nested tool/prompt calls still get reported correctly.
        IrExprKind::OptionSome { inner } => {
            scan_expr(inner, current_return_ty, context)?;
            if is_native_option_expr_type(&expr.ty) {
                Ok(())
            } else {
                Err(NotNativeReason::TaggedUnionRetryNotNative)
            }
        }
        IrExprKind::ResultOk { inner } | IrExprKind::ResultErr { inner } => {
            scan_expr(inner, current_return_ty, context)?;
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
            scan_expr(inner, current_return_ty, context)?;
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
        IrExprKind::TrustBoundary { inner } => scan_expr(inner, current_return_ty, context),
        IrExprKind::TryRetry { fallback: Some(_), .. } => Err(NotNativeReason::TryElseNotNative),
        IrExprKind::TryRetry { body, .. } => {
            scan_expr(body, current_return_ty, context)?;
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
    use super::{native_ability, native_ability_with_tools, NotNativeReason};
    use corvid_ast::Span;
    use corvid_ir::{IrAgent, IrBlock, IrFile};
    use corvid_resolve::DefId;
    use corvid_types::Type;

    #[test]
    fn tool_library_does_not_hide_later_interpreter_requirements() {
        let ir = crate::compile_to_ir(r#"
tool fetch() -> String
agent main() -> Int:
    value = fetch()
    words = [value, "other"]
    words.sort()
    return words.length()
"#).expect("valid program using a tool and list sorting");
        assert!(matches!(native_ability(&ir), Err(NotNativeReason::ToolCall { .. })));
        assert!(matches!(native_ability_with_tools(&ir, true), Err(NotNativeReason::BuiltinMethodNotNative { .. })));
    }

    #[test]
    fn decimal_programs_are_native_except_at_tool_and_prompt_boundaries() {
        let ir = crate::compile_to_ir(r#"
type Line:
    price: Decimal
    quantity: Int

agent main() -> String:
    line = Line(4.50, 2)
    total = line.price * line.quantity
    share = (total / 3).round_half_even(2)
    if total == 9:
        return share.to_string()
    return total.to_string()
"#).expect("valid decimal program");
        assert!(native_ability(&ir).is_ok(), "{:?}", native_ability(&ir));

        let ir = crate::compile_to_ir(r#"
tool quote(sku: String) -> Decimal
agent main() -> String:
    return quote("a").to_string()
"#).expect("valid program with a Decimal tool");
        // B2b: a tool of the program's own takes and returns a `Decimal`
        // natively, through the runtime call.
        assert_eq!(native_ability_with_tools(&ir, true), Ok(()));

        let ir = crate::compile_to_ir(r#"
agent main() -> String:
    total: Decimal = 12.5
    return total.format_currency("EUR", "de-DE")
"#).expect("valid currency program");
        assert!(native_ability(&ir).is_ok(), "{:?}", native_ability(&ir));

        let ir = crate::compile_to_ir(r#"
agent main() -> Decimal:
    total: Decimal = 0
    total += 0.10
    return total
"#).expect("valid compound decimal assignment");
        // Compound assignment on a `Decimal` local is the rebinding it
        // means, which is native (`scalar_compound`).
        assert!(native_ability(&ir).is_ok(), "{:?}", native_ability(&ir));
    }

    #[test]
    fn byte_string_programs_are_native_except_at_tool_and_prompt_boundaries() {
        for src in [
            "agent main() -> String:\n    return \"abc\".utf8().sha256().to_hex()\n",
            "agent same(a: Bytes, b: Bytes) -> Bool:\n    return a == b\n\nagent main() -> Bool:\n    return same(\"a\".utf8(), \"a\".utf8())\n",
            "type Signed:\n    body: String\n    signature: Bytes\n\nagent main() -> Bool:\n    s = Signed(\"b\", \"b\".utf8().hmac_sha256(\"k\".utf8()))\n    return s.body.utf8().verify_ed25519(s.signature, s.signature)\n",
        ] {
            let ir = crate::compile_to_ir(src).expect("valid bytes program");
            assert_eq!(native_ability(&ir), Ok(()), "{src}");
        }
        let ir = crate::compile_to_ir("tool fetch() -> List<Bytes>\nagent main() -> Int:\n    return fetch().length()\n")
            .expect("valid bytes tool program");
        assert_eq!(native_ability_with_tools(&ir, true), Ok(()));
    }

    #[test]
    fn secrets_are_native_except_at_tool_boundaries() {
        // Native dispatch N3: reading a secret through the runtime,
        // holding one in a record and comparing two compile natively.
        for src in [
            "effect secrets_read:\n    reversible: true\n\ntool secret(name: String) -> Result<Secret, String> uses secrets_read\n\nagent main() -> Bool:\n    return match secret(\"KEY\"):\n        Ok(key) -> true\n        Err(problem) -> false\n",
            "agent same(a: Secret, b: Secret) -> Bool:\n    return a == b\n\nagent main() -> Int:\n    return 1\n",
            "type Credentials:\n    user: String\n    password: Secret\n\nagent main() -> Int:\n    return 1\n",
            // A tool that is never called produces nothing.
            "tool sign(key: Secret, body: String) -> String\nagent main() -> Int:\n    return 1\n",
        ] {
            let ir = crate::compile_to_ir(src).expect("valid secret program");
            assert_eq!(native_ability_with_tools(&ir, true), Ok(()), "{src}");
        }
        // B2b: a tool of the program's own that takes or returns a secret
        // runs natively; the runtime call records `<redacted>` and reveals
        // the text only to the tool.
        for src in [
            "effect secrets_read:\n    reversible: true\n\ntool secret(name: String) -> Result<Secret, String> uses secrets_read\ntool sign(key: Secret, body: String) -> String\nagent main() -> String:\n    return match secret(\"KEY\"):\n        Ok(key) -> sign(key, \"body\")\n        Err(problem) -> problem\n",
        ] {
            let ir = crate::compile_to_ir(src).expect("valid secret program");
            assert_eq!(native_ability_with_tools(&ir, true), Ok(()), "{src}");
        }
        // Only the runtime's own tools produce a secret; one of the
        // program's own returning one stays on the interpreter.
        let ir = crate::compile_to_ir("tool key() -> Secret\nagent main() -> Int:\n    k = key()\n    return 1\n").expect("valid");
        assert_eq!(native_ability_with_tools(&ir, true), Err(NotNativeReason::ToolTypeNotNative { name: "key".into() }));
        // N3b: the cryptography a secret keys runs natively.
        for src in [
            "agent mac(key: Secret) -> Bytes:\n    return \"body\".utf8().hmac_sha256(key)\n\nagent main() -> Int:\n    return 1\n",
            "agent check(password: Secret) -> Bool:\n    return password.verify_password(\"x\")\n\nagent main() -> Int:\n    return 1\n",
        ] {
            let ir = crate::compile_to_ir(src).expect("valid secret program");
            assert_eq!(native_ability(&ir), Ok(()), "{src}");
        }
    }

    #[test]
    fn time_programs_are_native_except_at_tool_and_prompt_boundaries() {
        let ir = crate::compile_to_ir(r#"
agent main() -> String:
    start = Instant.from_epoch_milliseconds(1774684800000)
    local = start.in_zone("Europe/Berlin")
    later = local.add_days(1) + (start - start)
    berlin: TimeZone = "Europe/Berlin"
    if later.instant() > start and local.time_zone() == berlin:
        return later.to_string() + " " + [local.hour()].to_json()
    return "x"
"#).expect("valid time program");
        assert!(native_ability(&ir).is_ok(), "{:?}", native_ability(&ir));

        // `time_now` is a std tool returning an `Instant`, so a program
        // that reads the clock stays on the interpreter, where replay
        // records and substitutes it.
        let ir = crate::compile_to_ir(r#"
tool now() -> Instant
agent main() -> String:
    return now().to_string()
"#).expect("valid program with a clock tool");
        assert_eq!(native_ability_with_tools(&ir, true), Ok(()));
    }

    /// B1b: a prompt whose arguments and reply have a native JSON form
    /// runs natively, records, lists, `Option`, sum types and text-held
    /// scalars included; one with a type that has none stays on the
    /// interpreter and says which.
    #[test]
    fn prompts_are_native_when_their_types_have_a_native_json_form() {
        let ir = crate::compile_to_ir(r#"
type Severity:
    | Low
    | High(reason: String)

type Verdict:
    summary: String
    owner: Option<String>
    severity: Severity
    refund: Decimal

prompt triage(tags: List<String>, budget: Decimal, wait: Duration) -> Verdict:
    "Triage {tags} within {budget} and {wait}"

agent main() -> String:
    return triage(["login"], 12.50, 30 seconds).summary
"#).expect("valid prompt program");
        assert_eq!(native_ability(&ir), Ok(()));

        let ir = crate::compile_to_ir(r#"
prompt read(n: Int) -> JsonValue:
    "Read {n}"

agent main() -> Int:
    raw = read(1)
    return 1
"#).expect("valid prompt program");
        assert_eq!(
            native_ability(&ir),
            Err(NotNativeReason::PromptTypeNotNative { name: "read".into(), type_name: "JsonValue".into() })
        );
    }

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
            actions: vec![],
        };

        assert!(matches!(
            native_ability(&ir),
            Err(NotNativeReason::StreamLoweringNotImplemented)
        ));
    }
}
