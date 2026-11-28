//! IR Dup/Drop insertion driven by the ownership plan.
//!
//! This module is a shadow path. It takes an `IrAgent`, runs the
//! analysis, and returns a NEW agent with `IrStmt::Dup`/`IrStmt::Drop`
//! statements inserted at the precise positions the plan calls for.
//!
//! It is NOT wired into the compilation pipeline yet. The codegen
//! still consumes the un-transformed agent; the scattered
//! `emit_retain` / `emit_release` calls in `lowering.rs` remain the
//! authoritative ownership story until the unified pass took over.
//!
//! Why this ships in this form:
//!   - The ownership plan is a data structure; its correctness can be
//!     unit-tested. But "the plan applied to IR" is a separate
//!     transformation, and *its* correctness can also be unit-tested
//!     in isolation — without having to teach codegen to deduplicate
//!     against the existing emit_retain/release sites.
//!   - The integration step will (a) wire this transformation into the pipeline
//!     and (b) delete the 38 scattered emit_retain/release sites + 4
//!     peepholes. The two changes go together because either alone
//!     would either double-RC (if .6c only inserts) or zero-RC (if
//!     .6c only deletes). They MUST land in the same commit.
//!   - Verifying the insertion logic here means the pipeline wiring becomes a
//!     mechanical edit, not a creative one.
//!
//! ## Algorithm
//!
//! 1. Clone the input agent.
//! 2. Run `analyze_agent` on the clone (or on a borrow of the
//!    original — same result).
//! 3. Group all `dup`/`drop_after` plan entries by their `IrPath`.
//!    Sort each group's insertion list by IR-statement index in
//!    DESCENDING order so insertions don't shift later positions.
//! 4. Walk the IR tree following each path; at each leaf position,
//!    insert the planned `IrStmt::Dup` / `IrStmt::Drop` ops.
//!
//! ## Block-exit drops
//!
//! `OwnershipPlan::drops_at_block_exit` references CFG block IDs that
//! have no IR-tree analog — the entry CFG block is a flattening of
//! the agent's whole body. For .6b we resolve "drop at entry-block
//! exit" as "append a Drop at the END of `agent.body.stmts`, BEFORE
//! any trailing Return". For non-entry CFG blocks (synthetic join
//! blocks etc.) we don't currently emit drops here — the analysis
//! never assigns to those, since liveness only flags block-exit
//! drops for unused parameters of the function (entry block).

use corvid_ast::Span;
use corvid_ir::{IrAgent, IrBlock, IrExpr, IrExprKind, IrStmt};
use corvid_resolve::LocalId;
use std::collections::BTreeMap;

use crate::dataflow::{analyze_agent, IrNavStep, IrPath, OwnershipPlan};

mod outage;
mod propagation;

/// Public entry: clone `agent` and return a new agent with
/// `IrStmt::Dup` and `IrStmt::Drop` inserted per the ownership plan.
///
/// Pure: does not mutate the input. Stable across runs (the plan is
/// deterministic; insertion order is deterministic within a path).
pub fn insert_dup_drop(agent: &IrAgent) -> IrAgent {
    insert_dup_drop_with(agent, &crate::outage_plan::OutagePlan::default())
}

/// `insert_dup_drop`, with the calls that can raise an outage expanded
/// into their tests first (recoverable failures TE-3).
pub fn insert_dup_drop_with(agent: &IrAgent, outages: &crate::outage_plan::OutagePlan) -> IrAgent {
    // Diagnostic: when CORVID_DUP_DROP_DRY_RUN=1 is set, the pass
    // runs as a no-op — useful for bisecting whether a parity
    // failure is caused by pass-inserted Dup/Drop ops or by an
    // unguarded scattered emit site in codegen. Silent under
    // normal operation.
    if std::env::var("CORVID_DUP_DROP_DRY_RUN")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return agent.clone();
    }
    let (expanded, next) = expand_propagation(agent, outages);
    let (_cfg, plan) = analyze_agent(&expanded);
    apply_plan_owned(expanded, &plan, next)
}

pub(crate) fn expand_propagation(agent: &IrAgent, outages: &crate::outage_plan::OutagePlan) -> (IrAgent, u32) {
    let mut expanded = agent.clone();
    let mut next = next_local_id(agent);
    let cx = propagation::Cx { return_ty: &agent.return_ty, outages, unwind: outage::Unwind::Return };
    propagation::expand_block(&mut expanded.body, &cx, &mut next);
    (expanded, next)
}

/// Apply a pre-computed plan to a clone of `agent`. Split out from
/// `insert_dup_drop` so tests can inject hand-built plans for edge
/// cases the analyzer doesn't generate naturally.
pub fn apply_plan(agent: &IrAgent, plan: &OwnershipPlan) -> IrAgent {
    apply_plan_owned(agent.clone(), plan, next_local_id(agent))
}

fn apply_plan_owned(mut out: IrAgent, plan: &OwnershipPlan, mut next_local: u32) -> IrAgent {

    // Group insertions by IR-tree position. Each map is
    //   IrPath (without the trailing Stmt step) → Vec<(stmt_idx, IrStmt)>
    // sorted by stmt_idx DESCENDING so we insert from the back of the
    // block forward, keeping earlier indices stable.
    let mut group: BTreeMap<IrPath, Vec<(usize, IrStmt, InsertionSide)>> = BTreeMap::new();
    // Hoist instructions: for each Return whose expr reads any of
    // the drop-after locals, replace the expr with a bare Local ref
    // to a fresh tmp and insert a preceding Let + Drop sequence.
    // Separated from `group` because the hoist requires mutating the
    // Return statement itself, not just inserting siblings.
    let mut hoists: Vec<HoistPlan> = Vec::new();

    for (pp, locals) in &plan.dups {
        if let Some(path) = plan.ir_paths.get(pp) {
            let (parent, idx) = split_path(path);
            for &l in locals {
                group.entry(parent.clone()).or_default().push((
                    idx,
                    IrStmt::Dup {
                        local_id: l,
                        span: zero_span(),
                    },
                    InsertionSide::Before,
                ));
            }
        }
    }
    for (pp, locals) in &plan.drops_after {
        if let Some(path) = plan.ir_paths.get(pp) {
            let (parent, idx) = split_path(path);
            // A drop_after on a Return statement is unreachable if we
            // place it after the Return (never executes) and UAF-
            // hazardous if we place it before (return's expr still
            // reads the local). Two cases:
            //
            //   - If the Return's expr does NOT reference any of the
            //     locals being dropped, drop-before-Return is safe.
            //     Promote to Before.
            //   - If it DOES reference any, hoist the expr into a
            //     synthetic `Let tmp = <expr>; Drop <locals>; Return tmp`.
            //     The Drops then land after the expr has evaluated
            //     but before the return instruction fires.
            let target_block = find_block(&out.body, &parent);
            let return_expr = target_block
                .and_then(|b| b.stmts.get(idx))
                .and_then(|s| match s {
                    IrStmt::Return { value: Some(e), .. } => Some(e.clone()),
                    _ => None,
                });

            if let Some(expr) = return_expr {
                let needs_hoist = locals.iter().any(|&l| expr_reads_local(&expr, l));
                if needs_hoist {
                    let tmp_id = LocalId(next_local);
                    next_local += 1;
                    hoists.push(HoistPlan {
                        parent: parent.clone(),
                        return_idx: idx,
                        tmp_id,
                        drop_locals: locals.iter().copied().collect(),
                    });
                    continue;
                }
                // Non-reading Return: safe to drop-before.
                for &l in locals {
                    group.entry(parent.clone()).or_default().push((
                        idx,
                        IrStmt::Drop {
                            local_id: l,
                            span: zero_span(),
                        },
                        InsertionSide::Before,
                    ));
                }
                continue;
            }
            // Not a Return with a value-expr: normal drop-after.
            for &l in locals {
                group.entry(parent.clone()).or_default().push((
                    idx,
                    IrStmt::Drop {
                        local_id: l,
                        span: zero_span(),
                    },
                    InsertionSide::After,
                ));
            }
        }
    }

    // Edge drops: a local that dies on entry to an If branch or a loop
    // body is released at the start of that block, before its first
    // statement, so the drop runs on every path through the block.
    for (if_path, drops) in &plan.branch_drops {
        let mut then_path = if_path.clone();
        then_path.push(IrNavStep::IfThen);
        push_entry_drops(&mut group, then_path, &drops.then_drops);
        if drops.fallthrough_drops.is_empty() {
            continue;
        }
        if if_has_else(&out.body, if_path) {
            let mut else_path = if_path.clone();
            else_path.push(IrNavStep::IfElse);
            push_entry_drops(&mut group, else_path, &drops.fallthrough_drops);
        } else {
            // No statements to shift: the synthesized else holds only the drops.
            synthesize_else_drops(&mut out.body, if_path, &drops.fallthrough_drops);
        }
    }
    for (loop_path, locals) in &plan.loop_entry_drops {
        if let Some(step) = loop_body_step(&out.body, loop_path) {
            let mut body_path = loop_path.clone();
            body_path.push(step);
            push_entry_drops(&mut group, body_path, locals);
        }
    }

    // Sort each group's insertions: by (idx desc, side desc-so-After-first-then-Before).
    // Inserting from the back means earlier positions don't shift. At
    // the SAME idx, we want After insertions to land at idx+1 BEFORE
    // any Before insertions land at idx; otherwise the After's "idx+1"
    // becomes wrong once a Before shifts everything by one.
    // Concrete order:
    //   1. Process highest idx first.
    //   2. At equal idx, do `After` before `Before` (After inserts at
    //      idx+1; Before inserts at idx; processing After first leaves
    //      idx untouched for the subsequent Before insertion).
    for inserts in group.values_mut() {
        inserts.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| match (a.2, b.2) {
                (InsertionSide::After, InsertionSide::Before) => std::cmp::Ordering::Less,
                (InsertionSide::Before, InsertionSide::After) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
        });
    }

    // Return hoists use original statement coordinates. Apply them
    // before inserting siblings that would shift those coordinates.
    hoists.sort_by(|a, b| b.return_idx.cmp(&a.return_idx));
    for hoist in hoists {
        apply_hoist(&mut out.body, &hoist);
    }

    // A parent path is a prefix of each child path. Visit children first
    // so inserting statements in a parent never invalidates a child index.
    for (parent, inserts) in group.into_iter().rev() {
        let target = navigate_mut(&mut out.body, &parent);
        for (idx, stmt, side) in inserts {
            let insert_at = match side {
                InsertionSide::Before => idx,
                InsertionSide::After => idx + 1,
            };
            // Bound-check: if idx is out of range (shouldn't happen
            // for a well-formed plan), skip rather than panic.
            if insert_at <= target.stmts.len() {
                target.stmts.insert(insert_at, stmt);
            }
        }
    }

    // Block-exit drops: only handle the entry block (block 0 in the
    // CFG). Append `IrStmt::Drop` to the end of the agent body, but
    // place it BEFORE any trailing Return so the drop actually
    // executes.
    if let Some(locals) = plan.drops_at_block_exit.get(&0) {
        let stmts = &mut out.body.stmts;
        let ret_pos = stmts
            .iter()
            .rposition(|s| matches!(s, IrStmt::Return { .. }));
        let insert_at = ret_pos.unwrap_or(stmts.len());
        // Iterate in stable order (BTreeSet) for deterministic output.
        for &l in locals {
            stmts.insert(
                insert_at,
                IrStmt::Drop {
                    local_id: l,
                    span: zero_span(),
                },
            );
        }
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionSide {
    Before,
    After,
}

/// Recipe for hoisting a Return's expression into a synthetic Let
/// so drops can fire between expr eval and the actual return.
///
/// Before:                     After:
///   Return { expr: E }          Let tmp = E
///                               Drop drop_locals...
///                               Return { expr: Local(tmp) }
struct HoistPlan {
    /// Parent path leading to the block containing the Return.
    parent: IrPath,
    /// Index of the Return statement within that block.
    return_idx: usize,
    /// Fresh LocalId allocated for the synthetic tmp binding.
    tmp_id: LocalId,
    /// Locals to Drop between the Let and the Return.
    drop_locals: Vec<LocalId>,
}

/// Scan the agent's body to find the highest LocalId already in use,
/// plus parameter IDs. Returns `max + 1` as the starting point for
/// synthetic-Let fresh allocations.
///
/// Must cover every IR shape that can bind or reference a LocalId:
/// params, Let.local_id, For.var_local, Dup.local_id, Drop.local_id,
/// and any IrExprKind::Local's local_id inside nested expressions.
fn next_local_id(agent: &IrAgent) -> u32 {
    let mut max_id: u32 = 0;
    for p in &agent.params {
        if p.local_id.0 > max_id {
            max_id = p.local_id.0;
        }
    }
    scan_block(&agent.body, &mut max_id);
    max_id + 1
}

fn scan_block(block: &IrBlock, max_id: &mut u32) {
    for stmt in &block.stmts {
        scan_stmt(stmt, max_id);
    }
}

fn scan_stmt(stmt: &IrStmt, max_id: &mut u32) {
    match stmt {
        IrStmt::Assign {
            local_id,
            path,
            value,
            ..
        } => {
            if local_id.0 != u32::MAX && local_id.0 > *max_id {
                *max_id = local_id.0;
            }
            for seg in path {
                if let corvid_ir::IrPathSeg::Index(idx) = seg {
                    scan_expr(idx, max_id);
                }
            }
            scan_expr(value, max_id);
        }
        IrStmt::Let {
            local_id, value, ..
        } => {
            if local_id.0 > *max_id {
                *max_id = local_id.0;
            }
            scan_expr(value, max_id);
        }
        IrStmt::Return { value: Some(e), .. } => scan_expr(e, max_id),
        IrStmt::Return { value: None, .. } => {}
        IrStmt::Yield { value, .. } => scan_expr(value, max_id),
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            scan_expr(cond, max_id);
            scan_block(then_block, max_id);
            if let Some(eb) = else_block {
                scan_block(eb, max_id);
            }
        }
        IrStmt::For {
            var_local,
            iter,
            body,
            ..
        } => {
            if var_local.0 > *max_id {
                *max_id = var_local.0;
            }
            scan_expr(iter, max_id);
            scan_block(body, max_id);
        }
        IrStmt::Atomic { body, .. } => {
            scan_block(body, max_id);
        }
        IrStmt::While { cond, body, .. } => {
            scan_expr(cond, max_id);
            scan_block(body, max_id);
        }
        IrStmt::Destructure { value, .. } => {
            scan_expr(value, max_id);
        }
        IrStmt::Parallel { arms, .. } => {
            for arm in arms {
                scan_expr(&arm.call, max_id);
            }
        }
        IrStmt::Expr { expr, .. } => scan_expr(expr, max_id),
        IrStmt::Approve { args, .. } => {
            for a in args {
                scan_expr(a, max_id);
            }
        }
        IrStmt::Dup { local_id, .. } | IrStmt::Drop { local_id, .. } => {
            if local_id.0 > *max_id {
                *max_id = local_id.0;
            }
        }
        IrStmt::Break { .. } | IrStmt::Continue { .. } | IrStmt::Pass { .. } => {}
    }
}

fn scan_expr(expr: &IrExpr, max_id: &mut u32) {
    match &expr.kind {
        IrExprKind::MapLiteral { keys, values } => {
            for e in keys.iter().chain(values) {
                scan_expr(e, max_id);
            }
        }
        IrExprKind::Match { scrutinee, arms } => {
            scan_expr(scrutinee, max_id);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    scan_expr(g, max_id);
                }
                scan_expr(&arm.body, max_id);
            }
        }
        IrExprKind::Lambda { body, .. } => {
            scan_expr(body, max_id);
        }
        IrExprKind::StructLiteral { fields, spread, .. } => {
            for (_, v) in fields {
                scan_expr(v, max_id);
            }
            if let Some(s) = spread {
                scan_expr(s, max_id);
            }
        }
        IrExprKind::PageNew { items, next_cursor } => {
            scan_expr(items, max_id);
            scan_expr(next_cursor, max_id);
        }
        IrExprKind::TableOp { args, .. } => {
            for arg in args {
                scan_expr(arg, max_id);
            }
        }
        IrExprKind::TableQuery { filter, take, .. } => {
            for e in corvid_ir::table_query_operands(filter, take) {
                scan_expr(e, max_id);
            }
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            scan_expr(receiver, max_id);
            for a in args {
                scan_expr(a, max_id);
            }
        }
        IrExprKind::Local { local_id, .. } => {
            if local_id.0 > *max_id {
                *max_id = local_id.0;
            }
        }
        IrExprKind::Literal(_) | IrExprKind::Decl { .. } | IrExprKind::OptionNone | IrExprKind::Outage(corvid_ir::IrOutage::Pending | corvid_ir::IrOutage::Clear | corvid_ir::IrOutage::Unwind | corvid_ir::IrOutage::DeadlineBegin(_) | corvid_ir::IrOutage::DeadlineEnd | corvid_ir::IrOutage::DeadlineDepth) => {}
        IrExprKind::FieldAccess { target, .. } => scan_expr(target, max_id),
        IrExprKind::UnwrapGrounded { value } => scan_expr(value, max_id),
        IrExprKind::Index { target, index } => {
            scan_expr(target, max_id);
            scan_expr(index, max_id);
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            scan_expr(left, max_id);
            scan_expr(right, max_id);
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            scan_expr(operand, max_id)
        }
        IrExprKind::Call { args, .. } => {
            for a in args {
                scan_expr(a, max_id);
            }
        }
        IrExprKind::List { items } => {
            for item in items {
                scan_expr(item, max_id);
            }
        }
        IrExprKind::WeakNew { strong } => scan_expr(strong, max_id),
        IrExprKind::WeakUpgrade { weak } => scan_expr(weak, max_id),
        IrExprKind::StreamSplitBy { stream, .. } => scan_expr(stream, max_id),
        IrExprKind::StreamMerge { groups, .. } => scan_expr(groups, max_id),
        IrExprKind::StreamOrderedBy { stream, .. } => scan_expr(stream, max_id),
        IrExprKind::StreamResumeToken { stream } => scan_expr(stream, max_id),
        IrExprKind::ResumeStream { token, .. } => scan_expr(token, max_id),
        IrExprKind::ResultOk { inner }
        | IrExprKind::ResultErr { inner }
        | IrExprKind::OptionSome { inner }
        | IrExprKind::Ask { prompt: inner, .. }
        | IrExprKind::Choose { options: inner }
        | IrExprKind::TryPropagate { inner } => scan_expr(inner, max_id),
        IrExprKind::TrustBoundary { inner: body, .. } | IrExprKind::Outage(corvid_ir::IrOutage::Checked(body) | corvid_ir::IrOutage::PendingFor(body)) | IrExprKind::TryRetry { body, .. } => {
            scan_expr(body, max_id)
        }
        IrExprKind::Replay {
            trace,
            arms,
            else_body,
        } => {
            scan_expr(trace, max_id);
            for arm in arms {
                if let Some(capture) = &arm.capture {
                    if capture.local_id.0 > *max_id {
                        *max_id = capture.local_id.0;
                    }
                }
                if let corvid_ir::IrReplayPattern::Tool {
                    arg: corvid_ir::IrReplayToolArgPattern::Capture(capture),
                    ..
                } = &arm.pattern
                {
                    if capture.local_id.0 > *max_id {
                        *max_id = capture.local_id.0;
                    }
                }
                scan_expr(&arm.body, max_id);
            }
            scan_expr(else_body, max_id);
        }
    }
}

/// Walk `expr` and return true if it contains any read of `target`.
/// Used to decide whether a Return's expr forces a synthetic-Let hoist.
fn expr_reads_local(expr: &IrExpr, target: LocalId) -> bool {
    match &expr.kind {
        IrExprKind::MapLiteral { keys, values } => keys
            .iter()
            .chain(values)
            .any(|e| expr_reads_local(e, target)),
        IrExprKind::Match { scrutinee, arms } => {
            expr_reads_local(scrutinee, target)
                || arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .is_some_and(|g| expr_reads_local(g, target))
                        || expr_reads_local(&arm.body, target)
                })
        }
        IrExprKind::Lambda { body, .. } => expr_reads_local(body, target),
        IrExprKind::StructLiteral { fields, spread, .. } => {
            fields.iter().any(|(_, v)| expr_reads_local(v, target))
                || spread.as_ref().is_some_and(|s| expr_reads_local(s, target))
        }
        IrExprKind::PageNew { items, next_cursor } => {
            expr_reads_local(items, target) || expr_reads_local(next_cursor, target)
        }
        IrExprKind::TableOp { args, .. } => args.iter().any(|arg| expr_reads_local(arg, target)),
        IrExprKind::TableQuery { filter, take, .. } => corvid_ir::table_query_operands(filter, take).into_iter().any(|e| expr_reads_local(e, target)),
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            expr_reads_local(receiver, target) || args.iter().any(|a| expr_reads_local(a, target))
        }
        IrExprKind::Local { local_id, .. } => *local_id == target,
        IrExprKind::Literal(_) | IrExprKind::Decl { .. } | IrExprKind::OptionNone | IrExprKind::Outage(corvid_ir::IrOutage::Pending | corvid_ir::IrOutage::Clear | corvid_ir::IrOutage::Unwind | corvid_ir::IrOutage::DeadlineBegin(_) | corvid_ir::IrOutage::DeadlineEnd | corvid_ir::IrOutage::DeadlineDepth) => false,
        IrExprKind::FieldAccess { target: t, .. } => expr_reads_local(t, target),
        IrExprKind::UnwrapGrounded { value } => expr_reads_local(value, target),
        IrExprKind::Index { target: t, index } => {
            expr_reads_local(t, target) || expr_reads_local(index, target)
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            expr_reads_local(left, target) || expr_reads_local(right, target)
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            expr_reads_local(operand, target)
        }
        IrExprKind::Call { kind, args, .. } => {
            matches!(kind, corvid_ir::IrCallKind::ClosureLocal { local_id, .. } if *local_id == target)
                || args.iter().any(|arg| expr_reads_local(arg, target))
        }
        IrExprKind::List { items } => items.iter().any(|it| expr_reads_local(it, target)),
        IrExprKind::WeakNew { strong } => expr_reads_local(strong, target),
        IrExprKind::WeakUpgrade { weak } => expr_reads_local(weak, target),
        IrExprKind::StreamSplitBy { stream, .. } => expr_reads_local(stream, target),
        IrExprKind::StreamMerge { groups, .. } => expr_reads_local(groups, target),
        IrExprKind::StreamOrderedBy { stream, .. } => expr_reads_local(stream, target),
        IrExprKind::StreamResumeToken { stream } => expr_reads_local(stream, target),
        IrExprKind::ResumeStream { token, .. } => expr_reads_local(token, target),
        IrExprKind::ResultOk { inner }
        | IrExprKind::ResultErr { inner }
        | IrExprKind::OptionSome { inner }
        | IrExprKind::Ask { prompt: inner, .. }
        | IrExprKind::Choose { options: inner }
        | IrExprKind::TryPropagate { inner } => expr_reads_local(inner, target),
        IrExprKind::TrustBoundary { inner: body, .. } | IrExprKind::Outage(corvid_ir::IrOutage::Checked(body) | corvid_ir::IrOutage::PendingFor(body)) | IrExprKind::TryRetry { body, .. } => {
            expr_reads_local(body, target)
        }
        IrExprKind::Replay {
            trace,
            arms,
            else_body,
        } => {
            expr_reads_local(trace, target)
                || arms.iter().any(|arm| expr_reads_local(&arm.body, target))
                || expr_reads_local(else_body, target)
        }
    }
}

/// Execute one HoistPlan against the IR tree. Replaces `Return { E }`
/// with `[Let tmp = E, Drop locals..., Return { Local(tmp) }]`.
fn apply_hoist(root: &mut IrBlock, hoist: &HoistPlan) {
    let block = navigate_mut(root, &hoist.parent);
    let ret_idx = hoist.return_idx;
    // Bound-check: if the block was rewritten between plan construction
    // and hoist application, bail out rather than panic.
    if ret_idx >= block.stmts.len() {
        return;
    }
    let Some(orig_expr) = (match &block.stmts[ret_idx] {
        IrStmt::Return { value: Some(e), .. } => Some(e.clone()),
        _ => None,
    }) else {
        return;
    };
    let return_span = match &block.stmts[ret_idx] {
        IrStmt::Return { span, .. } => *span,
        _ => zero_span(),
    };
    let expr_ty = orig_expr.ty.clone();
    let expr_span = orig_expr.span;

    // Build the replacement sequence.
    //   Let tmp = <orig>
    //   Drop d0; Drop d1; ...
    //   Return Local(tmp)
    let let_stmt = IrStmt::Let {
        local_id: hoist.tmp_id,
        name: "__corvid_ret_tmp".to_string(),
        ty: expr_ty.clone(),
        value: orig_expr,
        span: return_span,
    };
    let mut drop_stmts: Vec<IrStmt> = hoist
        .drop_locals
        .iter()
        .map(|&l| IrStmt::Drop {
            local_id: l,
            span: zero_span(),
        })
        .collect();
    let new_return = IrStmt::Return {
        value: Some(IrExpr {
            kind: IrExprKind::Local {
                local_id: hoist.tmp_id,
                name: "__corvid_ret_tmp".to_string(),
            },
            ty: expr_ty,
            span: expr_span,
        }),
        span: return_span,
    };

    // Splice: replace block.stmts[ret_idx] with [let, drops..., return].
    block.stmts.remove(ret_idx);
    let mut insert_at = ret_idx;
    block.stmts.insert(insert_at, let_stmt);
    insert_at += 1;
    for drop_stmt in drop_stmts.drain(..) {
        block.stmts.insert(insert_at, drop_stmt);
        insert_at += 1;
    }
    block.stmts.insert(insert_at, new_return);
}

/// Split a navigation path into (parent_path, final_stmt_idx).
fn split_path(path: &IrPath) -> (IrPath, usize) {
    let mut parent: IrPath = path.iter().cloned().collect();
    let last = parent.pop().expect("IrPath always ends in Stmt(_)");
    let idx = match last {
        IrNavStep::Stmt(i) => i,
        _ => panic!("malformed IrPath — last step must be Stmt"),
    };
    (parent, idx)
}

/// Navigate from the root `IrBlock` down through nested If/For
/// children to the block containing the target statement. Mutable
/// reference; caller mutates `target.stmts`.
/// Read-only analog of `navigate_mut`. Returns None if the path
/// doesn't match the current IR shape (e.g., caller built the path
/// from a stale CFG). Callers treat None as "can't determine" and
/// fall back to the default insertion side.
fn find_block<'a>(root: &'a IrBlock, parent: &IrPath) -> Option<&'a IrBlock> {
    let mut current: &'a IrBlock = root;
    let mut i = 0;
    while i < parent.len() {
        let stmt_step = &parent[i];
        let descend_step = parent.get(i + 1)?;
        let stmt_idx = match stmt_step {
            IrNavStep::Stmt(s) => *s,
            _ => return None,
        };
        let stmt = current.stmts.get(stmt_idx)?;
        current = match (stmt, descend_step) {
            (IrStmt::If { then_block, .. }, IrNavStep::IfThen) => then_block,
            (
                IrStmt::If {
                    else_block: Some(eb),
                    ..
                },
                IrNavStep::IfElse,
            ) => eb,
            (IrStmt::For { body, .. }, IrNavStep::ForBody) => body,
            (IrStmt::While { body, .. }, IrNavStep::WhileBody) => body,
            _ => return None,
        };
        i += 2;
    }
    Some(current)
}

fn navigate_mut<'a>(root: &'a mut IrBlock, parent: &IrPath) -> &'a mut IrBlock {
    let mut current: &'a mut IrBlock = root;
    let mut i = 0;
    while i < parent.len() {
        // Each "descent" consumes a Stmt step (telling which child of
        // the current block to enter) followed by an If/For step
        // (which child block of that statement).
        let stmt_step = &parent[i];
        let descend_step = parent.get(i + 1);
        let stmt_idx = match stmt_step {
            IrNavStep::Stmt(s) => *s,
            _ => panic!("malformed IrPath — descent without preceding Stmt"),
        };
        let descend = match descend_step {
            Some(s) => s.clone(),
            None => panic!("malformed IrPath — Stmt with no descent step"),
        };
        let stmt = &mut current.stmts[stmt_idx];
        current = match (stmt, descend) {
            (IrStmt::If { then_block, .. }, IrNavStep::IfThen) => then_block,
            (
                IrStmt::If {
                    else_block: Some(eb),
                    ..
                },
                IrNavStep::IfElse,
            ) => eb,
            (IrStmt::For { body, .. }, IrNavStep::ForBody) => body,
            (IrStmt::While { body, .. }, IrNavStep::WhileBody) => body,
            _ => panic!("IrPath descent doesn't match IR shape"),
        };
        i += 2;
    }
    current
}

fn zero_span() -> Span {
    Span::new(0, 0)
}

/// Queue `Drop`s for `locals` at the start of the block `block_path`
/// leads into (an If branch or a loop body).
fn push_entry_drops(
    group: &mut BTreeMap<IrPath, Vec<(usize, IrStmt, InsertionSide)>>,
    block_path: IrPath,
    locals: &std::collections::BTreeSet<LocalId>,
) {
    if locals.is_empty() {
        return;
    }
    let inserts = group.entry(block_path).or_default();
    for &l in locals {
        inserts.push((
            0,
            IrStmt::Drop {
                local_id: l,
                span: zero_span(),
            },
            InsertionSide::Before,
        ));
    }
}

/// The statement `path` points at, if the path still matches the IR.
fn stmt_at<'a>(root: &'a IrBlock, path: &IrPath) -> Option<&'a IrStmt> {
    let (parent, idx) = split_path(path);
    find_block(root, &parent)?.stmts.get(idx)
}

fn if_has_else(root: &IrBlock, if_path: &IrPath) -> bool {
    matches!(stmt_at(root, if_path), Some(IrStmt::If { else_block: Some(_), .. }))
}

/// The navigation step into the body of the loop `loop_path` points at.
fn loop_body_step(root: &IrBlock, loop_path: &IrPath) -> Option<IrNavStep> {
    match stmt_at(root, loop_path)? {
        IrStmt::For { .. } => Some(IrNavStep::ForBody),
        IrStmt::While { .. } => Some(IrNavStep::WhileBody),
        _ => None,
    }
}

/// Give an If without an else branch one that holds only `Drop`s for
/// `locals`, the locals that die when the condition is false.
fn synthesize_else_drops(
    root: &mut IrBlock,
    if_path: &IrPath,
    locals: &std::collections::BTreeSet<LocalId>,
) {
    let (parent, idx) = split_path(if_path);
    let parent_block = navigate_mut(root, &parent);
    let Some(IrStmt::If { else_block, span, .. }) = parent_block.stmts.get_mut(idx) else {
        return; // stale path
    };
    *else_block = Some(IrBlock {
        stmts: locals
            .iter()
            .map(|&l| IrStmt::Drop {
                local_id: l,
                span: zero_span(),
            })
            .collect(),
        span: *span,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ast::BinaryOp;
    use corvid_ir::{IrAgent, IrBlock, IrExpr, IrExprKind, IrParam, IrStmt};
    use corvid_resolve::{DefId, LocalId};
    use corvid_types::Type;

    fn span() -> Span {
        Span::new(0, 0)
    }

    fn local_expr(id: u32, ty: Type) -> IrExpr {
        IrExpr {
            kind: IrExprKind::Local {
                local_id: LocalId(id),
                name: format!("l{id}"),
            },
            ty,
            span: span(),
        }
    }

    fn make_agent(params: Vec<(u32, Type)>, body: Vec<IrStmt>, ret: Type) -> IrAgent {
        IrAgent {
            id: DefId(0),
            name: "test".into(),
            extern_abi: None,
            params: params
                .into_iter()
                .map(|(id, ty)| IrParam {
                    name: format!("p{id}"),
                    local_id: LocalId(id),
                    ty,
                    span: span(),
                })
                .collect(),
            return_ty: ret,
            cost_budget: None,
            wrapping_arithmetic: false,
            is_replayable: false,
            pure_fn: false,
            retry_max_attempts: None,
            retry_backoff_ms: None,
            idempotency_key_param: None,
            body: IrBlock {
                stmts: body,
                span: span(),
            },
            span: span(),
            borrow_sig: None,
        }
    }

    /// `let t = s + s; return t` → first read of `s` in the let needs
    /// a Dup. Result IR: [Dup s, Let t = s+s, Return t].
    #[test]
    fn dup_inserted_before_non_last_use() {
        let agent = make_agent(
            vec![(0, Type::String)],
            vec![
                IrStmt::Let {
                    local_id: LocalId(1),
                    name: "t".into(),
                    ty: Type::String,
                    value: IrExpr {
                        kind: IrExprKind::BinOp {
                            op: BinaryOp::Add,
                            left: Box::new(local_expr(0, Type::String)),
                            right: Box::new(local_expr(0, Type::String)),
                        },
                        ty: Type::String,
                        span: span(),
                    },
                    span: span(),
                },
                IrStmt::Return {
                    value: Some(local_expr(1, Type::String)),
                    span: span(),
                },
            ],
            Type::String,
        );
        let out = insert_dup_drop(&agent);
        assert_eq!(
            out.body.stmts.len(),
            3,
            "Dup should have been inserted before the Let"
        );
        assert!(matches!(
            out.body.stmts[0],
            IrStmt::Dup {
                local_id: LocalId(0),
                ..
            }
        ));
        assert!(matches!(out.body.stmts[1], IrStmt::Let { .. }));
        assert!(matches!(out.body.stmts[2], IrStmt::Return { .. }));
    }

    /// Unused String param → Drop appears at the end of the body,
    /// BEFORE the Return.
    #[test]
    fn unused_param_drops_before_return() {
        let agent = make_agent(
            vec![(0, Type::String)],
            vec![IrStmt::Return {
                value: Some(IrExpr {
                    kind: IrExprKind::Literal(corvid_ir::IrLiteral::Int(0)),
                    ty: Type::Int,
                    span: span(),
                }),
                span: span(),
            }],
            Type::Int,
        );
        let out = insert_dup_drop(&agent);
        assert_eq!(out.body.stmts.len(), 2);
        assert!(
            matches!(
                out.body.stmts[0],
                IrStmt::Drop {
                    local_id: LocalId(0),
                    ..
                }
            ),
            "unused param drop comes before return"
        );
        assert!(matches!(out.body.stmts[1], IrStmt::Return { .. }));
    }

    /// `let t = s; return 0` (t never used) → Drop t after the Let,
    /// before the Return.
    #[test]
    fn defined_unused_drops_after_let() {
        let agent = make_agent(
            vec![(0, Type::String)],
            vec![
                IrStmt::Let {
                    local_id: LocalId(1),
                    name: "t".into(),
                    ty: Type::String,
                    value: local_expr(0, Type::String),
                    span: span(),
                },
                IrStmt::Return {
                    value: Some(IrExpr {
                        kind: IrExprKind::Literal(corvid_ir::IrLiteral::Int(0)),
                        ty: Type::Int,
                        span: span(),
                    }),
                    span: span(),
                },
            ],
            Type::Int,
        );
        let out = insert_dup_drop(&agent);
        // Expected order: [Let t, Drop t, Return 0]. Param 0 (s) is
        // consumed by the Let (last use of s), so no extra Dup needed.
        assert_eq!(out.body.stmts.len(), 3);
        assert!(matches!(out.body.stmts[0], IrStmt::Let { .. }));
        assert!(matches!(
            out.body.stmts[1],
            IrStmt::Drop {
                local_id: LocalId(1),
                ..
            }
        ));
        assert!(matches!(out.body.stmts[2], IrStmt::Return { .. }));
    }

    /// `return s` where s is the bare last use → no insertions.
    #[test]
    fn no_op_for_already_optimal() {
        let agent = make_agent(
            vec![(0, Type::String)],
            vec![IrStmt::Return {
                value: Some(local_expr(0, Type::String)),
                span: span(),
            }],
            Type::String,
        );
        let out = insert_dup_drop(&agent);
        assert_eq!(out.body.stmts.len(), 1, "no Dup or Drop should be inserted");
    }

    /// Deterministic output: two runs on equal IR produce equal IR.
    #[test]
    fn determinism_two_runs() {
        let mk = || {
            make_agent(
                vec![(0, Type::String)],
                vec![
                    IrStmt::Let {
                        local_id: LocalId(1),
                        name: "t".into(),
                        ty: Type::String,
                        value: local_expr(0, Type::String),
                        span: span(),
                    },
                    IrStmt::Return {
                        value: Some(local_expr(1, Type::String)),
                        span: span(),
                    },
                ],
                Type::String,
            )
        };
        let a = insert_dup_drop(&mk());
        let b = insert_dup_drop(&mk());
        assert_eq!(a.body.stmts.len(), b.body.stmts.len());
        for (sa, sb) in a.body.stmts.iter().zip(b.body.stmts.iter()) {
            // Compare structural variant — Spans we use are zero so
            // matching the tag is enough for determinism evidence.
            assert_eq!(
                std::mem::discriminant(sa),
                std::mem::discriminant(sb),
                "two runs produced different IR shape"
            );
        }
    }

    /// Insertion inside an `if` branch: `if cond: let t = s + s` —
    /// the Dup must land inside the then-block, not at the top level.
    #[test]
    fn dup_inside_then_branch() {
        let agent = make_agent(
            vec![(0, Type::String), (1, Type::Bool)],
            vec![
                IrStmt::If {
                    cond: local_expr(1, Type::Bool),
                    then_block: IrBlock {
                        stmts: vec![IrStmt::Let {
                            local_id: LocalId(2),
                            name: "t".into(),
                            ty: Type::String,
                            value: IrExpr {
                                kind: IrExprKind::BinOp {
                                    op: BinaryOp::Add,
                                    left: Box::new(local_expr(0, Type::String)),
                                    right: Box::new(local_expr(0, Type::String)),
                                },
                                ty: Type::String,
                                span: span(),
                            },
                            span: span(),
                        }],
                        span: span(),
                    },
                    else_block: None,
                    span: span(),
                },
                IrStmt::Return {
                    value: Some(IrExpr {
                        kind: IrExprKind::Literal(corvid_ir::IrLiteral::Int(0)),
                        ty: Type::Int,
                        span: span(),
                    }),
                    span: span(),
                },
            ],
            Type::Int,
        );
        let out = insert_dup_drop(&agent);
        // Top level should still be just [If, Return] — Dup is inside.
        assert!(matches!(out.body.stmts[0], IrStmt::If { .. }));
        if let IrStmt::If { then_block, .. } = &out.body.stmts[0] {
            assert!(
                then_block.stmts.iter().any(|s| matches!(
                    s,
                    IrStmt::Dup {
                        local_id: LocalId(0),
                        ..
                    }
                )),
                "Dup of param 0 should land inside the then-block"
            );
        }
    }
}
