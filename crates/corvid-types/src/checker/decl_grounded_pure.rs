//! `@grounded_pure` proof obligation (Provenance Propagation D6).
//!
//! An agent marked `@grounded_pure` MUST NOT launder a
//! `Grounded<T>` value anywhere in its body. Three laundering
//! shapes are caught here, mirroring the IR-visible discards that
//! slice 7 inserts at every silent coercion site:
//!
//! 1. **Implicit coercion** — the typechecker recorded the
//!    expression's span in `Checker::grounded_coercion_sites`
//!    while slot-checking (return / call-arg / struct-field /
//!    list-element / replay-arm / if-condition / etc.). Any
//!    recorded span inside this agent's body is a moat hole.
//! 2. **Explicit unwrap** — the user wrote
//!    `g.unwrap_discarding_sources()`. The IR would lower it to
//!    `UnwrapGrounded`; the AST shape is a method-style call.
//! 3. **Transitive call** — the body calls another agent that is
//!    NOT itself marked `@grounded_pure`. The callee might launder
//!    internally; the composition (R5 attribute-composition matrix)
//!    forbids the call. Tools and prompts are external boundaries
//!    that cannot launder by definition — their declared return
//!    type either is `Grounded<T>` (preserved) or non-grounded
//!    (the typechecker's slot-check at the call site catches the
//!    laundering as case 1).

use super::Checker;
use crate::errors::{TypeError, TypeErrorKind};
use corvid_ast::{AgentAttribute, Block, Expr, Stmt};
use corvid_resolve::{Binding, DeclKind};

impl<'a> Checker<'a> {
    /// Walk `body` enforcing `@grounded_pure`. Emits one
    /// `GroundedPureLaundering` error per offending site. Run
    /// after the main type-check pass so
    /// `self.grounded_coercion_sites` is fully populated.
    pub(super) fn check_grounded_pure_body(&mut self, agent_name: &str, body: &Block) {
        self.walk_grounded_pure_block(agent_name, body);
    }

    fn walk_grounded_pure_block(&mut self, agent: &str, block: &Block) {
        for stmt in &block.stmts {
            self.walk_grounded_pure_stmt(agent, stmt);
        }
    }

    fn walk_grounded_pure_stmt(&mut self, agent: &str, stmt: &Stmt) {
        match stmt {
            Stmt::Let { value, .. } => self.walk_grounded_pure_expr(agent, value),
            Stmt::Return { value, .. } => {
                if let Some(expr) = value {
                    self.walk_grounded_pure_expr(agent, expr);
                }
            }
            Stmt::Yield { value, .. } => self.walk_grounded_pure_expr(agent, value),
            Stmt::If {
                cond,
                then_block,
                else_block,
                ..
            } => {
                self.walk_grounded_pure_expr(agent, cond);
                self.walk_grounded_pure_block(agent, then_block);
                if let Some(eb) = else_block {
                    self.walk_grounded_pure_block(agent, eb);
                }
            }
            Stmt::For { iter, body, .. } => {
                self.walk_grounded_pure_expr(agent, iter);
                self.walk_grounded_pure_block(agent, body);
            }
            Stmt::While { cond, body, .. } => {
                self.walk_grounded_pure_expr(agent, cond);
                self.walk_grounded_pure_block(agent, body);
            }
            Stmt::Destructure { value, .. } => {
                self.walk_grounded_pure_expr(agent, value);
            }
            Stmt::Parallel { arms, .. } => {
                for arm in arms {
                    self.walk_grounded_pure_expr(agent, &arm.call);
                }
            }
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Pass { .. } => {}
            Stmt::Expr { expr, .. } => self.walk_grounded_pure_expr(agent, expr),
            Stmt::Approve { action, .. } => self.walk_grounded_pure_expr(agent, action),
            Stmt::Assign { target, value, .. } => {
                self.walk_grounded_pure_expr(agent, target);
                self.walk_grounded_pure_expr(agent, value);
            }
        }
    }

    fn walk_grounded_pure_expr(&mut self, agent: &str, expr: &Expr) {
        // Case 1: implicit coercion site recorded by slice 7a.
        if self.grounded_coercion_sites.contains(&expr.span()) {
            // Diagnose at the value-expression span. The
            // recorded span is the FROM site of the coercion;
            // the receiving slot's type is captured in the
            // diagnostic message via the typechecker's normal
            // error trail. We label the target by the
            // expression's own type as recorded in
            // `self.types`, which is the most actionable
            // surface to the user.
            let target = self
                .types
                .get(&expr.span())
                .map(|t| t.display_name())
                .unwrap_or_else(|| "value".into());
            self.errors.push(TypeError::with_guarantee(
                TypeErrorKind::GroundedPureLaundering {
                    agent: agent.to_string(),
                    kind: "implicit_coercion".into(),
                    target,
                },
                expr.span(),
                "grounded.no_laundering",
            ));
        }

        // Case 2 + 3: structural inspection.
        match expr {
            Expr::Call { callee, args, span } => {
                self.classify_grounded_pure_call(agent, callee, args, *span);
                // Walk callee + args even after classification —
                // a laundering site nested inside an arg still
                // matters.
                self.walk_grounded_pure_expr(agent, callee);
                for arg in args {
                    self.walk_grounded_pure_expr(agent, arg);
                }
            }
            Expr::FieldAccess { target, .. } | Expr::TryPropagate { inner: target, .. } => {
                self.walk_grounded_pure_expr(agent, target);
            }
            Expr::Lambda { body, .. } => {
                self.walk_grounded_pure_expr(agent, body);
            }
            Expr::StructLiteral { fields, spread, .. } => {
                for f in fields {
                    if let Some(v) = &f.value {
                        self.walk_grounded_pure_expr(agent, v);
                    }
                }
                if let Some(s) = spread {
                    self.walk_grounded_pure_expr(agent, s);
                }
            }
            Expr::Index { target, index, .. } => {
                self.walk_grounded_pure_expr(agent, target);
                self.walk_grounded_pure_expr(agent, index);
            }
            Expr::BinOp { left, right, .. } => {
                self.walk_grounded_pure_expr(agent, left);
                self.walk_grounded_pure_expr(agent, right);
            }
            Expr::UnOp { operand, .. } => self.walk_grounded_pure_expr(agent, operand),
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.walk_grounded_pure_expr(agent, scrutinee);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.walk_grounded_pure_expr(agent, g);
                    }
                    self.walk_grounded_pure_expr(agent, &arm.body);
                }
            }
            Expr::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.walk_grounded_pure_expr(agent, k);
                    self.walk_grounded_pure_expr(agent, v);
                }
            }
            Expr::List { items, .. } => {
                for item in items {
                    self.walk_grounded_pure_expr(agent, item);
                }
            }
            Expr::TrustBoundary { inner: body, .. } | Expr::TryRetry { body, .. } => {
                self.walk_grounded_pure_expr(agent, body)
            }
            Expr::Replay {
                trace,
                arms,
                else_body,
                ..
            } => {
                self.walk_grounded_pure_expr(agent, trace);
                for arm in arms {
                    self.walk_grounded_pure_expr(agent, &arm.body);
                }
                self.walk_grounded_pure_expr(agent, else_body);
            }
            Expr::Literal { .. } | Expr::Ident { .. } => {}
        }
    }

    /// Classify a call inside a `@grounded_pure` body. Catches the
    /// explicit `.unwrap_discarding_sources()` method form (case 2)
    /// and the call-to-non-grounded-pure-agent form (case 3). Free
    /// function / tool / prompt calls are not flagged here — if
    /// they launder via the type system, case 1 already fired on
    /// the slot-check at the call-arg or return site.
    fn classify_grounded_pure_call(
        &mut self,
        agent: &str,
        callee: &Expr,
        args: &[Expr],
        span: corvid_ast::Span,
    ) {
        // Case 2: explicit `<expr>.unwrap_discarding_sources()`.
        if args.is_empty() {
            if let Expr::FieldAccess { target, field, .. } = callee {
                if field.name == "unwrap_discarding_sources" {
                    let target_label = expr_label(target);
                    self.errors.push(TypeError::with_guarantee(
                        TypeErrorKind::GroundedPureLaundering {
                            agent: agent.to_string(),
                            kind: "explicit_unwrap".into(),
                            target: target_label,
                        },
                        span,
                        "grounded.no_laundering",
                    ));
                    return;
                }
            }
        }

        // Case 3: bare ident call. Resolved decl lookup tells us
        // whether the target is an agent + whether it's
        // `@grounded_pure`. Method-form `x.foo()` is out of scope
        // for v1 — the catalog + ident-call coverage is enough on
        // realistic programs, mirroring the
        // `@deterministic` policy.
        let Expr::Ident { name, span: id_span } = callee else {
            return;
        };
        let binding = self.bindings.get(id_span).cloned();
        let Some(Binding::Decl(def_id)) = binding else {
            return;
        };
        let entry = self.symbols.get(def_id);
        if !matches!(entry.kind, DeclKind::Agent) {
            // Tools / prompts don't have bodies that could launder
            // — they're external boundaries. The typechecker's
            // slot-check at the call-arg / return site catches any
            // laundering through them as case 1.
            return;
        }
        let Some(callee_agent) = self.agents_by_id.get(&def_id).copied() else {
            return;
        };
        if !AgentAttribute::is_grounded_pure(&callee_agent.attributes) {
            self.errors.push(TypeError::with_guarantee(
                TypeErrorKind::GroundedPureLaundering {
                    agent: agent.to_string(),
                    kind: "non_grounded_pure_call".into(),
                    target: name.name.clone(),
                },
                span,
                "grounded.no_laundering",
            ));
        }
    }
}

/// Best-effort label for a `<target>.unwrap_discarding_sources()`
/// receiver expression. Bare idents become their name; everything
/// else falls back to `"value"`. Used only for diagnostics.
fn expr_label(expr: &Expr) -> String {
    match expr {
        Expr::Ident { name, .. } => name.name.clone(),
        _ => "value".into(),
    }
}
