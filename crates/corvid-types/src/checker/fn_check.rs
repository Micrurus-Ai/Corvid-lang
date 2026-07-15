//! `fn` pure-function checking (slice 45r).
//!
//! A `fn` body must be statically EFFECT-FREE: it may call other
//! `fn`s and pure builtins (constructors, `range`, the
//! builtin-method table), but no tools, prompts, agents, or
//! fixtures, no `approve`, no `ask`/`choose`, no `replay` blocks,
//! and no `yield`. That guarantee is what makes a `fn` callable
//! from `@deterministic` contexts with zero ceremony.

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{Block, Expr, FnDecl, Stmt};
use corvid_resolve::{Binding, BuiltIn, DeclKind};

impl<'a> Checker<'a> {
    pub(super) fn check_fn(&mut self, f: &FnDecl) {
        self.bind_params(&f.params);
        let declared_ret = self.type_ref_to_type(&f.return_ty);
        let prev_ret = std::mem::replace(&mut self.current_return, Some(declared_ret));
        let prev_in_agent = std::mem::replace(&mut self.in_agent_body, true);
        let prev_saw_yield = std::mem::replace(&mut self.saw_yield, false);
        self.check_block(&f.body);
        self.current_return = prev_ret;
        self.in_agent_body = prev_in_agent;
        self.saw_yield = prev_saw_yield;

        self.walk_fn_purity_block(&f.name.name, &f.body);
    }

    fn fn_purity_violation(&mut self, fn_name: &str, what: &str, span: corvid_ast::Span) {
        self.errors.push(TypeError::new(
            TypeErrorKind::FnBodyNotPure {
                name: fn_name.to_string(),
                what: what.to_string(),
            },
            span,
        ));
    }

    fn walk_fn_purity_block(&mut self, fn_name: &str, block: &Block) {
        for stmt in &block.stmts {
            self.walk_fn_purity_stmt(fn_name, stmt);
        }
    }

    fn walk_fn_purity_stmt(&mut self, fn_name: &str, stmt: &Stmt) {
        match stmt {
            Stmt::Approve { span, .. } => {
                self.fn_purity_violation(fn_name, "an `approve` gate", *span);
            }
            Stmt::Yield { span, .. } => {
                self.fn_purity_violation(fn_name, "a `yield` (fns are not streams)", *span);
            }
            Stmt::Let { value, .. } => self.walk_fn_purity_expr(fn_name, value),
            Stmt::Assign { target, value, .. } => {
                self.walk_fn_purity_expr(fn_name, target);
                self.walk_fn_purity_expr(fn_name, value);
            }
            Stmt::Expr { expr, .. } => self.walk_fn_purity_expr(fn_name, expr),
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.walk_fn_purity_expr(fn_name, v);
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                self.walk_fn_purity_expr(fn_name, cond);
                self.walk_fn_purity_block(fn_name, then_block);
                if let Some(e) = else_block {
                    self.walk_fn_purity_block(fn_name, e);
                }
            }
            Stmt::For { iter, body, .. } => {
                self.walk_fn_purity_expr(fn_name, iter);
                self.walk_fn_purity_block(fn_name, body);
            }
            Stmt::While { cond, body, .. } => {
                self.walk_fn_purity_expr(fn_name, cond);
                self.walk_fn_purity_block(fn_name, body);
            }
            Stmt::Destructure { value, .. } => self.walk_fn_purity_expr(fn_name, value),
            // A parallel block is inherently effectful (its arms
            // are tool/prompt/agent calls).
            Stmt::Parallel { span, .. } => {
                self.fn_purity_violation(fn_name, "a `parallel:` block", *span);
            }
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Pass { .. } => {}
        }
    }

    fn walk_fn_purity_expr(&mut self, fn_name: &str, expr: &Expr) {
        match expr {
            Expr::Call { callee, args, span } => {
                if let Expr::Ident { name, .. } = callee.as_ref() {
                    match self.bindings.get(&name.span) {
                        Some(Binding::Decl(def_id)) => {
                            let kind = self.symbols.get(*def_id).kind;
                            match kind {
                                DeclKind::Tool => self.fn_purity_violation(
                                    fn_name,
                                    &format!("a call to tool `{}`", name.name),
                                    *span,
                                ),
                                DeclKind::Prompt => self.fn_purity_violation(
                                    fn_name,
                                    &format!("a call to prompt `{}`", name.name),
                                    *span,
                                ),
                                DeclKind::Agent => self.fn_purity_violation(
                                    fn_name,
                                    &format!("a call to agent `{}`", name.name),
                                    *span,
                                ),
                                DeclKind::Fixture => self.fn_purity_violation(
                                    fn_name,
                                    &format!("a call to fixture `{}`", name.name),
                                    *span,
                                ),
                                // Imported callables cross a module
                                // boundary the purity walk cannot see
                                // through; only imported FNs would be
                                // pure, and cross-module fn calls are
                                // not in the v1 surface.
                                DeclKind::ImportedUse => self.fn_purity_violation(
                                    fn_name,
                                    &format!("a call to imported `{}`", name.name),
                                    *span,
                                ),
                                _ => {}
                            }
                        }
                        Some(Binding::BuiltIn(BuiltIn::Ask | BuiltIn::Choose)) => {
                            self.fn_purity_violation(
                                fn_name,
                                &format!("`{}(...)` (human interaction)", name.name),
                                *span,
                            );
                        }
                        _ => {}
                    }
                } else {
                    self.walk_fn_purity_expr(fn_name, callee);
                }
                for a in args {
                    self.walk_fn_purity_expr(fn_name, a);
                }
            }
            Expr::Replay { span, .. } => {
                self.fn_purity_violation(fn_name, "a `replay` block (reads a trace)", *span);
            }
            Expr::FieldAccess { target, .. } => self.walk_fn_purity_expr(fn_name, target),
            Expr::Index { target, index, .. } => {
                self.walk_fn_purity_expr(fn_name, target);
                self.walk_fn_purity_expr(fn_name, index);
            }
            Expr::BinOp { left, right, .. } => {
                self.walk_fn_purity_expr(fn_name, left);
                self.walk_fn_purity_expr(fn_name, right);
            }
            Expr::UnOp { operand, .. } => self.walk_fn_purity_expr(fn_name, operand),
            Expr::List { items, .. } => {
                for i in items {
                    self.walk_fn_purity_expr(fn_name, i);
                }
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.walk_fn_purity_expr(fn_name, k);
                    self.walk_fn_purity_expr(fn_name, v);
                }
            }
            Expr::StructLiteral { fields, spread, .. } => {
                for f in fields {
                    if let Some(v) = &f.value {
                        self.walk_fn_purity_expr(fn_name, v);
                    }
                }
                if let Some(s) = spread {
                    self.walk_fn_purity_expr(fn_name, s);
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.walk_fn_purity_expr(fn_name, scrutinee);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.walk_fn_purity_expr(fn_name, g);
                    }
                    self.walk_fn_purity_expr(fn_name, &arm.body);
                }
            }
            Expr::Lambda { body, .. } => self.walk_fn_purity_expr(fn_name, body),
            Expr::TryPropagate { inner, .. } => self.walk_fn_purity_expr(fn_name, inner),
            Expr::TrustBoundary { inner: body, .. } | Expr::TryRetry { body, .. } => {
                self.walk_fn_purity_expr(fn_name, body)
            }
            Expr::Literal { .. } | Expr::Ident { .. } => {}
        }
    }
}
