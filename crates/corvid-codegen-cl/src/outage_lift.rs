//! Lift `match` arms that can raise an outage into agents of their own
//! (recoverable failures TE-3, widening the `match` narrowing).
//!
//! The IR has no statement form inside a `match` arm, so a call that can
//! fail there cannot be followed by the test that unwinds it. Lifting
//! fixes that without new control flow: the arm's body becomes the body
//! of a synthesized agent, whose parameters are the locals the body reads
//! (pattern bindings and captured locals alike), and the arm becomes one
//! call to it. The lifted agent's own failing calls unwind by its returns,
//! exactly as any agent's do. The arm's call is then the arm's whole body,
//! so nothing in the arm reads the zero bits an outage leaves; the
//! expansion pass marks it checked and tests the slot right after the
//! `match`.
//!
//! A guard that can fail is lifted the same way, into an agent returning
//! `Bool`. An outage there leaves the guard `false`, so the expansion
//! guards every later arm with "no outage is pending" and adds a last arm
//! that yields the unwind value, and no later guard or body runs before
//! the test after the `match`. Only `match` expressions in positions the
//! expansion pass hoists from are lifted; anything else is refused by the
//! outage plan, as before.

use crate::outage_plan::OutagePlan;
use corvid_ir::{IrAgent, IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrParam, IrPattern, IrStmt};
use corvid_resolve::{DefId, LocalId};
use std::collections::{BTreeMap, HashSet};

/// `ir` with every liftable failing `match` arm lifted, or `None` when
/// there is nothing to lift.
pub fn lift_failing_arms(ir: &IrFile) -> Option<IrFile> {
    let plan = OutagePlan::for_file(ir);
    let mut lifted = Vec::new();
    let mut agents = ir.agents.clone();
    let mut next = Lifter { plan: &plan, used: used_ids(ir), next_id: u32::MAX, lifted: &mut lifted };
    for agent in &mut agents {
        let owner = Owner { name: agent.name.clone(), wrapping_arithmetic: agent.wrapping_arithmetic };
        next.block(&mut agent.body, &owner);
    }
    if lifted.is_empty() {
        return None;
    }
    // A lifted body may hold failing arms of its own.
    let mut out = ir.clone();
    out.agents = agents;
    out.agents.extend(lifted);
    Some(lift_failing_arms(&out).unwrap_or(out))
}

/// Every definition id the program uses, so a lifted agent's id is
/// never one of them. Some ids sit at the top of the range as markers
/// (a built-in type's), so lifted ids are taken downward from there,
/// skipping each one in use, rather than counted up past the largest.
fn used_ids(ir: &IrFile) -> HashSet<u32> {
    let mut used = HashSet::new();
    corvid_ir::for_each_expr(ir, &mut |_, e| {
        if let IrExprKind::Call { kind: IrCallKind::Agent { def_id } | IrCallKind::Tool { def_id, .. } | IrCallKind::Prompt { def_id }, .. } = &e.kind {
            used.insert(def_id.0);
        }
    });
    used.extend(ir.agents.iter().map(|a| a.id.0));
    used.extend(ir.tools.iter().map(|t| t.id.0));
    used.extend(ir.prompts.iter().map(|p| p.id.0));
    used.extend(ir.types.iter().map(|t| t.id.0));
    used
}

/// What a lifted agent takes from the agent its arm came from.
struct Owner {
    name: String,
    wrapping_arithmetic: bool,
}

struct Lifter<'a> {
    plan: &'a OutagePlan,
    used: HashSet<u32>,
    /// The next candidate id, counting down.
    next_id: u32,
    lifted: &'a mut Vec<IrAgent>,
}

impl Lifter<'_> {
    fn block(&mut self, block: &mut IrBlock, owner: &Owner) {
        for stmt in &mut block.stmts {
            match stmt {
                IrStmt::Let { value, .. }
                | IrStmt::Expr { expr: value, .. }
                | IrStmt::Return { value: Some(value), .. }
                | IrStmt::Destructure { value, .. } => self.expr(value, owner),
                IrStmt::Approve { args, .. } => args.iter_mut().for_each(|arg| self.expr(arg, owner)),
                IrStmt::Assign { value, path, .. } => {
                    for part in path {
                        if let corvid_ir::IrPathSeg::Index(index) = part {
                            self.expr(index, owner);
                        }
                    }
                    self.expr(value, owner);
                }
                IrStmt::If { cond, then_block, else_block, .. } => {
                    self.expr(cond, owner);
                    self.block(then_block, owner);
                    if let Some(block) = else_block {
                        self.block(block, owner);
                    }
                }
                IrStmt::For { iter, body, .. } => {
                    self.expr(iter, owner);
                    self.block(body, owner);
                }
                IrStmt::While { cond, body, .. } => {
                    self.expr(cond, owner);
                    self.block(body, owner);
                }
                _ => {}
            }
        }
    }

    /// Descend where the expansion pass hoists from, lifting the failing
    /// arms of each `match` met there.
    fn expr(&mut self, expr: &mut IrExpr, owner: &Owner) {
        use IrExprKind::*;
        match &mut expr.kind {
            Match { scrutinee, arms } => {
                self.expr(scrutinee, owner);
                for arm in arms {
                    if let Some(guard) = &mut arm.guard {
                        if self.plan.can_fail(guard) && !self.is_checkable_call(guard) {
                            self.lift(guard, owner);
                        }
                    }
                    if self.plan.can_fail(&arm.body) && !self.is_checkable_call(&arm.body) {
                        self.lift(&mut arm.body, owner);
                    }
                }
            }
            TryPropagate { inner }
            | OptionSome { inner }
            | ResultOk { inner }
            | ResultErr { inner }
            | UnOp { operand: inner, .. }
            | WrappingUnOp { operand: inner, .. }
            | FieldAccess { target: inner, .. }
            | TrustBoundary { inner } => self.expr(inner, owner),
            BinOp { left, right, .. } | WrappingBinOp { left, right, .. } | Index { target: left, index: right } => {
                self.expr(left, owner);
                self.expr(right, owner);
            }
            Call { args, .. } | List { items: args } => args.iter_mut().for_each(|a| self.expr(a, owner)),
            MapLiteral { keys, values } => {
                keys.iter_mut().chain(values.iter_mut()).for_each(|e| self.expr(e, owner))
            }
            BuiltinMethod { receiver, args, .. } => {
                self.expr(receiver, owner);
                args.iter_mut().for_each(|a| self.expr(a, owner));
            }
            TryRetry { body, fallback, .. } => {
                self.expr(body, owner);
                if let Some(fallback) = fallback {
                    self.expr(fallback, owner);
                }
            }
            _ => {}
        }
    }

    /// An arm whose whole body is a failing call with arguments that
    /// cannot fail needs no lifting: the call itself can be checked.
    fn is_checkable_call(&self, body: &IrExpr) -> bool {
        matches!(&body.kind, IrExprKind::Call { kind, args, .. }
            if self.plan.call_can_fail(kind) && args.iter().all(|a| !self.plan.can_fail(a)))
    }

    fn lift(&mut self, body: &mut IrExpr, owner: &Owner) {
        let free = free_locals(body);
        while self.used.contains(&self.next_id) {
            self.next_id -= 1;
        }
        let id = DefId(self.next_id);
        self.used.insert(self.next_id);
        let name = format!("__corvid_{}_arm_{}", owner.name, id.0);
        let span = body.span;
        let params: Vec<IrParam> = free
            .iter()
            .map(|(local_id, (name, ty))| IrParam { name: name.clone(), local_id: *local_id, ty: ty.clone(), refinement: None, span })
            .collect();
        let args = params
            .iter()
            .map(|p| IrExpr {
                kind: IrExprKind::Local { local_id: p.local_id, name: p.name.clone() },
                ty: p.ty.clone(),
                span,
            })
            .collect();
        let ty = body.ty.clone();
        let lifted_body = std::mem::replace(
            body,
            IrExpr { kind: IrExprKind::Call { kind: IrCallKind::Agent { def_id: id }, callee_name: name.clone(), args }, ty: ty.clone(), span },
        );
        self.lifted.push(IrAgent {
            id,
            name,
            extern_abi: None,
            params,
            return_ty: ty,
            cost_budget: None,
            wrapping_arithmetic: owner.wrapping_arithmetic,
            is_replayable: false,
            pure_fn: false,
            retry_max_attempts: None,
            retry_backoff_ms: None,
            idempotency_key_param: None,
            body: IrBlock { stmts: vec![IrStmt::Return { value: Some(lifted_body), span }], span },
            span,
            borrow_sig: None,
        });
    }
}

/// The locals `expr` reads that it does not bind itself, with their
/// names and types, in a stable order.
fn free_locals(expr: &IrExpr) -> BTreeMap<LocalId, (String, corvid_types::Type)> {
    let mut bound = HashSet::new();
    corvid_ir::for_each_subexpr(expr, &mut |e| match &e.kind {
        IrExprKind::Match { arms, .. } => {
            for arm in arms {
                pattern_binds(&arm.pattern, &mut bound);
            }
        }
        IrExprKind::Lambda { params, .. } => {
            for param in params {
                bound.insert(param.local_id);
            }
        }
        _ => {}
    });
    let mut free = BTreeMap::new();
    corvid_ir::for_each_subexpr(expr, &mut |e| {
        if let IrExprKind::Local { local_id, name } = &e.kind {
            if !bound.contains(local_id) {
                free.entry(*local_id).or_insert_with(|| (name.clone(), e.ty.clone()));
            }
        }
    });
    free
}

fn pattern_binds(pattern: &IrPattern, out: &mut HashSet<LocalId>) {
    let mut ids = Vec::new();
    pattern.bound_locals(&mut ids);
    out.extend(ids);
}
