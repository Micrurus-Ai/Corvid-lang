use super::expr::require_bool;
use super::{Flow, Interpreter};
use crate::errors::{InterpError, InterpErrorKind};
use super::expr::eval_binop;
use crate::step::{StepAction, StepEvent, StmtKind};
use crate::value::{StreamChunk, StreamValue, Value};
use crate::value_to_json;
use async_recursion::async_recursion;
use corvid_ast::{BackpressurePolicy, Span};
use corvid_ir::{IrAgent, IrBlock, IrExpr, IrExprKind, IrPathSeg, IrStmt};
use corvid_types::Type;
use std::sync::Arc;

impl<'ir> Interpreter<'ir> {
    pub(super) async fn run_body(&mut self, agent: &'ir IrAgent) -> Result<Value, InterpError> {
        if matches!(&agent.return_ty, Type::Stream(_)) {
            return self.spawn_stream_agent(agent).await;
        }
        let saved_budget = self.cost_budget;
        let saved_used = self.cost_used;
        self.cost_budget = agent.cost_budget;
        self.cost_used = 0.0;
        let flow = self.eval_block(&agent.body).await;
        self.cost_budget = saved_budget;
        self.cost_used = saved_used;
        match flow? {
            Flow::Return(v) => Ok(v),
            Flow::Normal => Ok(Value::Nothing),
            Flow::Break | Flow::Continue => Err(InterpError::new(
                InterpErrorKind::Other("loop control flow escaped its enclosing loop".into()),
                agent.span,
            )),
        }
    }

    async fn spawn_stream_agent(&mut self, agent: &'ir IrAgent) -> Result<Value, InterpError> {
        let (sender, stream) =
            StreamValue::channel(super::effect_compose::default_stream_backpressure());
        let ir = self.ir.clone();
        let runtime = self.runtime.clone();
        let agent = agent.clone();
        let env = self.env.clone();
        let local_names = self.local_names.clone();
        tokio::spawn(async move {
            let mut sub = Interpreter::new(&ir, &runtime).with_mocks();
            sub.env = env;
            sub.local_names = local_names;
            sub.stream_sender = Some(sender);
            sub.stream_cost_budget = agent.cost_budget;
            let outcome = sub.eval_block(&agent.body).await;
            let maybe_sender = sub.stream_sender.take();
            match outcome {
                Ok(Flow::Normal) | Ok(Flow::Return(_)) => {}
                Ok(Flow::Break) | Ok(Flow::Continue) => {
                    if let Some(sender) = maybe_sender {
                        let _ = sender
                            .send(Err(InterpError::new(
                                InterpErrorKind::Other(
                                    "loop control flow escaped its enclosing loop".into(),
                                ),
                                agent.span,
                            )))
                            .await;
                    }
                }
                Err(err) => {
                    if let Some(sender) = maybe_sender {
                        let _ = sender.send(Err(err)).await;
                    }
                }
            }
        });
        Ok(Value::Stream(stream))
    }

    pub(super) async fn singleton_stream(
        &self,
        chunk: StreamChunk,
        backpressure: BackpressurePolicy,
    ) -> Result<Value, InterpError> {
        let (sender, stream) = StreamValue::channel(backpressure);
        let _ = sender.send_chunk(Ok(chunk)).await;
        Ok(Value::Stream(stream))
    }

    pub(super) async fn singleton_stream_error(
        &self,
        err: InterpError,
        backpressure: BackpressurePolicy,
    ) -> Result<Value, InterpError> {
        let (sender, stream) = StreamValue::channel(backpressure);
        let _ = sender.send_chunk(Err(err)).await;
        Ok(Value::Stream(stream))
    }

    pub(super) fn prepend_stream_chunk(&self, first: StreamChunk, stream: StreamValue) -> Value {
        let backpressure = stream.backpressure().clone();
        let (sender, combined) = StreamValue::channel(backpressure);
        tokio::spawn(async move {
            if !sender.send_chunk(Ok(first)).await {
                return;
            }
            while let Some(item) = stream.next_chunk().await {
                if !sender.send_chunk(item).await {
                    break;
                }
            }
        });
        Value::Stream(combined)
    }

    fn chunk_for_expr(&self, expr: &IrExpr, value: Value) -> StreamChunk {
        if let IrExprKind::Local { local_id, .. } = &expr.kind {
            if let Some(chunk) = self.stream_locals.get(local_id) {
                return StreamChunk {
                    value,
                    cost: chunk.cost,
                    confidence: chunk.confidence,
                    tokens: chunk.tokens,
                };
            }
        }
        StreamChunk::new(value)
    }

    fn stream_limit_violation(&self, chunk: &StreamChunk, span: Span) -> Option<InterpError> {
        let budget = self.stream_cost_budget?;
        let used = self.stream_cost_used + chunk.cost;
        if used > budget {
            Some(InterpError::new(
                InterpErrorKind::BudgetExceeded { budget, used },
                span,
            ))
        } else {
            None
        }
    }

    pub(super) fn charge_cost(&mut self, cost: f64, span: Span) -> Result<(), InterpError> {
        let Some(budget) = self.cost_budget else {
            self.cost_used += cost;
            return Ok(());
        };
        let used = self.cost_used + cost;
        if used > budget {
            return Err(InterpError::new(
                InterpErrorKind::BudgetExceeded { budget, used },
                span,
            ));
        }
        self.cost_used = used;
        Ok(())
    }

    #[async_recursion]
    pub(super) async fn eval_block(
        &mut self,
        block: &'ir IrBlock,
    ) -> Result<Flow, InterpError> {
        for stmt in &block.stmts {
            match self.eval_stmt(stmt).await? {
                Flow::Normal => continue,
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    #[async_recursion]
    async fn eval_stmt(&mut self, stmt: &'ir IrStmt) -> Result<Flow, InterpError> {
        match stmt {
            IrStmt::Let {
                local_id,
                name,
                value,
                ..
            } => {
                if self.should_yield_statement() {
                    self.maybe_yield(StepEvent::BeforeStatement {
                        kind: StmtKind::Let { name: name.clone() },
                        span: value.span,
                        env: self.env_snapshot(),
                    })
                    .await?;
                }
                let v = match self.eval_expr(value).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(Flow::Return(v)),
                };
                self.env.bind(*local_id, v);
                self.local_names.insert(*local_id, name.clone());
                if let IrExprKind::Local {
                    local_id: source_local,
                    ..
                } = &value.kind
                {
                    if let Some(chunk) = self.stream_locals.get(source_local).cloned() {
                        self.stream_locals.insert(
                            *local_id,
                            StreamChunk {
                                value: self.env.lookup(*local_id).unwrap_or(Value::Nothing),
                                ..chunk
                            },
                        );
                    } else {
                        self.stream_locals.remove(local_id);
                    }
                } else {
                    self.stream_locals.remove(local_id);
                }
                Ok(Flow::Normal)
            }
            IrStmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => match self.eval_expr(e).await?.into_value() {
                        Ok(v) | Err(v) => v,
                    },
                    None => Value::Nothing,
                };
                Ok(Flow::Return(v))
            }
            IrStmt::Yield { value, span } => {
                let yielded = match self.eval_expr(value).await?.into_value() {
                    Ok(v) | Err(v) => v,
                };
                let Some(sender) = self.stream_sender.as_ref() else {
                    return Err(InterpError::new(
                        InterpErrorKind::NotImplemented("stream yield statements".into()),
                        *span,
                    ));
                };
                let chunk = self.chunk_for_expr(value, yielded);
                if let Some(err) = self.stream_limit_violation(&chunk, *span) {
                    let _ = sender.send_chunk(Err(err)).await;
                    return Ok(Flow::Return(Value::Nothing));
                }
                self.stream_cost_used += chunk.cost;
                if !sender.send_chunk(Ok(chunk)).await {
                    return Ok(Flow::Return(Value::Nothing));
                }
                Ok(Flow::Normal)
            }
            IrStmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let c = match self.eval_expr(cond).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(Flow::Return(v)),
                };
                // D2 Provenance Propagation: `require_bool` strips
                // `Value::Grounded` so a grounded-Bool condition picks
                // the right branch. Centralised in the helper rather
                // than duplicated here.
                let take_then = require_bool(&c, cond.span, "`if` condition")?;
                if take_then {
                    self.eval_block(then_block).await
                } else if let Some(eb) = else_block {
                    self.eval_block(eb).await
                } else {
                    Ok(Flow::Normal)
                }
            }
            IrStmt::For {
                var_local,
                iter,
                body,
                span,
                ..
            } => {
                let iter_val = match self.eval_expr(iter).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(Flow::Return(v)),
                };
                match iter_val {
                    Value::List(items) => {
                        self.stream_locals.remove(var_local);
                        for item in items.iter_cloned() {
                            self.env.bind(*var_local, item);
                            match self.eval_block(body).await? {
                                Flow::Normal | Flow::Continue => continue,
                                Flow::Break => return Ok(Flow::Normal),
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                            }
                        }
                    }
                    Value::String(s) => {
                        self.stream_locals.remove(var_local);
                        for item in s.chars().map(|c| Value::String(Arc::from(c.to_string()))) {
                            self.env.bind(*var_local, item);
                            match self.eval_block(body).await? {
                                Flow::Normal | Flow::Continue => continue,
                                Flow::Break => return Ok(Flow::Normal),
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                            }
                        }
                    }
                    Value::Stream(stream) => {
                        while let Some(item) = stream.next_chunk().await {
                            let chunk = item?;
                            self.env.bind(*var_local, chunk.value.clone());
                            self.stream_locals.insert(*var_local, chunk);
                            match self.eval_block(body).await? {
                                Flow::Normal | Flow::Continue => continue,
                                Flow::Break => return Ok(Flow::Normal),
                                Flow::Return(v) => return Ok(Flow::Return(v)),
                            }
                        }
                        self.stream_locals.remove(var_local);
                    }
                    other => {
                        return Err(InterpError::new(
                            InterpErrorKind::TypeMismatch {
                                expected: "List, Stream, or String".into(),
                                got: other.type_name(),
                            },
                            *span,
                        ));
                    }
                }
                Ok(Flow::Normal)
            }
            // `parallel:` block (slice 46e): every arm's call runs
            // CONCURRENTLY on its own sub-interpreter (cloned env —
            // shared cells stay shared per the reference-semantics
            // model) with a BUFFERED per-arm tracer. At the join:
            // buffers flush in ARM ORDER (the recorded trace is
            // indistinguishable from sequential arm-order
            // execution, so replay works unchanged and replays
            // arms sequentially), costs charge to the parent
            // budget in arm order, then the error rule fires
            // (first failed arm BY INDEX — deterministic), then
            // arm names bind.
            IrStmt::Parallel { arms, span } => {
                let mut buffers = Vec::with_capacity(arms.len());
                let mut arm_runtimes = Vec::with_capacity(arms.len());
                for _ in arms {
                    let (tracer, buffer) = self.runtime.tracer().buffered();
                    buffers.push(buffer);
                    arm_runtimes.push(self.runtime.with_arm_tracer(tracer));
                }
                let ir = self.ir;
                let mut futures = Vec::with_capacity(arms.len());
                for (arm, arm_rt) in arms.iter().zip(arm_runtimes.iter()) {
                    let env = self.env.clone();
                    let mocks = self.mock_tools.clone();
                    let budget = self.cost_budget.map(|b| b - self.cost_used);
                    futures.push(async move {
                        let mut sub = Interpreter::new(ir, arm_rt);
                        sub.env = env;
                        sub.mock_tools = mocks;
                        sub.cost_budget = budget;
                        let outcome = sub.eval_expr(&arm.call).await;
                        (outcome, sub.cost_used)
                    });
                }
                let results = futures::future::join_all(futures).await;

                // 1. Flush arm trace buffers IN ARM ORDER.
                for buffer in &buffers {
                    self.runtime.tracer().flush_buffer(buffer);
                }
                // 2. Charge costs in arm order (all arms ran; all
                //    are paid — the parallel operator's Sum).
                for (_, cost) in &results {
                    self.charge_cost(*cost, *span)?;
                }
                // 3. Error rule: first failed arm by index.
                let mut values = Vec::with_capacity(results.len());
                for (outcome, _) in results {
                    match outcome {
                        Ok(flow) => match flow.into_value() {
                            Ok(v) => values.push(v),
                            Err(v) => return Ok(Flow::Return(v)),
                        },
                        Err(e) => return Err(e),
                    }
                }
                // 4. Bind arm names.
                for (arm, value) in arms.iter().zip(values) {
                    self.env.bind(arm.local_id, value);
                }
                Ok(Flow::Normal)
            }
            // Destructuring binding (slice 45n): evaluate once,
            // bind every pattern binding transactionally through
            // the 45i pattern machinery. The checker guarantees
            // irrefutability, so a non-match here is a hard error
            // (type confusion), not a silent skip.
            IrStmt::Destructure {
                pattern,
                value,
                span,
            } => {
                let v = match self.eval_expr(value).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(Flow::Return(v)),
                };
                let mut binds = Vec::new();
                if !crate::interp::pattern_matches(pattern, &v, &mut binds) {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "a value matching the destructuring pattern".into(),
                            got: v.type_name(),
                        },
                        *span,
                    ));
                }
                for (local_id, _, bound) in binds {
                    self.env.bind(local_id, bound);
                }
                Ok(Flow::Normal)
            }
            // `while cond:` (slice 45k) — re-evaluate the condition
            // before every iteration; a non-Bool condition traps
            // (the checker only lets Bool/Unknown through).
            IrStmt::While { cond, body, span } => {
                loop {
                    let cond_val = match self.eval_expr(cond).await?.into_value() {
                        Ok(v) => v,
                        Err(v) => return Ok(Flow::Return(v)),
                    };
                    let keep_going = match cond_val {
                        Value::Bool(b) => b,
                        Value::Grounded(g) => match g.inner.get() {
                            Value::Bool(b) => b,
                            other => {
                                return Err(InterpError::new(
                                    InterpErrorKind::TypeMismatch {
                                        expected: "Bool".into(),
                                        got: other.type_name(),
                                    },
                                    *span,
                                ));
                            }
                        },
                        other => {
                            return Err(InterpError::new(
                                InterpErrorKind::TypeMismatch {
                                    expected: "Bool".into(),
                                    got: other.type_name(),
                                },
                                *span,
                            ));
                        }
                    };
                    if !keep_going {
                        break;
                    }
                    match self.eval_block(body).await? {
                        Flow::Normal | Flow::Continue => continue,
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            IrStmt::Approve { label, args, span } => {
                let mut json_args = Vec::with_capacity(args.len());
                for a in args {
                    let v = match self.eval_expr(a).await?.into_value() {
                        Ok(v) => v,
                        Err(v) => return Ok(Flow::Return(v)),
                    };
                    json_args.push(value_to_json(&v));
                }

                if self.should_yield_boundary() {
                    let action = self
                        .maybe_yield(StepEvent::BeforeApproval {
                            label: label.clone(),
                            args: json_args.clone(),
                            confidence_gate: None,
                            span: *span,
                            env: self.env_snapshot(),
                        })
                        .await?;

                    match action {
                        StepAction::Approve => {
                            if self.should_yield_boundary() {
                                self.maybe_yield(StepEvent::AfterApproval {
                                    label: label.clone(),
                                    approved: true,
                                    span: *span,
                                })
                                .await?;
                            }
                            return Ok(Flow::Normal);
                        }
                        StepAction::Deny => {
                            if self.should_yield_boundary() {
                                self.maybe_yield(StepEvent::AfterApproval {
                                    label: label.clone(),
                                    approved: false,
                                    span: *span,
                                })
                                .await?;
                            }
                            return Err(InterpError::new(
                                InterpErrorKind::Runtime(
                                    corvid_runtime::RuntimeError::ApprovalDenied {
                                        action: label.clone(),
                                    },
                                ),
                                *span,
                            ));
                        }
                        _ => {}
                    }
                }

                let result = self.runtime.approval_gate(label, json_args).await;
                let approved = result.is_ok();

                if self.should_yield_boundary() {
                    self.maybe_yield(StepEvent::AfterApproval {
                        label: label.clone(),
                        approved,
                        span: *span,
                    })
                    .await?;
                }

                result.map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), *span))?;
                Ok(Flow::Normal)
            }
            IrStmt::Expr { expr, .. } => {
                if let Err(v) = self.eval_expr(expr).await?.into_value() {
                    return Ok(Flow::Return(v));
                }
                Ok(Flow::Normal)
            }
            // Place assignment (45b): `x.field = v`, `xs[i] = v`,
            // compound `op=`. Reference semantics: structs and lists
            // are shared heap cells, so mutation through one binding
            // is visible through every alias. Evaluation order: path
            // index expressions left-to-right, then the value, then
            // the store. The compound operator reads the current slot
            // exactly once and reuses the checked `eval_binop`.
            IrStmt::Assign {
                local_id,
                name,
                path,
                op,
                value,
                span,
            } => {
                let mut idx_values: Vec<Option<Value>> = Vec::with_capacity(path.len());
                for seg in path {
                    match seg {
                        IrPathSeg::Index(idx_expr) => {
                            let v = match self.eval_expr(idx_expr).await?.into_value() {
                                Ok(v) => v,
                                Err(v) => return Ok(Flow::Return(v)),
                            };
                            idx_values.push(Some(v));
                        }
                        IrPathSeg::Field(_) => idx_values.push(None),
                    }
                }
                let rhs = match self.eval_expr(value).await?.into_value() {
                    Ok(v) => v,
                    Err(v) => return Ok(Flow::Return(v)),
                };

                let root = self.env.lookup(*local_id).ok_or_else(|| {
                    InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: format!("a bound local `{name}`"),
                            got: "an unbound local".into(),
                        },
                        *span,
                    )
                })?;

                if path.is_empty() {
                    // Plain `x = v` lowers to IrStmt::Let; reaching
                    // here with an empty path is compound rebind
                    // (`x += v`).
                    let new_v = match op {
                        Some(op) => eval_binop(*op, root, rhs, *span, false)?,
                        None => rhs,
                    };
                    self.env.bind(*local_id, new_v);
                    self.stream_locals.remove(local_id);
                    return Ok(Flow::Normal);
                }

                // Walk to the container that owns the FINAL segment.
                let mut cur = root;
                for (seg, idx_v) in path[..path.len() - 1]
                    .iter()
                    .zip(idx_values[..path.len() - 1].iter())
                {
                    cur = assign_path_read(cur, seg, idx_v, *span)?;
                }

                let last_seg = path.last().expect("non-empty path");
                let last_idx = idx_values.last().expect("non-empty path");
                match (last_seg, cur) {
                    (IrPathSeg::Field(field), Value::Struct(sv)) => {
                        let new_v = match op {
                            Some(op) => {
                                let current = sv.get_field(field).ok_or_else(|| {
                                    InterpError::new(
                                        InterpErrorKind::UnknownField {
                                            struct_name: sv.type_name().to_string(),
                                            field: field.clone(),
                                        },
                                        *span,
                                    )
                                })?;
                                eval_binop(*op, current, rhs, *span, false)?
                            }
                            None => rhs,
                        };
                        sv.set_field(field.clone(), new_v);
                    }
                    // Map store (45g): m[k] = v inserts or updates;
                    // compound (m[k] += v) reads the current value or
                    // traps when the key is missing.
                    (IrPathSeg::Index(_), Value::Map(mv)) => {
                        let key = last_idx
                            .as_ref()
                            .expect("index segment has a value")
                            .clone();
                        let new_v = match op {
                            Some(op) => {
                                let current = mv.get_by_key(&key).ok_or_else(|| {
                                    InterpError::new(
                                        InterpErrorKind::TypeMismatch {
                                            expected: "an existing key for compound assignment"
                                                .into(),
                                            got: format!("missing key `{key}`"),
                                        },
                                        *span,
                                    )
                                })?;
                                eval_binop(*op, current, rhs, *span, false)?
                            }
                            None => rhs,
                        };
                        mv.insert_or_update(key, new_v);
                    }
                    (IrPathSeg::Index(_), Value::List(lv)) => {
                        let idx = match last_idx.as_ref().expect("index segment has a value") {
                            Value::Int(i) => *i,
                            other => {
                                return Err(InterpError::new(
                                    InterpErrorKind::TypeMismatch {
                                        expected: "Int".into(),
                                        got: other.type_name(),
                                    },
                                    *span,
                                ))
                            }
                        };
                        let len = lv.len();
                        if idx < 0 || (idx as usize) >= len {
                            return Err(InterpError::new(
                                InterpErrorKind::IndexOutOfBounds { len, index: idx },
                                *span,
                            ));
                        }
                        let new_v = match op {
                            Some(op) => {
                                let current =
                                    lv.get(idx as usize).expect("bounds checked above");
                                eval_binop(*op, current, rhs, *span, false)?
                            }
                            None => rhs,
                        };
                        lv.set(idx as usize, new_v);
                    }
                    (IrPathSeg::Field(_), other) => {
                        return Err(InterpError::new(
                            InterpErrorKind::TypeMismatch {
                                expected: "a struct value".into(),
                                got: other.type_name(),
                            },
                            *span,
                        ))
                    }
                    (IrPathSeg::Index(_), other) => {
                        return Err(InterpError::new(
                            InterpErrorKind::TypeMismatch {
                                expected: "List".into(),
                                got: other.type_name(),
                            },
                            *span,
                        ))
                    }
                }
                Ok(Flow::Normal)
            }
            IrStmt::Break { .. } => Ok(Flow::Break),
            IrStmt::Continue { .. } => Ok(Flow::Continue),
            IrStmt::Pass { .. } => Ok(Flow::Normal),
            IrStmt::Dup { .. } | IrStmt::Drop { .. } => Ok(Flow::Normal),
        }
    }
}


/// Read one step of a place-assignment path (slice 45b): `.field` on a
/// struct or `[idx]` on a list. Mirrors the read-path errors of
/// `FieldAccess` / `Index` expression evaluation.
fn assign_path_read(
    cur: Value,
    seg: &IrPathSeg,
    idx_v: &Option<Value>,
    span: corvid_ast::Span,
) -> Result<Value, InterpError> {
    match (seg, cur) {
        (IrPathSeg::Field(field), Value::Struct(sv)) => {
            sv.get_field(field).ok_or_else(|| {
                InterpError::new(
                    InterpErrorKind::UnknownField {
                        struct_name: sv.type_name().to_string(),
                        field: field.clone(),
                    },
                    span,
                )
            })
        }
        (IrPathSeg::Index(_), Value::Map(mv)) => {
            let key = idx_v.as_ref().expect("index segment has a value");
            mv.get_by_key(key).ok_or_else(|| {
                InterpError::new(
                    InterpErrorKind::TypeMismatch {
                        expected: "an existing key on the assignment path".into(),
                        got: format!("missing key `{key}`"),
                    },
                    span,
                )
            })
        }
        (IrPathSeg::Index(_), Value::List(lv)) => {
            let idx = match idx_v.as_ref().expect("index segment has a value") {
                Value::Int(i) => *i,
                other => {
                    return Err(InterpError::new(
                        InterpErrorKind::TypeMismatch {
                            expected: "Int".into(),
                            got: other.type_name(),
                        },
                        span,
                    ))
                }
            };
            let len = lv.len();
            if idx < 0 || (idx as usize) >= len {
                return Err(InterpError::new(
                    InterpErrorKind::IndexOutOfBounds { len, index: idx },
                    span,
                ));
            }
            Ok(lv.get(idx as usize).expect("bounds checked above"))
        }
        (IrPathSeg::Field(_), other) => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "a struct value".into(),
                got: other.type_name(),
            },
            span,
        )),
        (IrPathSeg::Index(_), other) => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "List".into(),
                got: other.type_name(),
            },
            span,
        )),
    }
}
