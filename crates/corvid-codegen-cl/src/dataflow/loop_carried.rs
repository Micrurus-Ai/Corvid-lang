//! Reference-counted locals whose value is carried out of one loop
//! iteration, though their first binding is inside the loop.
//!
//! Native lowering lowers a loop body once, so a `Let` there that is the
//! local's first binding in program order is lowered as a first binding:
//! it releases nothing, and at run time each iteration's new value
//! replaced the previous one without releasing it. A local whose value
//! dies within each iteration is fine, because the ownership pass drops
//! it at its last read. A local whose value is still needed after the
//! iteration leaked every value but the last.
//!
//! Such a local is bound before the body runs, with no value, as a
//! parameter is, so the `Let` in the loop is a rebinding that releases
//! the previous value (the first time there is none). The ownership
//! analysis (`cfg`) and native lowering both read the set this module
//! computes, so they agree on which `Let` is a rebinding.
//!
//! A local is carried when its first binding is inside a loop and it is
//! read outside that loop. (A read before the first binding is refused by
//! the resolver, so within the loop every read follows the binding.)

use super::cfg::collect_reads;
use crate::ownership::is_refcounted;
use corvid_ir::{IrAgent, IrBlock, IrExpr, IrPathSeg, IrStmt};
use corvid_resolve::LocalId;
use corvid_types::Type;
use std::collections::{BTreeMap, BTreeSet};

/// The loop-carried reference-counted locals of `agent`, with their types.
pub(crate) fn loop_carried_locals(agent: &IrAgent) -> BTreeMap<LocalId, Type> {
    let mut walk = Walk::default();
    walk.bound.extend(agent.params.iter().map(|p| p.local_id));
    walk.block(&agent.body);
    walk.carried
}

#[derive(Default)]
struct Walk {
    /// Locals bound so far, in program order.
    bound: BTreeSet<LocalId>,
    /// For a refcounted local first bound inside a loop: the outermost
    /// loop it was bound in, and its type.
    first_bound_in: BTreeMap<LocalId, (usize, Type)>,
    /// The loops enclosing the statement being walked, outermost first.
    loops: Vec<usize>,
    next_loop: usize,
    carried: BTreeMap<LocalId, Type>,
}

impl Walk {
    fn block(&mut self, block: &IrBlock) {
        for stmt in &block.stmts {
            self.stmt(stmt);
        }
    }

    fn reads(&mut self, expr: &IrExpr) {
        for read in collect_reads(expr, false) {
            self.read(read.local_id);
        }
    }

    fn read(&mut self, local: LocalId) {
        if let Some((loop_id, ty)) = self.first_bound_in.get(&local) {
            if !self.loops.contains(loop_id) {
                self.carried.insert(local, ty.clone());
            }
        }
    }

    fn bind(&mut self, local: LocalId, ty: &Type) {
        if !self.bound.insert(local) {
            return;
        }
        let Some(&outermost) = self.loops.first() else { return };
        if !is_refcounted(ty) {
            return;
        }
        self.first_bound_in.insert(local, (outermost, ty.clone()));
    }

    fn in_loop(&mut self, body: impl FnOnce(&mut Self)) {
        let id = self.next_loop;
        self.next_loop += 1;
        self.loops.push(id);
        body(self);
        self.loops.pop();
    }

    fn stmt(&mut self, stmt: &IrStmt) {
        match stmt {
            IrStmt::Let { local_id, ty, value, .. } => {
                self.reads(value);
                self.bind(*local_id, ty);
            }
            IrStmt::Assign { local_id, path, value, .. } => {
                for seg in path {
                    if let IrPathSeg::Index(index) = seg {
                        self.reads(index);
                    }
                }
                self.reads(value);
                self.read(*local_id);
            }
            IrStmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.reads(value);
                }
            }
            IrStmt::Yield { value, .. } | IrStmt::Destructure { value, .. } => self.reads(value),
            IrStmt::Expr { expr, .. } => self.reads(expr),
            IrStmt::If { cond, then_block, else_block, .. } => {
                self.reads(cond);
                self.block(then_block);
                if let Some(else_block) = else_block {
                    self.block(else_block);
                }
            }
            IrStmt::For { var_local, iter, body, .. } => {
                self.reads(iter);
                self.bound.insert(*var_local);
                self.in_loop(|walk| walk.block(body));
            }
            IrStmt::While { cond, body, .. } => self.in_loop(|walk| {
                walk.reads(cond);
                walk.block(body);
            }),
            IrStmt::Parallel { arms, .. } => {
                for arm in arms {
                    self.reads(&arm.call);
                }
            }
            IrStmt::Atomic { body, .. } => self.block(body),
            IrStmt::Approve { args, .. } => {
                for arg in args {
                    self.reads(arg);
                }
            }
            IrStmt::Break { .. }
            | IrStmt::Continue { .. }
            | IrStmt::Pass { .. }
            | IrStmt::Dup { .. }
            | IrStmt::Drop { .. } => {}
        }
    }
}
