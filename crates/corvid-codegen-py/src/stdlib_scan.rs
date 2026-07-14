//! Pre-transpile scan for stdlib executing tools.
//!
//! The Python transpile tier has no dispatch for the stdlib's
//! executing tools (io/http/db/json/time/random/rag/mcp): a
//! transpiled `await tool_call("io_read_text", ...)` would reach an
//! empty tool registry and fail at runtime, far from the cause, and
//! opaque stdlib types (`DbHandle`, `JsonValue`) would degrade to
//! `object`-typed hints. This scan lets the driver refuse LOUDLY at
//! transpile time instead: it walks the IR and reports every call
//! to a stdlib executing tool, with its span, so the diagnostic can
//! point at the exact call and route the user to the interpreter
//! tier (`corvid run`), which dispatches all of them.

use corvid_ast::Span;
use corvid_ir::{IrBlock, IrCallKind, IrExpr, IrExprKind, IrFile, IrStmt};

/// Every executing `tool` the stdlib declares, across all modules.
/// The interpreter tier dispatches these; the Python transpile tier
/// does not. Kept in lockstep with the `std/*.cor` sources by the
/// `stdlib_tool_list_matches_std_sources` test below — adding a
/// stdlib tool without updating this list fails CI.
pub const STDLIB_EXECUTING_TOOLS: &[&str] = &[
    // std/io
    "io_read_text",
    "io_write_text",
    "io_list_dir",
    // std/http
    "http_get",
    "http_post_json",
    // std/db
    "db_open",
    "db_query",
    "db_execute",
    // std/json
    "json_parse",
    "json_get_int",
    "json_get_float",
    "json_get_string",
    "json_get_bool",
    "json_get_object",
    "json_get_array",
    "json_object_new",
    "json_object_set_int",
    "json_object_set_float",
    "json_object_set_string",
    "json_object_set_bool",
    "json_object_finish",
    // std/secrets
    "secret_read",
    // std/cache
    "cache_put",
    "cache_get",
    "cache_invalidate",
    "cache_invalidate_provenance",
    // std/mcp
    "mcp_call",
    // std/rag
    "rag_ingest",
    "rag_search",
    // std/random
    "random_float",
    "random_int",
    // std/time
    "time_now_utc",
    "time_monotonic_ms",
    "time_parse_iso",
    "time_format_iso",
];

/// One stdlib-executing-tool call found in the IR.
#[derive(Debug, Clone)]
pub struct StdlibToolCall {
    pub tool: String,
    pub span: Span,
}

/// Walk the IR and collect every call to a stdlib executing tool.
/// An empty result means the program is transpile-safe with respect
/// to stdlib dispatch.
pub fn find_stdlib_executing_tool_calls(ir: &IrFile) -> Vec<StdlibToolCall> {
    let mut hits = Vec::new();
    for agent in &ir.agents {
        scan_block(&agent.body, &mut hits);
    }
    hits
}

fn scan_block(block: &IrBlock, hits: &mut Vec<StdlibToolCall>) {
    for stmt in &block.stmts {
        scan_stmt(stmt, hits);
    }
}

fn scan_stmt(stmt: &IrStmt, hits: &mut Vec<StdlibToolCall>) {
    match stmt {
        IrStmt::Let { value, .. } => scan_expr(value, hits),
        IrStmt::Assign { value, .. } => scan_expr(value, hits),
        IrStmt::Yield { value, .. } => scan_expr(value, hits),
        IrStmt::Return { value: Some(v), .. } => scan_expr(v, hits),
        IrStmt::Return { value: None, .. } => {}
        IrStmt::If {
            cond,
            then_block,
            else_block,
            ..
        } => {
            scan_expr(cond, hits);
            scan_block(then_block, hits);
            if let Some(b) = else_block {
                scan_block(b, hits);
            }
        }
        IrStmt::For { iter, body, .. } => {
            scan_expr(iter, hits);
            scan_block(body, hits);
        }
        IrStmt::While { cond, body, .. } => {
            scan_expr(cond, hits);
            scan_block(body, hits);
        }
        IrStmt::Destructure { value, .. } => scan_expr(value, hits),
        IrStmt::Parallel { arms, .. } => {
            for arm in arms {
                scan_expr(&arm.call, hits);
            }
        }
        IrStmt::Approve { args, .. } => {
            for a in args {
                scan_expr(a, hits);
            }
        }
        IrStmt::Expr { expr, .. } => scan_expr(expr, hits),
        IrStmt::Break { .. }
        | IrStmt::Continue { .. }
        | IrStmt::Pass { .. }
        | IrStmt::Dup { .. }
        | IrStmt::Drop { .. } => {}
    }
}

fn scan_expr(expr: &IrExpr, hits: &mut Vec<StdlibToolCall>) {
    match &expr.kind {
        IrExprKind::Call {
            kind,
            callee_name,
            args,
        } => {
            if matches!(kind, IrCallKind::Tool { .. })
                && STDLIB_EXECUTING_TOOLS.contains(&callee_name.as_str())
            {
                hits.push(StdlibToolCall {
                    tool: callee_name.clone(),
                    span: expr.span,
                });
            }
            for a in args {
                scan_expr(a, hits);
            }
        }
        IrExprKind::FieldAccess { target, .. } => scan_expr(target, hits),
        IrExprKind::UnwrapGrounded { value } => scan_expr(value, hits),
        IrExprKind::Index { target, index } => {
            scan_expr(target, hits);
            scan_expr(index, hits);
        }
        IrExprKind::BinOp { left, right, .. } | IrExprKind::WrappingBinOp { left, right, .. } => {
            scan_expr(left, hits);
            scan_expr(right, hits);
        }
        IrExprKind::UnOp { operand, .. } | IrExprKind::WrappingUnOp { operand, .. } => {
            scan_expr(operand, hits)
        }
        IrExprKind::List { items } => {
            for it in items {
                scan_expr(it, hits);
            }
        }
        IrExprKind::MapLiteral { keys, values } => {
            for k in keys {
                scan_expr(k, hits);
            }
            for v in values {
                scan_expr(v, hits);
            }
        }
        IrExprKind::Match { scrutinee, arms } => {
            scan_expr(scrutinee, hits);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    scan_expr(guard, hits);
                }
                scan_expr(&arm.body, hits);
            }
        }
        IrExprKind::Lambda { body, .. } => scan_expr(body, hits),
        IrExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                scan_expr(v, hits);
            }
        }
        IrExprKind::BuiltinMethod { receiver, args, .. } => {
            scan_expr(receiver, hits);
            for a in args {
                scan_expr(a, hits);
            }
        }
        IrExprKind::WeakNew { strong } => scan_expr(strong, hits),
        IrExprKind::WeakUpgrade { weak } => scan_expr(weak, hits),
        IrExprKind::OptionSome { inner }
        | IrExprKind::ResultOk { inner }
        | IrExprKind::ResultErr { inner }
        | IrExprKind::TryPropagate { inner } => scan_expr(inner, hits),
        IrExprKind::TryRetry { body, .. } => scan_expr(body, hits),
        IrExprKind::StreamSplitBy { stream, .. }
        | IrExprKind::StreamOrderedBy { stream, .. }
        | IrExprKind::StreamResumeToken { stream } => scan_expr(stream, hits),
        IrExprKind::StreamMerge { groups, .. } => scan_expr(groups, hits),
        IrExprKind::ResumeStream { token, .. } => scan_expr(token, hits),
        IrExprKind::Replay { .. }
        | IrExprKind::Ask { .. }
        | IrExprKind::Choose { .. }
        | IrExprKind::Literal(_)
        | IrExprKind::Local { .. }
        | IrExprKind::Decl { .. }
        | IrExprKind::OptionNone => {}
    }
}

#[cfg(test)]
mod tests {
    use super::STDLIB_EXECUTING_TOOLS;

    /// Anti-drift gate: the tool list above must exactly match the
    /// `public tool` declarations in the repo's `std/*.cor` sources.
    /// A stdlib slice that adds an executing tool without updating
    /// the transpile scan fails here.
    #[test]
    fn stdlib_tool_list_matches_std_sources() {
        let std_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("repo root is two parents up")
            .join("std");
        let mut declared: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&std_dir).expect("std dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("cor") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("std module source");
            for line in source.lines() {
                let trimmed = line.trim_start();
                let Some(rest) = trimmed
                    .strip_prefix("public tool ")
                    .or_else(|| trimmed.strip_prefix("tool "))
                else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    declared.push(name);
                }
            }
        }
        declared.sort();
        let mut listed: Vec<String> = STDLIB_EXECUTING_TOOLS
            .iter()
            .map(|s| s.to_string())
            .collect();
        listed.sort();
        assert_eq!(
            listed, declared,
            "STDLIB_EXECUTING_TOOLS drifted from the std/*.cor tool declarations"
        );
    }
}
