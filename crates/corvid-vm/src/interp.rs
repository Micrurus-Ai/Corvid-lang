//! Tree-walking interpreter, async edition.
//!
//! Asynchronous from the top because tool calls, prompt calls, and
//! approvals are async at the runtime boundary. The performance hit of
//! boxing recursive futures (via `async-recursion`) is the price for
//! keeping this tier behaviourally identical to the future Cranelift
//! backend, which will also be async-native. Behavioural parity is what
//! makes this interpreter useful as a correctness oracle.

#[path = "interp/effect_compose.rs"]
mod effect_compose;
#[path = "interp/expr.rs"]
mod expr;
#[path = "interp/grounding.rs"]
mod grounding;
#[path = "interp/prompt/mod.rs"]
mod prompt;
#[path = "interp/replay.rs"]
mod replay;
#[path = "interp/run_validate.rs"]
mod run_validate;
#[path = "interp/stmt.rs"]
mod stmt;
#[path = "interp/stream_ops.rs"]
mod stream_ops;
#[path = "interp/test_runner.rs"]
mod test_runner;
#[path = "interp/test_trace.rs"]
mod test_trace;

pub use run_validate::{
    bind_and_run_agent, build_struct, run_agent, run_agent_stepping, run_agent_with_env,
};
pub use test_runner::{
    run_all_tests, run_all_tests_with_options, run_test, SnapshotOptions, TestAssertionExecution,
    TestAssertionStatus, TestExecution, TestRunOptions, TraceFixtureOptions,
};

use self::expr::{eval_binop, eval_builtin_method, eval_literal, eval_unop, require_bool};
use self::grounding::{maybe_ground_tool_result, tool_has_retrieval_effect};
use crate::conv::{json_to_value, value_to_json};
use crate::env::Env;
use crate::errors::{InterpError, InterpErrorKind};
use crate::step::{self, ConfidenceGateStep, StepAction, StepController, StepEvent};
use crate::value::{
    value_confidence, BoxedValue, ClosureValue, ListValue, StreamChunk, StreamSender, Value,
};
use async_recursion::async_recursion;
use corvid_ast::{BinaryOp, Span};
use corvid_ir::{IrPattern, 
    IrAgent, IrCallKind, IrExpr, IrExprKind, IrFile, IrFixture, IrMock, IrParam, IrPrompt, IrTool,
    IrType,
};
use corvid_resolve::{DefId, LocalId};
use corvid_runtime::{DbValue, Runtime};
use corvid_types::Type;
use effect_compose::{composed_confidence, default_stream_backpressure, stream_start_is_retryable};
use std::collections::HashMap;
use std::sync::Arc;

/// Control-flow outcome of evaluating a statement or block.
#[derive(Debug, Clone)]
enum Flow {
    Normal,
    Return(Value),
    Break,
    Continue,
}

#[derive(Debug, Clone)]
enum ExprFlow {
    Value(Value),
    Propagate(Value),
}

impl ExprFlow {
    fn into_value(self) -> Result<Value, Value> {
        match self {
            ExprFlow::Value(v) => Ok(v),
            ExprFlow::Propagate(v) => Err(v),
        }
    }
}

struct Interpreter<'ir> {
    ir: &'ir IrFile,
    env: Env,
    types_by_id: HashMap<DefId, &'ir IrType>,
    tools_by_id: HashMap<DefId, &'ir IrTool>,
    prompts_by_id: HashMap<DefId, &'ir IrPrompt>,
    agents_by_id: HashMap<DefId, &'ir IrAgent>,
    fixtures_by_id: HashMap<DefId, &'ir IrFixture>,
    mock_tools: HashMap<DefId, &'ir IrMock>,
    runtime: &'ir Runtime,
    local_names: HashMap<LocalId, String>,
    stepper: Option<StepController>,
    stream_sender: Option<StreamSender>,
    stream_locals: HashMap<LocalId, StreamChunk>,
    cost_budget: Option<f64>,
    cost_used: f64,
    stream_cost_budget: Option<f64>,
    stream_cost_used: f64,
}

impl<'ir> Interpreter<'ir> {
    fn new(ir: &'ir IrFile, runtime: &'ir Runtime) -> Self {
        let types_by_id: HashMap<DefId, &IrType> = ir.types.iter().map(|t| (t.id, t)).collect();
        let tools_by_id: HashMap<DefId, &IrTool> = ir.tools.iter().map(|t| (t.id, t)).collect();
        let prompts_by_id: HashMap<DefId, &IrPrompt> =
            ir.prompts.iter().map(|p| (p.id, p)).collect();
        let agents_by_id: HashMap<DefId, &IrAgent> = ir.agents.iter().map(|a| (a.id, a)).collect();
        let fixtures_by_id: HashMap<DefId, &IrFixture> =
            ir.fixtures.iter().map(|f| (f.id, f)).collect();
        Self {
            ir,
            env: Env::new(),
            types_by_id,
            tools_by_id,
            prompts_by_id,
            agents_by_id,
            fixtures_by_id,
            mock_tools: HashMap::new(),
            runtime,
            local_names: HashMap::new(),
            stepper: None,
            stream_sender: None,
            stream_locals: HashMap::new(),
            cost_budget: None,
            cost_used: 0.0,
            stream_cost_budget: None,
            stream_cost_used: 0.0,
        }
    }

    fn with_mocks(mut self) -> Self {
        self.mock_tools = self.ir.mocks.iter().map(|m| (m.target_id, m)).collect();
        self
    }

    fn bind_params(&mut self, agent: &'ir IrAgent, args: Vec<Value>) -> Result<(), InterpError> {
        if agent.params.len() != args.len() {
            return Err(InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "agent `{}` expects {} arg(s), got {}",
                    agent.name,
                    agent.params.len(),
                    args.len()
                )),
                agent.span,
            ));
        }
        for (p, v) in agent.params.iter().zip(args) {
            self.env.bind(p.local_id, v.clone());
            self.local_names.insert(p.local_id, p.name.clone());
            self.stream_locals.remove(&p.local_id);
        }
        Ok(())
    }

    fn bind_ir_params(
        &mut self,
        callable: &str,
        params: &'ir [IrParam],
        args: Vec<Value>,
        span: Span,
    ) -> Result<(), InterpError> {
        if params.len() != args.len() {
            return Err(InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "`{callable}` expects {} arg(s), got {}",
                    params.len(),
                    args.len()
                )),
                span,
            ));
        }
        for (p, v) in params.iter().zip(args) {
            self.env.bind(p.local_id, v.clone());
            self.local_names.insert(p.local_id, p.name.clone());
            self.stream_locals.remove(&p.local_id);
        }
        Ok(())
    }

    fn env_snapshot(&self) -> step::EnvSnapshot {
        step::snapshot_env(&self.env, &self.local_names)
    }

    async fn maybe_yield(&mut self, event: StepEvent) -> Result<StepAction, InterpError> {
        if let Some(stepper) = self.stepper.as_mut() {
            let action = stepper.yield_event(event).await;
            if matches!(action, StepAction::Abort) {
                return Err(InterpError::new(
                    InterpErrorKind::Other("execution aborted by step controller".into()),
                    Span::new(0, 0),
                ));
            }
            Ok(action)
        } else {
            Ok(StepAction::Resume)
        }
    }

    fn should_yield_statement(&self) -> bool {
        self.stepper
            .as_ref()
            .is_some_and(|s| s.should_yield_on_statement())
    }

    fn should_yield_boundary(&self) -> bool {
        self.stepper
            .as_ref()
            .is_some_and(|s| s.should_yield_on_boundary())
    }

    #[async_recursion]
    // NOTE: `expr` is NOT tied to the `'ir` file lifetime — closure
    // bodies (slice 45j) live inside `Value::Closure` cells and are
    // evaluated through this same entry point.
    async fn eval_expr(&mut self, expr: &IrExpr) -> Result<ExprFlow, InterpError> {
        match &expr.kind {
            // Builtin methods (slice 45c) — one arm per
            // `BuiltinMethodKind`; the shared corvid_types table
            // guarantees the checker only let matching receivers
            // through.
            IrExprKind::BuiltinMethod {
                kind,
                receiver,
                args,
            } => {
                let recv = match self.eval_expr(receiver).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    match self.eval_expr(a).await?.into_value() {
                        Ok(v) => arg_vals.push(v),
                        Err(v) => return Ok(ExprFlow::Propagate(v)),
                    }
                }
                // The lambda-taking methods (slice 45j) re-enter
                // the evaluator to apply the closure, so they
                // dispatch here in the async path instead of the
                // sync helper.
                {
                    use corvid_types::BuiltinMethodKind as Bmk;
                    if matches!(
                        kind,
                        Bmk::ListMap
                            | Bmk::ListFilter
                            | Bmk::ListFold
                            | Bmk::ListAny
                            | Bmk::ListAll
                            | Bmk::ResultMapErr
                    ) {
                        return self
                            .eval_higher_order_list_method(*kind, recv, arg_vals, expr.span)
                            .await
                            .map(ExprFlow::Value);
                    }
                }
                Ok(ExprFlow::Value(eval_builtin_method(
                    *kind, recv, arg_vals, expr.span,
                )?))
            }
            IrExprKind::Literal(lit) => Ok(ExprFlow::Value(eval_literal(lit))),

            // Named struct literal (slice 45n): build the cell from
            // the spread's fields first (handle copies into a NEW
            // cell), then apply the named overrides.
            IrExprKind::StructLiteral {
                def_id,
                type_name,
                fields,
                spread,
            } => {
                let mut out: Vec<(String, Value)> = Vec::new();
                if let Some(s) = spread {
                    let base = match self.eval_expr(s).await?.into_value() {
                        Ok(v) => v,
                        Err(v) => return Ok(ExprFlow::Propagate(v)),
                    };
                    match base {
                        Value::Struct(sv) => {
                            sv.with_fields(|m| {
                                for (k, v) in m {
                                    out.push((k.clone(), v.clone()));
                                }
                            });
                        }
                        other => {
                            return Err(InterpError::new(
                                InterpErrorKind::TypeMismatch {
                                    expected: type_name.clone(),
                                    got: other.type_name(),
                                },
                                expr.span,
                            ));
                        }
                    }
                }
                for (fname, fexpr) in fields {
                    let v = match self.eval_expr(fexpr).await?.into_value() {
                        Ok(v) => v,
                        Err(v) => return Ok(ExprFlow::Propagate(v)),
                    };
                    if let Some(slot) = out.iter_mut().find(|(k, _)| k == fname) {
                        slot.1 = v;
                    } else {
                        out.push((fname.clone(), v));
                    }
                }
                Ok(ExprFlow::Value(Value::new_struct(
                    *def_id,
                    type_name.clone(),
                    out,
                )))
            }
            // Lambda (slice 45j): evaluate to a closure that
            // snapshots the visible environment BY VALUE. Values
            // clone; heap cells share.
            IrExprKind::Lambda { params, body } => {
                Ok(ExprFlow::Value(Value::Closure(ClosureValue::new(
                    params
                        .iter()
                        .map(|p| (p.local_id, p.name.clone()))
                        .collect(),
                    (**body).clone(),
                    self.env.entries_snapshot(),
                ))))
            }

            IrExprKind::Local { local_id, .. } => self
                .env
                .lookup(*local_id)
                .map(ExprFlow::Value)
                .ok_or_else(|| {
                    InterpError::new(InterpErrorKind::UndefinedLocal(*local_id), expr.span)
                }),

            IrExprKind::Decl { .. } => Err(InterpError::new(
                InterpErrorKind::NotImplemented(
                    "bare top-level declaration reference (imports/functions)".into(),
                ),
                expr.span,
            )),

            IrExprKind::Call {
                kind,
                callee_name,
                args,
            } => {
                self.eval_call(kind, callee_name, args, &expr.ty, expr.span)
                    .await
            }

            IrExprKind::Ask { prompt, target_ty } => {
                let prompt = match self.eval_expr(prompt).await?.into_value() {
                    Ok(Value::String(s)) => s.to_string(),
                    Ok(other) => {
                        return Err(InterpError::new(
                            InterpErrorKind::TypeMismatch {
                                expected: "String".into(),
                                got: other.type_name(),
                            },
                            prompt.span,
                        ));
                    }
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let value = self
                    .runtime
                    .ask_human(&prompt, target_ty.display_name())
                    .await
                    .map_err(|err| InterpError::new(InterpErrorKind::Runtime(err), expr.span))?;
                let value = json_to_value(value, target_ty, &self.types_by_id).map_err(|err| {
                    InterpError::new(
                        InterpErrorKind::Runtime(corvid_runtime::RuntimeError::Marshal(
                            err.to_string(),
                        )),
                        expr.span,
                    )
                })?;
                Ok(ExprFlow::Value(value))
            }

            IrExprKind::Choose { options } => {
                let options_value = match self.eval_expr(options).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let Value::List(list) = options_value else {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "List".into(),
                            got: options_value.type_name(),
                        },
                        options.span,
                    ));
                };
                let values = list.iter_cloned();
                let options_json = values.iter().map(value_to_json).collect::<Vec<_>>();
                let index = self
                    .runtime
                    .choose_human(options_json)
                    .await
                    .map_err(|err| InterpError::new(InterpErrorKind::Runtime(err), expr.span))?;
                let Some(value) = values.get(index).cloned() else {
                    return Err(InterpError::new(
                        InterpErrorKind::Runtime(corvid_runtime::RuntimeError::Other(format!(
                            "human choice index {index} out of range"
                        ))),
                        expr.span,
                    ));
                };
                Ok(ExprFlow::Value(value))
            }

            IrExprKind::FieldAccess { target, field } => {
                let t = match self.eval_expr(target).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match t {
                    Value::Struct(s) => s.get_field(field).map(ExprFlow::Value).ok_or_else(|| {
                        InterpError::new(
                            InterpErrorKind::UnknownField {
                                struct_name: s.type_name().to_string(),
                                field: field.clone(),
                            },
                            expr.span,
                        )
                    }),
                    Value::Partial(p) => p.get_field(field).map(ExprFlow::Value).ok_or_else(|| {
                        InterpError::new(
                            InterpErrorKind::UnknownField {
                                struct_name: p.type_name().to_string(),
                                field: field.clone(),
                            },
                            expr.span,
                        )
                    }),
                    other => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "struct".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::UnwrapGrounded { value } => {
                let value = match self.eval_expr(value).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match value {
                    Value::Grounded(grounded) => {
                        // Slice 7b semantics: discard the
                        // provenance chain (the typechecker-flagged
                        // legacy coercion is the user explicitly
                        // dropping "where it came from"), but
                        // preserve confidence — "how trusty it is"
                        // is a separate concern that downstream
                        // confidence-gate checks still need. The
                        // re-wrap keeps `Value::Grounded` only when
                        // confidence < 1.0 so a fully-confident
                        // value strips clean to bare.
                        let conf = grounded.confidence;
                        let inner = grounded.inner.get();
                        if conf < 1.0 {
                            Ok(ExprFlow::Value(Value::Grounded(
                                crate::value::GroundedValue::with_confidence(
                                    inner,
                                    crate::ProvenanceChain::new(),
                                    conf,
                                ),
                            )))
                        } else {
                            Ok(ExprFlow::Value(inner))
                        }
                    }
                    // Defensive no-op: the typechecker only inserts
                    // `UnwrapGrounded` where it observed a
                    // `Grounded<T> -> T` coercion, so the runtime
                    // should always deliver `Value::Grounded` here
                    // (`maybe_ground_*_result` aligns the runtime
                    // with the type promise). If a tier ships out of
                    // sync, the strip is a harmless identity rather
                    // than a panic.
                    other => Ok(ExprFlow::Value(other)),
                }
            }

            IrExprKind::Index { target, index } => {
                let t = match self.eval_expr(target).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let i = match self.eval_expr(index).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match (t, i) {
                    // Map read (45g): m[k] returns Option<V> — a
                    // missing key is None, never a trap.
                    (Value::Map(m), key) => Ok(ExprFlow::Value(match m.get_by_key(&key) {
                        Some(v) => Value::OptionSome(crate::value::BoxedValue::new(v)),
                        None => Value::OptionNone,
                    })),
                    (Value::List(items), Value::Int(idx)) => {
                        let len = items.len();
                        let in_range = idx >= 0 && (idx as usize) < len;
                        if !in_range {
                            return Err(InterpError::new(
                                InterpErrorKind::IndexOutOfBounds { len, index: idx },
                                expr.span,
                            ));
                        }
                        Ok(ExprFlow::Value(
                            items.get(idx as usize).expect("checked list index"),
                        ))
                    }
                    (other, _) => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "List".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::BinOp { op, left, right } => {
                // Short-circuit `and` / `or`: evaluate the right operand
                // only when the left doesn't determine the result. This
                // matches the Cranelift lowering's merge-block pattern
                // and lets idioms like `true or (1 / 0 == 0)` return
                // `true` instead of raising.
                match op {
                    BinaryOp::And => {
                        let l = match self.eval_expr(left).await?.into_value() {
                            Ok(v) => v,
                            Err(v) => return Ok(ExprFlow::Propagate(v)),
                        };
                        let lb = require_bool(&l, left.span, "left operand of `and`")?;
                        if !lb {
                            return Ok(ExprFlow::Value(Value::Bool(false)));
                        }
                        let r = match self.eval_expr(right).await?.into_value() {
                            Ok(v) => v,
                            Err(v) => return Ok(ExprFlow::Propagate(v)),
                        };
                        let rb = require_bool(&r, right.span, "right operand of `and`")?;
                        return Ok(ExprFlow::Value(Value::Bool(rb)));
                    }
                    BinaryOp::Or => {
                        let l = match self.eval_expr(left).await?.into_value() {
                            Ok(v) => v,
                            Err(v) => return Ok(ExprFlow::Propagate(v)),
                        };
                        let lb = require_bool(&l, left.span, "left operand of `or`")?;
                        if lb {
                            return Ok(ExprFlow::Value(Value::Bool(true)));
                        }
                        let r = match self.eval_expr(right).await?.into_value() {
                            Ok(v) => v,
                            Err(v) => return Ok(ExprFlow::Propagate(v)),
                        };
                        let rb = require_bool(&r, right.span, "right operand of `or`")?;
                        return Ok(ExprFlow::Value(Value::Bool(rb)));
                    }
                    _ => {}
                }
                let l = match self.eval_expr(left).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let r = match self.eval_expr(right).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(eval_binop(*op, l, r, expr.span, false)?))
            }

            IrExprKind::WrappingBinOp { op, left, right } => {
                let l = match self.eval_expr(left).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let r = match self.eval_expr(right).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(eval_binop(*op, l, r, expr.span, true)?))
            }

            IrExprKind::UnOp { op, operand } => {
                let v = match self.eval_expr(operand).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(eval_unop(*op, v, expr.span, false)?))
            }

            IrExprKind::WrappingUnOp { op, operand } => {
                let v = match self.eval_expr(operand).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(eval_unop(*op, v, expr.span, true)?))
            }

            // `match` (45i): evaluate the scrutinee once, then try
            // each arm in order. Pattern bindings write into the flat
            // function scope (Python-style); a failed guard leaves
            // its bindings set, matching the flat-scope model. The
            // checker's exhaustiveness pass makes the no-arm trap
            // unreachable except through guards.
            IrExprKind::Match { scrutinee, arms } => {
                let value = match self.eval_expr(scrutinee).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                for arm in arms {
                    let mut bindings: Vec<(corvid_resolve::LocalId, String, Value)> = Vec::new();
                    if !pattern_matches(&arm.pattern, &value, &mut bindings) {
                        continue;
                    }
                    for (local_id, name, v) in bindings {
                        self.env.bind(local_id, v);
                        self.local_names.insert(local_id, name);
                    }
                    if let Some(guard) = &arm.guard {
                        let g = match self.eval_expr(guard).await?.into_value() {
                            Ok(v) => v,
                            Err(v) => return Ok(ExprFlow::Propagate(v)),
                        };
                        match g {
                            Value::Bool(true) => {}
                            Value::Bool(false) => continue,
                            other => {
                                return Err(InterpError::new(
                                    InterpErrorKind::TypeMismatch {
                                        expected: "Bool".into(),
                                        got: other.type_name(),
                                    },
                                    guard.span,
                                ))
                            }
                        }
                    }
                    return self.eval_expr(&arm.body).await;
                }
                Err(InterpError::new(
                    InterpErrorKind::DispatchFailed(
                        "no match arm matched the scrutinee (guards excluded every arm)"
                            .to_string(),
                    ),
                    expr.span,
                ))
            }
            IrExprKind::MapLiteral { keys, values } => {
                let mut entries = Vec::with_capacity(keys.len());
                for (k, v) in keys.iter().zip(values) {
                    let kv = match self.eval_expr(k).await?.into_value() {
                        Ok(x) => x,
                        Err(x) => return Ok(ExprFlow::Propagate(x)),
                    };
                    let vv = match self.eval_expr(v).await?.into_value() {
                        Ok(x) => x,
                        Err(x) => return Ok(ExprFlow::Propagate(x)),
                    };
                    entries.push((kv, vv));
                }
                // MapValue::new applies last-duplicate-wins.
                Ok(ExprFlow::Value(Value::Map(crate::value::MapValue::new(
                    entries,
                ))))
            }
            IrExprKind::List { items } => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match self.eval_expr(it).await?.into_value() {
                        Ok(v) => out.push(v),
                        Err(v) => return Ok(ExprFlow::Propagate(v)),
                    }
                }
                Ok(ExprFlow::Value(Value::List(ListValue::new(out))))
            }

            IrExprKind::WeakNew { strong } => {
                let strong = match self.eval_expr(strong).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                let weak = strong.downgrade().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "String, Struct, or List".into(),
                            got: strong.type_name(),
                        },
                        expr.span,
                    )
                })?;
                Ok(ExprFlow::Value(Value::Weak(weak)))
            }

            IrExprKind::WeakUpgrade { weak } => {
                let weak = match self.eval_expr(weak).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match weak {
                    Value::Weak(weak) => match weak.upgrade() {
                        Some(value) => {
                            Ok(ExprFlow::Value(Value::OptionSome(BoxedValue::new(value))))
                        }
                        None => Ok(ExprFlow::Value(Value::OptionNone)),
                    },
                    other => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Weak".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::StreamSplitBy { stream, key } => {
                let stream = match self.eval_expr(stream).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                stream_ops::split_by(stream, key, expr.span)
                    .await
                    .map(ExprFlow::Value)
            }

            IrExprKind::StreamMerge { groups, policy } => {
                let groups = match self.eval_expr(groups).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                stream_ops::merge(groups, *policy, expr.span)
                    .await
                    .map(ExprFlow::Value)
            }

            IrExprKind::StreamOrderedBy { stream, policy } => {
                let stream = match self.eval_expr(stream).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                stream_ops::ordered_by(stream, *policy, expr.span)
                    .await
                    .map(ExprFlow::Value)
            }

            IrExprKind::StreamResumeToken { stream } => {
                let stream = match self.eval_expr(stream).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match stream {
                    Value::Stream(stream) => match stream.resume_token() {
                        Some(token) => Ok(ExprFlow::Value(Value::ResumeToken(token))),
                        None => Err(InterpError::new(
                            InterpErrorKind::Other(
                                "stream does not carry a resumable prompt context".into(),
                            ),
                            expr.span,
                        )),
                    },
                    other => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Stream<T>".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::ResumeStream {
                prompt_def_id,
                prompt_name,
                token,
            } => {
                let token = match self.eval_expr(token).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match token {
                    Value::ResumeToken(token) => {
                        self.resume_prompt_stream(*prompt_def_id, prompt_name, token, expr.span)
                            .await
                    }
                    other => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "ResumeToken<T>".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::ResultOk { inner } => {
                let v = match self.eval_expr(inner).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(Value::ResultOk(BoxedValue::new(v))))
            }

            IrExprKind::ResultErr { inner } => {
                let v = match self.eval_expr(inner).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(Value::ResultErr(BoxedValue::new(v))))
            }

            IrExprKind::OptionSome { inner } => {
                let v = match self.eval_expr(inner).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(ExprFlow::Propagate(v)),
                };
                Ok(ExprFlow::Value(Value::OptionSome(BoxedValue::new(v))))
            }

            IrExprKind::OptionNone => Ok(ExprFlow::Value(Value::OptionNone)),

            IrExprKind::TryPropagate { inner } => {
                let inner = match self.eval_expr(inner).await? {
                    ExprFlow::Value(v) => v,
                    ExprFlow::Propagate(v) => return Ok(ExprFlow::Propagate(v)),
                };
                match inner {
                    Value::ResultOk(v) => Ok(ExprFlow::Value(v.get())),
                    Value::ResultErr(v) => Ok(ExprFlow::Propagate(Value::ResultErr(v))),
                    Value::OptionSome(v) => Ok(ExprFlow::Value(v.get())),
                    Value::OptionNone => Ok(ExprFlow::Propagate(Value::OptionNone)),
                    other => Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Result or Option".into(),
                            got: other.type_name(),
                        },
                        expr.span,
                    )),
                }
            }

            IrExprKind::TryRetry {
                body,
                attempts,
                backoff: _,
            } => {
                let total = (*attempts).max(1);
                let mut last_runtime_error: Option<InterpError> = None;
                let mut last_result_err: Option<Value> = None;
                let mut last_stream_start_err: Option<InterpError> = None;
                let mut last_stream_start_chunk: Option<StreamChunk> = None;
                let mut saw_option_retry = false;
                for _ in 0..total {
                    match self.eval_expr(body).await {
                        Ok(ExprFlow::Value(Value::Stream(stream))) => {
                            match stream.next_chunk().await {
                                Some(Ok(chunk)) if stream_start_is_retryable(&chunk.value) => {
                                    if matches!(chunk.value, Value::OptionNone) {
                                        saw_option_retry = true;
                                    } else {
                                        last_stream_start_chunk = Some(chunk);
                                    }
                                }
                                Some(Ok(chunk)) => {
                                    let combined = self.prepend_stream_chunk(chunk, stream);
                                    return Ok(ExprFlow::Value(combined));
                                }
                                Some(Err(err)) => {
                                    last_stream_start_err = Some(err);
                                }
                                None => return Ok(ExprFlow::Value(Value::Stream(stream))),
                            }
                        }
                        Ok(ExprFlow::Value(Value::ResultErr(err))) => {
                            last_result_err = Some(Value::ResultErr(err));
                        }
                        Ok(ExprFlow::Value(Value::OptionNone)) => {
                            saw_option_retry = true;
                        }
                        Ok(ExprFlow::Value(v)) => return Ok(ExprFlow::Value(v)),
                        Ok(ExprFlow::Propagate(v)) => return Ok(ExprFlow::Propagate(v)),
                        Err(err) => last_runtime_error = Some(err),
                    }
                }
                if let Some(v) = last_result_err {
                    Ok(ExprFlow::Value(v))
                } else if let Some(chunk) = last_stream_start_chunk {
                    Ok(ExprFlow::Value(
                        self.singleton_stream(chunk, default_stream_backpressure())
                            .await?,
                    ))
                } else if saw_option_retry {
                    Ok(ExprFlow::Value(Value::OptionNone))
                } else if let Some(err) = last_stream_start_err {
                    Ok(ExprFlow::Value(
                        self.singleton_stream_error(err, default_stream_backpressure())
                            .await?,
                    ))
                } else if let Some(err) = last_runtime_error {
                    Err(err)
                } else {
                    Ok(ExprFlow::Value(Value::Nothing))
                }
            }

            IrExprKind::Replay {
                trace,
                arms,
                else_body,
            } => {
                self.eval_replay_expr(trace, arms, else_body, expr.span)
                    .await
            }
        }
    }

    /// Apply a closure (slice 45j): install its captured
    /// environment, bind parameters, evaluate the body, restore the
    /// caller's environment (also on error). `?` propagation inside
    /// a lambda body is rejected loudly — the closure boundary is
    /// not a Result-returning function.
    #[async_recursion]
    async fn apply_closure(
        &mut self,
        closure: &ClosureValue,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, InterpError> {
        if args.len() != closure.arity() {
            return Err(InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "closure expects {} argument(s), got {}",
                    closure.arity(),
                    args.len()
                )),
                span,
            ));
        }
        let mut call_env = Env::new();
        for (lid, v) in closure.env_cloned() {
            call_env.bind(lid, v);
        }
        for ((lid, _), v) in closure.params().iter().zip(args) {
            call_env.bind(*lid, v);
        }
        let saved = std::mem::replace(&mut self.env, call_env);
        let result = self.eval_expr(closure.body()).await;
        self.env = saved;
        match result?.into_value() {
            Ok(v) => Ok(v),
            Err(_) => Err(InterpError::new(
                InterpErrorKind::NotImplemented(
                    "`?` propagation inside a lambda body (return the Result and branch at the call site instead)"
                        .into(),
                ),
                span,
            )),
        }
    }

    /// The lambda-taking `List` methods (slice 45j): `map`,
    /// `filter`, `fold`, `any`, `all`. Applies the closure once per
    /// element, left to right; `any`/`all` short-circuit.
    async fn eval_higher_order_list_method(
        &mut self,
        kind: corvid_types::BuiltinMethodKind,
        recv: Value,
        mut args: Vec<Value>,
        span: Span,
    ) -> Result<Value, InterpError> {
        use corvid_types::BuiltinMethodKind as Bmk;
        // Result.map_err (45l): the one lambda-taking method whose
        // receiver is not a list. Ok passes through untouched; the
        // closure runs only on the Err side.
        if kind == Bmk::ResultMapErr {
            let f = match args.remove(0) {
                Value::Closure(c) => c,
                other => {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Function".into(),
                            got: other.type_name(),
                        },
                        span,
                    ))
                }
            };
            return match recv {
                Value::ResultOk(v) => Ok(Value::ResultOk(v)),
                Value::ResultErr(e) => {
                    let mapped = self.apply_closure(&f, vec![e.get()], span).await?;
                    Ok(Value::ResultErr(BoxedValue::new(mapped)))
                }
                other => Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "Result".into(),
                        got: other.type_name(),
                    },
                    span,
                )),
            };
        }
        let Value::List(list) = &recv else {
            return Err(InterpError::new(
                InterpErrorKind::TypeMismatch {
                    expected: "List".into(),
                    got: recv.type_name(),
                },
                span,
            ));
        };
        let items = list.iter_cloned();
        let want_closure = |v: Value, span: Span| -> Result<ClosureValue, InterpError> {
            match v {
                Value::Closure(c) => Ok(c),
                other => Err(InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "Function".into(),
                        got: other.type_name(),
                    },
                    span,
                )),
            }
        };
        match kind {
            Bmk::ListMap => {
                let f = want_closure(args.remove(0), span)?;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.apply_closure(&f, vec![item], span).await?);
                }
                Ok(Value::List(ListValue::new(out)))
            }
            Bmk::ListFilter => {
                let f = want_closure(args.remove(0), span)?;
                let mut out = Vec::new();
                for item in items {
                    match self.apply_closure(&f, vec![item.clone()], span).await? {
                        Value::Bool(true) => out.push(item),
                        Value::Bool(false) => {}
                        other => {
                            return Err(InterpError::new(
                                InterpErrorKind::TypeMismatch {
                                    expected: "Bool (from the filter predicate)".into(),
                                    got: other.type_name(),
                                },
                                span,
                            ));
                        }
                    }
                }
                Ok(Value::List(ListValue::new(out)))
            }
            Bmk::ListFold => {
                let mut acc = args.remove(0);
                let f = want_closure(args.remove(0), span)?;
                for item in items {
                    acc = self.apply_closure(&f, vec![acc, item], span).await?;
                }
                Ok(acc)
            }
            Bmk::ListAny | Bmk::ListAll => {
                let f = want_closure(args.remove(0), span)?;
                let is_all = kind == Bmk::ListAll;
                for item in items {
                    match self.apply_closure(&f, vec![item], span).await? {
                        Value::Bool(b) => {
                            if b != is_all {
                                // any: first true wins; all: first
                                // false loses. Short-circuit.
                                return Ok(Value::Bool(!is_all));
                            }
                        }
                        other => {
                            return Err(InterpError::new(
                                InterpErrorKind::TypeMismatch {
                                    expected: "Bool (from the predicate)".into(),
                                    got: other.type_name(),
                                },
                                span,
                            ));
                        }
                    }
                }
                Ok(Value::Bool(is_all))
            }
            other => Err(InterpError::new(
                InterpErrorKind::DispatchFailed(format!(
                    "{other:?} is not a higher-order list method"
                )),
                span,
            )),
        }
    }

    /// Dispatch a call expression. Routes Tool / Prompt / Agent through
    /// the right runtime path; an `Unknown` kind is a hard error
    /// (typecheck should have caught it).
    async fn eval_call(
        &mut self,
        kind: &IrCallKind,
        callee_name: &str,
        args: &[IrExpr],
        result_ty: &Type,
        span: Span,
    ) -> Result<ExprFlow, InterpError> {
        // Evaluate args eagerly (left to right) before any external call.
        let mut arg_values = Vec::with_capacity(args.len());
        for a in args {
            match self.eval_expr(a).await?.into_value() {
                Ok(v) => arg_values.push(v),
                Err(v) => return Ok(ExprFlow::Propagate(v)),
            }
        }

        match kind {
            IrCallKind::Tool { def_id, .. } => {
                let tool = self.tools_by_id.get(def_id).copied().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "tool `{callee_name}` is missing from the IR"
                        )),
                        span,
                    )
                })?;

                let json_args: Vec<serde_json::Value> =
                    arg_values.iter().map(value_to_json).collect();

                // Runtime confidence gate: if the tool has
                // `trust: autonomous_if_confident(T)` in its declared
                // effects, check that composed input confidence >= T.
                // If below, activate the same approval path used by
                // explicit `approve` statements before dispatching the
                // tool.
                let input_confidence = composed_confidence(&arg_values);
                let confidence_gate = tool.confidence_gate.map(|threshold| ConfidenceGateStep {
                    threshold,
                    actual: input_confidence,
                    triggered: input_confidence < threshold,
                });
                if let Some(gate) = confidence_gate {
                    if gate.triggered {
                        let label = format!("ConfidenceGate:{callee_name}");
                        if self.should_yield_boundary() {
                            let action = self
                                .maybe_yield(StepEvent::BeforeApproval {
                                    label: label.clone(),
                                    args: json_args.clone(),
                                    confidence_gate: Some(gate),
                                    span,
                                    env: self.env_snapshot(),
                                })
                                .await?;
                            match action {
                                StepAction::Approve => {
                                    self.maybe_yield(StepEvent::AfterApproval {
                                        label,
                                        approved: true,
                                        span,
                                    })
                                    .await?;
                                }
                                StepAction::Deny => {
                                    self.maybe_yield(StepEvent::AfterApproval {
                                        label: label.clone(),
                                        approved: false,
                                        span,
                                    })
                                    .await?;
                                    return Err(InterpError::new(
                                        InterpErrorKind::Runtime(
                                            corvid_runtime::RuntimeError::ApprovalDenied {
                                                action: label,
                                            },
                                        ),
                                        span,
                                    ));
                                }
                                _ => {
                                    let result =
                                        self.runtime.approval_gate(&label, json_args.clone()).await;
                                    let approved = result.is_ok();
                                    self.maybe_yield(StepEvent::AfterApproval {
                                        label,
                                        approved,
                                        span,
                                    })
                                    .await?;
                                    result.map_err(|e| {
                                        InterpError::new(InterpErrorKind::Runtime(e), span)
                                    })?;
                                }
                            }
                        } else {
                            self.runtime
                                .approval_gate(&label, json_args.clone())
                                .await
                                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
                        }
                    }
                }
                let is_grounded = tool_has_retrieval_effect(tool);
                let result_decode_ty = match (&tool.return_ty, is_grounded) {
                    (Type::Grounded(inner), true) => inner.as_ref(),
                    _ => &tool.return_ty,
                };

                if self.should_yield_boundary() {
                    let action = self
                        .maybe_yield(StepEvent::BeforeToolCall {
                            tool_name: callee_name.to_string(),
                            args: json_args.clone(),
                            input_confidence,
                            confidence_gate,
                            span,
                            env: self.env_snapshot(),
                        })
                        .await?;
                    if let StepAction::Override(val) = action {
                        let value = json_to_value(val, result_decode_ty, &self.types_by_id)
                            .map_err(|e| {
                                InterpError::new(
                                    InterpErrorKind::Marshal(format!(
                                        "tool `{callee_name}` override: {e}"
                                    )),
                                    span,
                                )
                            })?;
                        return Ok(ExprFlow::Value(maybe_ground_tool_result(
                            tool,
                            callee_name,
                            value,
                        )));
                    }
                }

                let start = std::time::Instant::now();
                let result_value = if let Some(mock) = self.mock_tools.get(def_id).copied() {
                    let mut sub = Interpreter::new(self.ir, self.runtime).with_mocks();
                    sub.bind_ir_params(callee_name, &mock.params, arg_values, span)?;
                    sub.eval_block(&mock.body)
                        .await
                        .and_then(|flow| match flow {
                            Flow::Return(value) => Ok(value),
                            Flow::Normal => Ok(Value::Nothing),
                            Flow::Break | Flow::Continue => Err(InterpError::new(
                                InterpErrorKind::Other(
                                    "loop control flow escaped mock body".into(),
                                ),
                                mock.span,
                            )),
                        })?
                } else if is_stdlib_db_tool(callee_name) {
                    // Phase 33S3b — typed-Value dispatch for the
                    // executing SQLite stdlib tools. `db_open`
                    // returns `Value::DbHandle(Arc<...>)` directly
                    // (JSON cannot carry the opaque handle);
                    // `db_query` / `db_execute` extract the
                    // `Arc<DbHandleInner>` from the first arg's
                    // `Value::DbHandle` and pass it to the runtime
                    // alongside JSON-marshalled SQL + params, then
                    // convert the JSON-shaped result envelope back
                    // through `json_to_value`. The opacity gate in
                    // `conv.rs` ensures that user tools NOT in the
                    // stdlib set cannot mint handles through JSON.
                    dispatch_stdlib_db_tool(
                        self.runtime,
                        callee_name,
                        &arg_values,
                        result_decode_ty,
                        &self.types_by_id,
                        span,
                    )
                    .await?
                } else if is_stdlib_json_tool(callee_name) {
                    // Phase 33R5b-a — typed-Value dispatch for the
                    // executing JSON stdlib tools. `json_parse`
                    // returns `Value::JsonValue(Arc<...>)` wrapped
                    // in `Value::ResultOk` / `Value::ResultErr`;
                    // `json_get_*` extract the Arc and return
                    // typed Result values; `json_object_new`
                    // returns `Value::JsonBuilder(Arc<Mutex<...>>)`;
                    // `json_object_set_*` mutate and return the
                    // same builder; `json_object_finish` returns
                    // a `String` snapshot.
                    dispatch_stdlib_json_tool(
                        self.runtime,
                        callee_name,
                        &arg_values,
                        span,
                    )
                    .await?
                } else if is_typed_json_decoder_tool_call(callee_name, result_decode_ty) {
                    // Phase 33R5b-b — typed-decoder convention.
                    // User declares a tool with signature
                    // `tool decode_X_from_json(text: String) ->
                    // Result<X, String>` and the runtime decodes
                    // the text into the target type X via
                    // `serde_json::from_str` + `json_to_value`
                    // against the IR type table. This is the
                    // Corvid-idiomatic shape — the typechecker
                    // enforces the target type at compile time,
                    // the runtime handles the dispatch.
                    dispatch_typed_json_decoder(
                        &arg_values,
                        result_decode_ty,
                        &self.types_by_id,
                        callee_name,
                        span,
                    )?
                } else {
                    let result = self
                        .runtime
                        .call_tool(callee_name, json_args)
                        .await
                        .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
                    json_to_value(result, result_decode_ty, &self.types_by_id).map_err(|e| {
                        InterpError::new(
                            InterpErrorKind::Marshal(format!("tool `{callee_name}`: {e}")),
                            span,
                        )
                    })?
                };
                let elapsed_ms = start.elapsed().as_millis() as u64;
                let result = value_to_json(&result_value);

                if self.should_yield_boundary() {
                    let action = self
                        .maybe_yield(StepEvent::AfterToolCall {
                            tool_name: callee_name.to_string(),
                            result: result.clone(),
                            result_confidence: 1.0,
                            elapsed_ms,
                            span,
                        })
                        .await?;
                    if let StepAction::Override(val) = action {
                        let value = json_to_value(val, result_decode_ty, &self.types_by_id)
                            .map_err(|e| {
                                InterpError::new(
                                    InterpErrorKind::Marshal(format!(
                                        "tool `{callee_name}` override: {e}"
                                    )),
                                    span,
                                )
                            })?;
                        return Ok(ExprFlow::Value(maybe_ground_tool_result(
                            tool,
                            callee_name,
                            value,
                        )));
                    }
                }

                // If the tool has a `retrieval` effect (data: grounded),
                // wrap the result in Grounded with a provenance chain.
                Ok(ExprFlow::Value(maybe_ground_tool_result(
                    tool,
                    callee_name,
                    result_value,
                )))
            }
            IrCallKind::Prompt { def_id } => {
                self.dispatch_prompt_expr(*def_id, callee_name, &arg_values, span)
                    .await
            }
            IrCallKind::Agent { def_id } => {
                let agent = self.agents_by_id.get(def_id).copied().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "agent `{callee_name}` is missing from the IR"
                        )),
                        span,
                    )
                })?;

                if self.should_yield_boundary() {
                    let json_args: Vec<serde_json::Value> =
                        arg_values.iter().map(value_to_json).collect();
                    self.maybe_yield(StepEvent::BeforeAgentCall {
                        agent_name: callee_name.to_string(),
                        args: json_args,
                        input_confidence: composed_confidence(&arg_values),
                        span,
                    })
                    .await?;
                }

                let mut sub = Interpreter::new(self.ir, self.runtime);
                sub.mock_tools = self.mock_tools.clone();
                // Propagate the step controller into sub-agent calls so
                // step-through continues across agent boundaries.
                if let Some(ref stepper) = self.stepper {
                    sub.stepper = Some(StepController::new(
                        Arc::clone(&stepper.hook_ref()),
                        stepper.mode,
                    ));
                }
                sub.bind_params(agent, arg_values)?;
                let result = sub.run_body(agent).await.map(ExprFlow::Value);

                if self.should_yield_boundary() {
                    let result_json = match &result {
                        Ok(ExprFlow::Value(v)) => value_to_json(v),
                        _ => serde_json::Value::Null,
                    };
                    self.maybe_yield(StepEvent::AfterAgentCall {
                        agent_name: callee_name.to_string(),
                        result: result_json,
                        result_confidence: result
                            .as_ref()
                            .ok()
                            .and_then(|flow| match flow {
                                ExprFlow::Value(value) => Some(value_confidence(value)),
                                ExprFlow::Propagate(value) => Some(value_confidence(value)),
                            })
                            .unwrap_or(1.0),
                        span,
                    })
                    .await?;
                }

                result
            }
            IrCallKind::Fixture { def_id } => {
                let fixture = self.fixtures_by_id.get(def_id).copied().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "fixture `{callee_name}` is missing from the IR"
                        )),
                        span,
                    )
                })?;
                let mut sub = Interpreter::new(self.ir, self.runtime).with_mocks();
                sub.bind_ir_params(callee_name, &fixture.params, arg_values, span)?;
                sub.eval_block(&fixture.body)
                    .await
                    .and_then(|flow| match flow {
                        Flow::Return(value) => Ok(ExprFlow::Value(value)),
                        Flow::Normal => Ok(ExprFlow::Value(Value::Nothing)),
                        Flow::Break | Flow::Continue => Err(InterpError::new(
                            InterpErrorKind::Other("loop control flow escaped fixture body".into()),
                            fixture.span,
                        )),
                    })
            }
            // Sum-variant construction (45h): positional payload,
            // variant metadata from the owning IrType.
            IrCallKind::EnumConstructor {
                def_id,
                variant_index,
            } => {
                let ir_type = self.types_by_id.get(def_id).copied().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "sum type for variant `{callee_name}` is missing from the IR"
                        )),
                        span,
                    )
                })?;
                let variant = ir_type
                    .variants
                    .get(*variant_index as usize)
                    .ok_or_else(|| {
                        InterpError::new(
                            InterpErrorKind::DispatchFailed(format!(
                                "variant index {variant_index} out of range for `{}`",
                                ir_type.name
                            )),
                            span,
                        )
                    })?;
                if arg_values.len() != variant.fields.len() {
                    return Err(InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "variant `{callee_name}` expects {} field(s), got {}",
                            variant.fields.len(),
                            arg_values.len(),
                        )),
                        span,
                    ));
                }
                Ok(ExprFlow::Value(Value::Enum(crate::value::EnumValue::new(
                    ir_type.id,
                    ir_type.name.clone(),
                    *variant_index,
                    variant.name.clone(),
                    arg_values,
                ))))
            }
            IrCallKind::StructConstructor { def_id } => {
                // Build a `Value::Struct` from the constructor args, in
                // field declaration order (mirrors the codegen-cl
                // lowering's store-at-offset pattern).
                let ir_type = self.types_by_id.get(def_id).copied().ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "struct type `{callee_name}` is missing from the IR"
                        )),
                        span,
                    )
                })?;
                if arg_values.len() != ir_type.fields.len() {
                    return Err(InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "struct constructor `{callee_name}` expects {} field(s), got {}",
                            ir_type.fields.len(),
                            arg_values.len(),
                        )),
                        span,
                    ));
                }
                let fields: Vec<(String, Value)> = ir_type
                    .fields
                    .iter()
                    .zip(arg_values.into_iter())
                    .map(|(f, v)| (f.name.clone(), v))
                    .collect();
                Ok(ExprFlow::Value(Value::Struct(
                    crate::value::StructValue::new(ir_type.id, ir_type.name.clone(), fields),
                )))
            }
            // Closure call (slice 45j): `f(x)` where `f` is a
            // function-typed local holding a closure value.
            IrCallKind::ClosureLocal { local_id } => {
                let callee = self.env.lookup(*local_id).ok_or_else(|| {
                    InterpError::new(InterpErrorKind::UndefinedLocal(*local_id), span)
                })?;
                let Value::Closure(closure) = callee else {
                    return Err(InterpError::new(
                        InterpErrorKind::DispatchFailed(format!(
                            "`{callee_name}` is not a function (runtime type `{}`)",
                            callee.type_name()
                        )),
                        span,
                    ));
                };
                let result = self.apply_closure(&closure, arg_values, span).await?;
                Ok(ExprFlow::Value(result))
            }
            IrCallKind::Unknown => {
                let _ = result_ty;
                Err(InterpError::new(
                    InterpErrorKind::DispatchFailed(format!(
                        "call to `{callee_name}` did not resolve to a tool, prompt, or agent"
                    )),
                    span,
                ))
            }
        }
    }
}

// ============================================================================
// Phase 33S3b — typed-Value dispatch for the executing SQLite
// stdlib tools (`db_open` / `db_query` / `db_execute`).
//
// These three tools cannot round-trip through the JSON
// `runtime.call_tool` path because their signatures carry
// `Value::DbHandle(Arc<DbHandleInner>)` — the opaque, refcounted
// handle whose underlying connection lives in the runtime's
// `DbHandleRegistry`. The opacity gate in `conv.rs`
// (`json_to_value` refusing `Type::DbHandle`) is the load-bearing
// security property; this dispatch helper is the trusted path
// that bypasses the gate for stdlib calls only.
//
// `is_stdlib_db_tool` is the exact-name gate (mirrors
// `is_stdlib_io_tool` / `is_stdlib_http_tool` in
// `corvid-runtime`). User-defined tools whose names happen to
// start with `db_` fall through to the normal JSON dispatch path.
// ============================================================================

fn is_stdlib_db_tool(name: &str) -> bool {
    matches!(name, "db_open" | "db_query" | "db_execute")
}

async fn dispatch_stdlib_db_tool(
    runtime: &Runtime,
    callee_name: &str,
    arg_values: &[Value],
    result_decode_ty: &Type,
    types_by_id: &HashMap<DefId, &corvid_ir::IrType>,
    span: Span,
) -> Result<Value, InterpError> {
    match callee_name {
        "db_open" => {
            let path = match arg_values.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => {
                    return Err(InterpError::new(
                        InterpErrorKind::Other(
                            "db_open expected one String path argument".into(),
                        ),
                        span,
                    ))
                }
            };
            let handle = runtime
                .db_open_tool(path)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::DbHandle(handle))
        }
        "db_query" | "db_execute" => {
            let handle = match arg_values.first() {
                Some(Value::DbHandle(arc)) => arc.clone(),
                _ => {
                    return Err(InterpError::new(
                        InterpErrorKind::Other(format!(
                            "{callee_name} expected its first argument to be a DbHandle \
                             (only `db_open` mints valid handles)"
                        )),
                        span,
                    ))
                }
            };
            let sql = match arg_values.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => {
                    return Err(InterpError::new(
                        InterpErrorKind::Other(format!(
                            "{callee_name} expected its second argument to be a String SQL \
                             statement"
                        )),
                        span,
                    ))
                }
            };
            let params = extract_db_params(arg_values.get(2), callee_name, span)?;
            let result_json = if callee_name == "db_query" {
                runtime
                    .db_query_tool(&handle, sql, params)
                    .await
                    .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?
            } else {
                runtime
                    .db_execute_tool(&handle, sql, params)
                    .await
                    .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?
            };
            json_to_value(result_json, result_decode_ty, types_by_id).map_err(|e| {
                InterpError::new(
                    InterpErrorKind::Marshal(format!("tool `{callee_name}`: {e}")),
                    span,
                )
            })
        }
        other => Err(InterpError::new(
            InterpErrorKind::DispatchFailed(format!(
                "stdlib db dispatch reached unknown name `{other}` — gate-keeper drift"
            )),
            span,
        )),
    }
}

/// Phase 33S3b — convert a `List<DbParam>` Corvid value into the
/// typed `Vec<DbValue>` the runtime's `DbHandleRegistry::query` /
/// `execute` expects. Each `DbParam` carries a `value_kind`
/// discriminator naming which value field is valid; the typed
/// `db_param_int` / `db_param_text` / `db_param_float` /
/// `db_param_bool` / `db_param_null` constructors in `std/db.cor`
/// set the discriminator + the relevant value field.
///
/// This is where parameter binding stays injection-safe: a
/// `DbParam` whose `string_value` carries SQL syntax
/// (`"'; DROP TABLE users; --"`) is converted to
/// `DbValue::Text(...)` and threaded through
/// `rusqlite::params_from_iter` — never interpolated into the
/// SQL string. The 33S3b plumbing test
/// `db_param_text_with_sql_metacharacters_is_bound_as_data`
/// pins this property.
fn extract_db_params(
    arg: Option<&Value>,
    callee_name: &str,
    span: Span,
) -> Result<Vec<DbValue>, InterpError> {
    let Some(Value::List(items)) = arg else {
        return Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee_name} expected its third argument to be a List<DbParam>"
            )),
            span,
        ));
    };
    let mut out = Vec::new();
    for item in items.iter_cloned().into_iter() {
        let Value::Struct(s) = item else {
            return Err(InterpError::new(
                InterpErrorKind::Other(format!(
                    "{callee_name} list element is not a DbParam struct"
                )),
                span,
            ));
        };
        let value = s.with_fields(|fields| {
            let value_kind = match fields.get("value_kind") {
                Some(Value::String(s)) => s.to_string(),
                _ => "Null".to_string(),
            };
            match value_kind.as_str() {
                "Int" => match fields.get("int_value") {
                    Some(Value::Int(n)) => DbValue::Integer(*n),
                    _ => DbValue::Null,
                },
                "Float" => match fields.get("float_value") {
                    Some(Value::Float(f)) => DbValue::Float(*f),
                    _ => DbValue::Null,
                },
                "String" => match fields.get("string_value") {
                    Some(Value::String(s)) => DbValue::Text(s.to_string()),
                    _ => DbValue::Null,
                },
                "Bool" => match fields.get("bool_value") {
                    Some(Value::Bool(b)) => DbValue::Bool(*b),
                    _ => DbValue::Null,
                },
                _ => DbValue::Null,
            }
        });
        out.push(value);
    }
    Ok(out)
}

// ============================================================================
// Phase 33R5b-a — typed-Value dispatch for the executing JSON
// stdlib tools (`json_parse` / `json_get_*` / `json_object_new`
// / `json_object_set_*` / `json_object_finish`).
//
// `is_stdlib_json_tool` is the exact-name gate (mirrors
// `is_stdlib_io_tool` / `is_stdlib_http_tool` / `is_stdlib_db_tool`).
// User-defined tools whose names happen to start with `json_`
// fall through to the normal JSON dispatch path.
// ============================================================================

fn is_stdlib_json_tool(name: &str) -> bool {
    matches!(
        name,
        "json_parse"
            | "json_get_int"
            | "json_get_float"
            | "json_get_string"
            | "json_get_bool"
            | "json_get_object"
            | "json_get_array"
            | "json_object_new"
            | "json_object_set_int"
            | "json_object_set_float"
            | "json_object_set_string"
            | "json_object_set_bool"
            | "json_object_finish"
    )
}

async fn dispatch_stdlib_json_tool(
    runtime: &Runtime,
    callee_name: &str,
    arg_values: &[Value],
    span: Span,
) -> Result<Value, InterpError> {
    match callee_name {
        "json_parse" => {
            let text = expect_string_arg(arg_values, 0, callee_name, span)?;
            let result = runtime
                .json_parse_tool(text)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_arc_json(result))
        }
        "json_get_int" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_int_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_int(result))
        }
        "json_get_float" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_float_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_float(result))
        }
        "json_get_string" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_string_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_string(result))
        }
        "json_get_bool" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_bool_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_bool(result))
        }
        "json_get_object" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_object_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_arc_json(result))
        }
        "json_get_array" => {
            let handle = expect_json_value_arg(arg_values, 0, callee_name, span)?;
            let field = expect_string_arg(arg_values, 1, callee_name, span)?;
            let result = runtime
                .json_get_array_tool(&handle, field)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(wrap_result_array(result))
        }
        "json_object_new" => {
            let builder = runtime
                .json_object_new_tool()
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::JsonBuilder(builder))
        }
        "json_object_set_int" => {
            let builder = expect_json_builder_arg(arg_values, 0, callee_name, span)?;
            let key = expect_string_arg(arg_values, 1, callee_name, span)?;
            let value = expect_int_arg(arg_values, 2, callee_name, span)?;
            let result = runtime
                .json_object_set_int_tool(builder, key, value)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::JsonBuilder(result))
        }
        "json_object_set_float" => {
            let builder = expect_json_builder_arg(arg_values, 0, callee_name, span)?;
            let key = expect_string_arg(arg_values, 1, callee_name, span)?;
            let value = expect_float_arg(arg_values, 2, callee_name, span)?;
            let result = runtime
                .json_object_set_float_tool(builder, key, value)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::JsonBuilder(result))
        }
        "json_object_set_string" => {
            let builder = expect_json_builder_arg(arg_values, 0, callee_name, span)?;
            let key = expect_string_arg(arg_values, 1, callee_name, span)?;
            let value = expect_string_arg(arg_values, 2, callee_name, span)?;
            let result = runtime
                .json_object_set_string_tool(builder, key, value)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::JsonBuilder(result))
        }
        "json_object_set_bool" => {
            let builder = expect_json_builder_arg(arg_values, 0, callee_name, span)?;
            let key = expect_string_arg(arg_values, 1, callee_name, span)?;
            let value = expect_bool_arg(arg_values, 2, callee_name, span)?;
            let result = runtime
                .json_object_set_bool_tool(builder, key, value)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::JsonBuilder(result))
        }
        "json_object_finish" => {
            let builder = expect_json_builder_arg(arg_values, 0, callee_name, span)?;
            let result = runtime
                .json_object_finish_tool(&builder)
                .await
                .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
            Ok(Value::String(Arc::from(result.as_str())))
        }
        other => Err(InterpError::new(
            InterpErrorKind::DispatchFailed(format!(
                "stdlib json dispatch reached unknown name `{other}` — gate-keeper drift"
            )),
            span,
        )),
    }
}

fn expect_string_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<String, InterpError> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be a String"
            )),
            span,
        )),
    }
}

fn expect_int_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<i64, InterpError> {
    match args.get(index) {
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be an Int"
            )),
            span,
        )),
    }
}

fn expect_float_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<f64, InterpError> {
    match args.get(index) {
        Some(Value::Float(f)) => Ok(*f),
        Some(Value::Int(n)) => Ok(*n as f64),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be a Float"
            )),
            span,
        )),
    }
}

fn expect_bool_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<bool, InterpError> {
    match args.get(index) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be a Bool"
            )),
            span,
        )),
    }
}

fn expect_json_value_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<Arc<serde_json::Value>, InterpError> {
    match args.get(index) {
        Some(Value::JsonValue(arc)) => Ok(arc.clone()),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be a JsonValue (only `json_parse` mints these)"
            )),
            span,
        )),
    }
}

fn expect_json_builder_arg(
    args: &[Value],
    index: usize,
    callee: &str,
    span: Span,
) -> Result<
    Arc<std::sync::Mutex<serde_json::Map<String, serde_json::Value>>>,
    InterpError,
> {
    match args.get(index) {
        Some(Value::JsonBuilder(arc)) => Ok(arc.clone()),
        _ => Err(InterpError::new(
            InterpErrorKind::Other(format!(
                "{callee} expected argument {index} to be a JsonBuilder (only `json_object_new` mints these)"
            )),
            span,
        )),
    }
}

/// Wrap a `Result<Arc<serde_json::Value>, String>` into a Corvid
/// `Result<JsonValue, String>` Value. The Ok branch becomes a
/// `Value::ResultOk(BoxedValue(Value::JsonValue(arc)))`, the Err
/// branch becomes `Value::ResultErr(BoxedValue(Value::String))`.
fn wrap_result_arc_json(
    result: Result<Arc<serde_json::Value>, String>,
) -> Value {
    match result {
        Ok(arc) => Value::ResultOk(BoxedValue::new(Value::JsonValue(arc))),
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

fn wrap_result_int(result: Result<i64, String>) -> Value {
    match result {
        Ok(n) => Value::ResultOk(BoxedValue::new(Value::Int(n))),
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

fn wrap_result_float(result: Result<f64, String>) -> Value {
    match result {
        Ok(f) => Value::ResultOk(BoxedValue::new(Value::Float(f))),
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

fn wrap_result_string(result: Result<String, String>) -> Value {
    match result {
        Ok(s) => Value::ResultOk(BoxedValue::new(Value::String(Arc::from(s.as_str())))),
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

fn wrap_result_bool(result: Result<bool, String>) -> Value {
    match result {
        Ok(b) => Value::ResultOk(BoxedValue::new(Value::Bool(b))),
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

fn wrap_result_array(
    result: Result<Vec<Arc<serde_json::Value>>, String>,
) -> Value {
    match result {
        Ok(arcs) => {
            let list_items: Vec<Value> =
                arcs.into_iter().map(Value::JsonValue).collect();
            Value::ResultOk(BoxedValue::new(Value::List(ListValue::new(list_items))))
        }
        Err(msg) => Value::ResultErr(BoxedValue::new(Value::String(Arc::from(msg.as_str())))),
    }
}

// ============================================================================
// Phase 33R5b-b — typed-decoder convention.
//
// When a user declares a tool with the signature
//   tool decode_<X>_from_json(text: String) -> Result<X, String>
// where X is any Corvid type the runtime can convert from JSON
// (a user-declared struct, a primitive, a list, etc.), the
// interpreter intercepts the call and dispatches a generic
// JSON decode against the declared target type — no per-type
// runtime handler needed.
//
// The convention is keyed on TWO conditions simultaneously:
//
//   1. The tool name MATCHES the pattern `decode_*_from_json`
//      (where * is non-empty).
//   2. The declared return type MATCHES `Result<T, String>` for
//      some T (the typechecker enforces this; we re-check
//      structurally before dispatching).
//
// Both conditions together prevent the dispatch from silently
// intercepting an unrelated user tool that happens to have one
// or the other property.
// ============================================================================

fn is_typed_json_decoder_tool_call(callee_name: &str, result_decode_ty: &Type) -> bool {
    // Name pattern: decode_<X>_from_json where <X> is non-empty.
    let Some(rest) = callee_name.strip_prefix("decode_") else {
        return false;
    };
    let Some(target) = rest.strip_suffix("_from_json") else {
        return false;
    };
    if target.is_empty() {
        return false;
    }
    // Return type pattern: Result<T, String> for some T.
    matches!(result_decode_ty, Type::Result(_ok, err) if matches!(**err, Type::String))
}

fn dispatch_typed_json_decoder(
    arg_values: &[Value],
    result_decode_ty: &Type,
    types_by_id: &HashMap<DefId, &corvid_ir::IrType>,
    callee_name: &str,
    span: Span,
) -> Result<Value, InterpError> {
    let text = match arg_values.first() {
        Some(Value::String(s)) => s.to_string(),
        _ => {
            return Err(InterpError::new(
                InterpErrorKind::Other(format!(
                    "{callee_name} expected a single String argument (the JSON text)"
                )),
                span,
            ))
        }
    };
    let (ok_ty, _err_ty) = match result_decode_ty {
        Type::Result(ok, err) => (ok.as_ref(), err.as_ref()),
        _ => {
            return Err(InterpError::new(
                InterpErrorKind::Other(format!(
                    "{callee_name} expected a Result<T, String> return type; got {}",
                    result_decode_ty.display_name()
                )),
                span,
            ))
        }
    };

    // Phase 33R5b-b parse path. Serde failure is the load-bearing
    // recoverable-error property — we wrap the diagnostic as
    // `Result::Err(message)` so user code can pattern-match and
    // route the error up to its caller. No panic; no escape.
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(err) => {
            return Ok(Value::ResultErr(BoxedValue::new(Value::String(Arc::from(
                format!("malformed JSON in `{callee_name}`: {err}").as_str(),
            )))));
        }
    };

    // Convert the parsed JSON to a typed Value against the
    // user-declared target type. `json_to_value` is the same
    // path the io / http / db dispatch surfaces use to convert
    // JSON envelopes back into typed values — it handles
    // structs, lists, options, results, etc.
    match json_to_value(parsed, ok_ty, types_by_id) {
        Ok(value) => Ok(Value::ResultOk(BoxedValue::new(value))),
        Err(err) => {
            // A type-shape mismatch (e.g. JSON has a String where
            // the user declared an Int) is the SECOND load-bearing
            // recoverable-error path. The typechecker can't enforce
            // this at compile time because the JSON shape is dynamic;
            // the runtime catches it and surfaces it through the
            // Result::Err branch.
            Ok(Value::ResultErr(BoxedValue::new(Value::String(Arc::from(
                format!("JSON shape mismatch in `{callee_name}`: {err}").as_str(),
            )))))
        }
    }
}


/// Try to match a lowered pattern against a value (slice 45i).
/// Collects bindings without touching the environment so a failed
/// sibling subpattern can't leave partial state; the caller applies
/// them on success.
pub(crate) fn pattern_matches(
    pattern: &IrPattern,
    value: &Value,
    bindings: &mut Vec<(corvid_resolve::LocalId, String, Value)>,
) -> bool {
    match pattern {
        IrPattern::Wildcard => true,
        IrPattern::Literal(lit) => {
            let lit_v = eval_literal(lit);
            lit_v == *value
        }
        IrPattern::Bind { local_id, name } => {
            bindings.push((*local_id, name.clone(), value.clone()));
            true
        }
        IrPattern::At {
            local_id,
            name,
            inner,
        } => {
            let checkpoint = bindings.len();
            bindings.push((*local_id, name.clone(), value.clone()));
            if pattern_matches(inner, value, bindings) {
                true
            } else {
                bindings.truncate(checkpoint);
                false
            }
        }
        IrPattern::Variant {
            owner,
            variant_index,
            args,
            ..
        } => {
            let Value::Enum(e) = value else { return false };
            if e.type_id() != *owner || e.variant_index() != *variant_index {
                return false;
            }
            let fields = e.fields_cloned();
            if args.len() != fields.len() {
                return false;
            }
            let checkpoint = bindings.len();
            for (p, v) in args.iter().zip(fields.iter()) {
                if !pattern_matches(p, v, bindings) {
                    bindings.truncate(checkpoint);
                    return false;
                }
            }
            true
        }
        IrPattern::Some_(inner) => match value {
            Value::OptionSome(v) => pattern_matches(inner, &v.get(), bindings),
            _ => false,
        },
        IrPattern::None_ => matches!(value, Value::OptionNone),
        IrPattern::Ok_(inner) => match value {
            Value::ResultOk(v) => pattern_matches(inner, &v.get(), bindings),
            _ => false,
        },
        IrPattern::Err_(inner) => match value {
            Value::ResultErr(v) => pattern_matches(inner, &v.get(), bindings),
            _ => false,
        },
        IrPattern::Record { fields } => {
            let Value::Struct(sv) = value else { return false };
            let checkpoint = bindings.len();
            for (fname, sub) in fields {
                let Some(fv) = sv.get_field(fname) else {
                    bindings.truncate(checkpoint);
                    return false;
                };
                if !pattern_matches(sub, &fv, bindings) {
                    bindings.truncate(checkpoint);
                    return false;
                }
            }
            true
        }
    }
}
