//! Statement + block + approve tracking.
//!
//! Walks every statement in a block body, tracking control-flow
//! side effects (let-bind types, approve-stack additions, yield
//! legality, return type matches, effect-frontier bumps).
//!
//! Extracted from `checker.rs` as part of Phase 20i responsibility
//! decomposition.

use super::{Approval, Checker};
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{Block, Expr, Stmt, WeakEffect};
use corvid_resolve::Binding;

impl<'a> Checker<'a> {
    pub(super) fn check_block(&mut self, b: &Block) {
        // Save approval-stack depth so approvals don't leak out of this block.
        let saved_depth = self.approvals.len();
        for stmt in &b.stmts {
            self.check_stmt(stmt);
        }
        self.approvals.truncate(saved_depth);
    }

    pub(super) fn check_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let {
                name, ty, value, ..
            } => {
                // Slice 45q: the two known inference limits, made
                // visible. `[]` and bare `None` without an
                // annotation bind Unknown inner types; downstream
                // checks weaken silently.
                if ty.is_none() {
                    let construct_hint = match value {
                        corvid_ast::Expr::List { items, .. } if items.is_empty() => Some((
                            "an empty list literal `[]`".to_string(),
                            format!("{}: List<T> = []", name.name),
                        )),
                        corvid_ast::Expr::Ident { name: n, .. } if n.name == "None" => Some((
                            "a bare `None`".to_string(),
                            format!("{}: Option<T> = None", name.name),
                        )),
                        _ => None,
                    };
                    if let Some((construct, hint)) = construct_hint {
                        self.warnings.push(crate::errors::TypeWarning::new(
                            crate::errors::TypeWarningKind::InferenceNeedsAnnotation {
                                binding: name.name.clone(),
                                construct,
                                hint,
                            },
                            s.span(),
                        ));
                    }
                }
                let explicit_ty = ty.as_ref().map(|t| self.type_ref_to_type(t));
                let value_ty = self.check_expr_as(value, explicit_ty.as_ref());
                let local_ty = match ty {
                    Some(_) => explicit_ty.expect("explicit let type already computed"),
                    None => value_ty.clone(),
                };
                if !value_ty.is_assignable_to(&local_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: local_ty.display_name(),
                            got: value_ty.display_name(),
                            context: format!("assignment to `{}`", name.name),
                        },
                        value.span(),
                    ));
                }
                self.record_if_grounded_coercion(&value_ty, &local_ty, value.span());
                if let Some(Binding::Local(local_id)) = self.bindings.get(&name.span) {
                    self.update_weak_local_on_assignment(*local_id, value, &local_ty);
                    self.local_types.insert(*local_id, local_ty);
                }
            }
            Stmt::Return { value, span } => {
                let got = match value {
                    Some(e) => {
                        let expected = self.current_return.clone();
                        self.check_expr_as(e, expected.as_ref())
                    }
                    None => Type::Nothing,
                };
                if let Some(expected) = self.current_return.clone() {
                    if !got.is_assignable_to(&expected) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::ReturnTypeMismatch {
                                expected: expected.display_name(),
                                got: got.display_name(),
                            },
                            *span,
                        ));
                    }
                    if let Some(e) = value {
                        self.record_if_grounded_coercion(&got, &expected, e.span());
                    }
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                let cond_ty = self.check_expr(cond);
                // D2: control-flow conditions accept `Grounded<Bool>` —
                // branching consumes the bool to pick a path, it does
                // not emit a laundered value, so contagion through `if`
                // is not required (and `&&` / `||` stay out of scope per
                // D1). D5: record the condition span when grounded so
                // IR lowering inserts a visible `UnwrapGrounded` at the
                // condition — the bool is destroyed by the branch, not
                // propagated, but `@grounded_pure` still has to see the
                // discard.
                if !matches!(cond_ty.ungrounded(), Type::Bool | Type::Unknown) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Bool".into(),
                            got: cond_ty.display_name(),
                            context: "`if` condition".into(),
                        },
                        cond.span(),
                    ));
                }
                self.record_if_grounded_coercion(&cond_ty, &Type::Bool, cond.span());
                let entry_frontier = self.effect_frontier;
                let entry_weak_refresh = self.weak_refresh.clone();

                self.effect_frontier = entry_frontier;
                self.weak_refresh = entry_weak_refresh.clone();
                self.check_block(then_block);
                let then_frontier = self.effect_frontier;
                let then_refresh = self.weak_refresh.clone();

                let (else_frontier, else_refresh) = if let Some(b) = else_block {
                    self.effect_frontier = entry_frontier;
                    self.weak_refresh = entry_weak_refresh.clone();
                    self.check_block(b);
                    (self.effect_frontier, self.weak_refresh.clone())
                } else {
                    (entry_frontier, entry_weak_refresh.clone())
                };

                self.effect_frontier = then_frontier.merge_max(else_frontier);
                self.weak_refresh =
                    self.merge_weak_refresh(&entry_weak_refresh, &then_refresh, &else_refresh);
            }
            Stmt::Yield { value, span } => {
                let yielded = self.check_expr(value);
                if !self.in_agent_body {
                    self.errors
                        .push(TypeError::new(TypeErrorKind::YieldOutsideAgent, *span));
                    return;
                }
                match self.current_return.clone() {
                    Some(Type::Stream(inner)) => {
                        self.saw_yield = true;
                        if !yielded.is_assignable_to(&inner) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::YieldReturnTypeMismatch {
                                    expected: inner.display_name(),
                                    got: yielded.display_name(),
                                },
                                value.span(),
                            ));
                        }
                        self.record_if_grounded_coercion(&yielded, &inner, value.span());
                    }
                    Some(other) => {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::YieldRequiresStreamReturn {
                                declared: other.display_name(),
                            },
                            *span,
                        ));
                    }
                    None => {
                        self.errors
                            .push(TypeError::new(TypeErrorKind::YieldOutsideAgent, *span));
                    }
                }
            }
            Stmt::For {
                var, iter, body, ..
            } => {
                let iter_ty = self.check_expr(iter);
                // Derive the loop variable's type from the iterable.
                // Lists iterate their element type; Strings iterate
                // chars (which Corvid currently models as String).
                let var_ty = match &iter_ty {
                    Type::List(elem) => (**elem).clone(),
                    Type::Stream(elem) => (**elem).clone(),
                    Type::String => Type::String,
                    Type::Unknown => Type::Unknown,
                    _other => Type::Unknown,
                };
                if let Some(Binding::Local(local_id)) = self.bindings.get(&var.span) {
                    self.local_types.insert(*local_id, var_ty);
                }
                let entry_frontier = self.effect_frontier;
                let entry_weak_refresh = self.weak_refresh.clone();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                let body_frontier = self.effect_frontier;
                let body_refresh = self.weak_refresh.clone();
                self.effect_frontier = entry_frontier.merge_max(body_frontier);
                self.weak_refresh = self.merge_weak_refresh(
                    &entry_weak_refresh,
                    &entry_weak_refresh,
                    &body_refresh,
                );
            }
            Stmt::Parallel { arms, .. } => {
                for arm in arms {
                    // v1 rule: the RHS must be a call to a
                    // tool/prompt/agent/fn — the concurrent unit is
                    // the effectful call; wrap richer logic in an
                    // agent. Stream-returning calls are rejected
                    // (join semantics for streams are post-v1).
                    let is_call = matches!(&arm.call, corvid_ast::Expr::Call { .. });
                    if !is_call {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::ParallelArmInvalid {
                                arm: arm.name.name.clone(),
                                message: "each arm must be `name = call(...)` — wrap richer logic in an agent and call it".into(),
                            },
                            arm.span,
                        ));
                    }
                    let ty = self.check_expr(&arm.call);
                    if matches!(ty.ungrounded(), Type::Stream(_)) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::ParallelArmInvalid {
                                arm: arm.name.name.clone(),
                                message: "stream-returning calls cannot join in a parallel block (v1)".into(),
                            },
                            arm.span,
                        ));
                    }
                    if let Some(corvid_resolve::Binding::Local(id)) =
                        self.bindings.get(&arm.name.span)
                    {
                        self.local_types.insert(*id, ty);
                    }
                }
            }
            Stmt::Destructure {
                pattern,
                value,
                span,
            } => {
                let val_ty = self.check_expr(value);
                // Reuse the match machinery: types every binding
                // against the value's field types.
                self.check_pattern(pattern, &val_ty);
                // Statement-position destructuring must be
                // IRREFUTABLE — refutable shapes belong in `match`.
                if !self.pattern_is_irrefutable(pattern) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::StructLiteralInvalid {
                            type_name: match pattern {
                                corvid_ast::Pattern::Record { name, .. } => name.name.clone(),
                                _ => "<pattern>".into(),
                            },
                            message:
                                "destructuring patterns must be irrefutable (bare names and `..` only)"
                                    .into(),
                        },
                        *span,
                    ));
                }
            }
            Stmt::While { cond, body, .. } => {
                // Same Grounded<Bool> acceptance rule as `if`: the
                // branch consumes the bool, it does not launder it.
                let cond_ty = self.check_expr(cond);
                if !matches!(cond_ty.ungrounded(), Type::Bool | Type::Unknown) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "Bool".into(),
                            got: cond_ty.display_name(),
                            context: "`while` condition".into(),
                        },
                        cond.span(),
                    ));
                }
                let entry_frontier = self.effect_frontier;
                let entry_weak_refresh = self.weak_refresh.clone();
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                let body_frontier = self.effect_frontier;
                let body_refresh = self.weak_refresh.clone();
                self.effect_frontier = entry_frontier.merge_max(body_frontier);
                self.weak_refresh = self.merge_weak_refresh(
                    &entry_weak_refresh,
                    &entry_weak_refresh,
                    &body_refresh,
                );
            }
            Stmt::Break { span } => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::LoopFlowOutsideLoop {
                            keyword: "break".into(),
                        },
                        *span,
                    ));
                }
            }
            Stmt::Continue { span } => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::LoopFlowOutsideLoop {
                            keyword: "continue".into(),
                        },
                        *span,
                    ));
                }
            }
            Stmt::Pass { .. } => {}
            Stmt::Approve { action, .. } => {
                self.check_approve(action);
                self.bump_effect(WeakEffect::Approve);
            }
            Stmt::Expr { expr, .. } => {
                let _ = self.check_expr(expr);
            }
            // Place assignment (45b): `x.field = v`, `xs[i] = v`,
            // compound `target op= value`. The parser guarantees the
            // target is an Ident / FieldAccess / Index; the checker
            // additionally requires the path's ROOT to be a local
            // variable so IR lowering has a stable base slot.
            Stmt::Assign {
                target, op, value, ..
            } => {
                if let Some(reason) = self.assign_target_root_problem(target) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidAssignTarget { reason },
                        target.span(),
                    ));
                }
                let mut target_ty = self.check_expr(target);
                // Map store (45g): the READ type of m[k] is Option<V>
                // but the assignment SLOT is V — `m[k] = v` inserts or
                // updates, so the value must be a V, not an Option.
                let mut map_slot = false;
                if let Expr::Index { target: base, .. } = target {
                    if let Type::Map(_, val_ty) = self.check_expr(base) {
                        target_ty = (*val_ty).clone();
                        map_slot = true;
                    }
                }
                // Compound on a map slot can't reuse check_binop (it
                // would see the Option<V> read type). Type the value
                // against the slot and apply the operator rule at the
                // type level: numeric slots take all five ops;
                // String/List take only `+`.
                if map_slot {
                    if let Some(op) = op {
                        let value_ty = self.check_expr_as(value, Some(&target_ty));
                        let op_ok = match (&target_ty, op) {
                            (Type::Int | Type::Float, _) => true,
                            (
                                Type::String | Type::List(_),
                                corvid_ast::BinaryOp::Add,
                            ) => true,
                            _ => false,
                        };
                        if !op_ok {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::TypeMismatch {
                                    expected: format!(
                                        "a compound operator valid for `{}`",
                                        target_ty.display_name()
                                    ),
                                    got: format!("`{op:?}=`"),
                                    context: "map compound assignment".into(),
                                },
                                s.span(),
                            ));
                        }
                        if !value_ty.is_assignable_to(&target_ty) {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::TypeMismatch {
                                    expected: target_ty.display_name(),
                                    got: value_ty.display_name(),
                                    context: "assignment target".into(),
                                },
                                value.span(),
                            ));
                        }
                        return;
                    }
                }
                let result_ty = match op {
                    // Compound: the operator's normal type rule runs on
                    // (target, value); `check_binop` re-checks both
                    // operands, which can duplicate a diagnostic on an
                    // already-invalid target — accepted trade-off to
                    // keep ONE operator type table.
                    Some(op) => self.check_binop(*op, target, value, s.span()),
                    None => self.check_expr_as(value, Some(&target_ty)),
                };
                if !result_ty.is_assignable_to(&target_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: target_ty.display_name(),
                            got: result_ty.display_name(),
                            context: "assignment target".into(),
                        },
                        value.span(),
                    ));
                }
                self.record_if_grounded_coercion(&result_ty, &target_ty, value.span());
            }
        }
    }

    /// Walk to the root of an assignment-target path and report why it
    /// cannot be assigned through, if anything. Valid roots are local
    /// variables (params, loop vars, and bindings).
    fn assign_target_root_problem(&self, target: &Expr) -> Option<String> {
        let mut root = target;
        loop {
            match root {
                Expr::FieldAccess { target, .. } | Expr::Index { target, .. } => {
                    root = target;
                }
                Expr::Ident { name, .. } => {
                    return match self.bindings.get(&name.span) {
                        Some(Binding::Local(_)) => None,
                        _ => Some(format!(
                            "the path's root `{}` must be a local variable                              (a binding, parameter, or loop variable)",
                            name.name
                        )),
                    };
                }
                _other => {
                    return Some(
                        "the path's root must be a local variable, not a call                          or literal expression"
                            .to_string(),
                    );
                }
            }
        }
    }

    pub(super) fn check_approve(&mut self, action: &Expr) {
        if let Expr::Call { callee, args, .. } = action {
            if let Expr::Ident { name, .. } = &**callee {
                let approval = Approval {
                    label: name.name.clone(),
                    arity: args.len(),
                };
                // `approvals` is the lexical-scope stack that gets
                // truncated on block exit (see `check_block`).
                // `approvals_seen_in_agent` is the body-wide audit
                // log that lets the dangerous-call diagnostic
                // discriminate "no approve at all" from "approve
                // exists but out of lexical scope" — see
                // `approval.token_lexical_only` in
                // `corvid-guarantees::GUARANTEE_REGISTRY`.
                self.approvals.push(approval.clone());
                self.approvals_seen_in_agent.push(approval);
            }
            // Always typecheck the args themselves for binding validity.
            for arg in args {
                let _ = self.check_expr(arg);
            }
        } else {
            let _ = self.check_expr(action);
        }
    }
}
