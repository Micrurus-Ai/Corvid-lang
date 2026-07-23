//! Call-shape checks: tool, prompt, agent, struct constructor,
//! built-in constructor (`Weak::new`, `Weak::upgrade`, `Stream::…`,
//! `List::…`), and method-call dispatch.
//!
//! Also hosts `check_args_against_params`, the shared arity +
//! type-compatibility validator used by every typed-callable
//! check.
//!
//! Extracted from `checker.rs` as part of Phase 20i responsibility
//! decomposition.

use super::{is_weakable_type, pascal_case, snake_case, Checker, EffectFrontier};
use crate::errors::{TypeError, TypeErrorKind};
use crate::types::Type;
use corvid_ast::{Effect, Expr, Ident, Param, Span, WeakEffect, WeakEffectRow};
use corvid_resolve::{resolver::MethodKind, Binding, BuiltIn, DeclKind, DefId, LocalId};
use std::collections::HashMap;

impl<'a> Checker<'a> {
    pub(super) fn check_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        // A callee of shape `target.field` is a method call.
        // Lower it: typecheck the receiver, look up the method by
        // (receiver_type_def_id, method_name), validate args (with
        // the receiver implicitly prepended), reuse the appropriate
        // tool / prompt / agent dispatch path.
        if let Expr::FieldAccess {
            target,
            field,
            span: callee_span,
        } = callee
        {
            if let Some(ty) = self.check_imported_call(target, field, args, *callee_span, span) {
                return ty;
            }
            return self.check_method_call(target, field, args, span);
        }

        // Identify what's being called by looking at the callee's binding.
        let Expr::Ident { name, .. } = callee else {
            // Indirect or chained callee — typecheck args and give up.
            for a in args {
                let _ = self.check_expr(a);
            }
            return Type::Unknown;
        };

        let Some(binding) = self.bindings.get(&name.span) else {
            // Unresolved callee (e.g. approve label encountered outside an
            // approve — shouldn't happen for well-formed code). Typecheck args.
            for a in args {
                let _ = self.check_expr(a);
            }
            return Type::Unknown;
        };

        match binding {
            Binding::Decl(def_id) => {
                let def_id = *def_id;
                let entry = self.symbols.get(def_id);
                match entry.kind {
                    DeclKind::Tool => self.check_tool_call(def_id, &name.name, args, span),
                    DeclKind::Prompt => self.check_prompt_call(def_id, &name.name, args),
                    DeclKind::Agent => self.check_agent_call(def_id, &name.name, args),
                    DeclKind::Fn => self.check_fn_call(def_id, &name.name, args),
                    DeclKind::Fixture => self.check_fixture_call(def_id, &name.name, args, span),
                    DeclKind::ImportedUse => {
                        self.check_imported_use_call(def_id, &name.name, args, name.span, span)
                    }
                    DeclKind::Import
                    | DeclKind::Store
                    | DeclKind::Eval
                    | DeclKind::Test
                    | DeclKind::Mock
                    | DeclKind::Effect
                    | DeclKind::Model
                    | DeclKind::Server
                    | DeclKind::Identity => {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        Type::Unknown
                    }
                    DeclKind::Type => self.check_struct_constructor(def_id, &name.name, args),
                    DeclKind::Variant => self.check_variant_constructor(def_id, &name.name, args),
                }
            }
            Binding::BuiltIn(builtin) => {
                self.check_builtin_constructor_call(*builtin, name, args, expected)
            }
            Binding::Local(lid) => {
                // First-class function values (45j): a local of
                // function type is callable, with args checked
                // against its parameter types. Unknown-typed locals
                // stay lenient; anything else is not callable.
                let lid = *lid;
                let callee_ty = self.local_types.get(&lid).cloned().unwrap_or(Type::Unknown);
                match callee_ty {
                    Type::Function { params, ret, .. } => {
                        if args.len() != params.len() {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::ArityMismatch {
                                    callee: name.name.clone(),
                                    expected: params.len(),
                                    got: args.len(),
                                },
                                span,
                            ));
                            for a in args {
                                let _ = self.check_expr(a);
                            }
                            return (*ret).clone();
                        }
                        for (arg, pty) in args.iter().zip(params.iter()) {
                            let got = self.check_expr_as(arg, Some(pty));
                            if !got.is_assignable_to(pty) {
                                self.errors.push(TypeError::new(
                                    TypeErrorKind::TypeMismatch {
                                        expected: pty.display_name(),
                                        got: got.display_name(),
                                        context: format!("argument to `{}`", name.name),
                                    },
                                    arg.span(),
                                ));
                            }
                        }
                        (*ret).clone()
                    }
                    Type::Unknown => {
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        Type::Unknown
                    }
                    other => {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::NotCallable {
                                got: other.display_name(),
                            },
                            callee.span(),
                        ));
                        for a in args {
                            let _ = self.check_expr(a);
                        }
                        Type::Unknown
                    }
                }
            }
        }
    }

    fn check_tool_call(
        &mut self,
        def_id: DefId,
        tool_name: &str,
        args: &[Expr],
        span: Span,
    ) -> Type {
        // A call resolves to either a `tool` or a connector
        // `operation` — both carry the same signature shape (params /
        // effect / effect row / return), so a call to either types
        // identically. We look the signature up from whichever table
        // holds this DefId. Both `.get(..)` results are `&'a`
        // references that do NOT borrow `self`, so subsequent
        // `self`-mutating calls remain legal.
        let (params, effect, effect_row, return_ty): (
            &[Param],
            Effect,
            &corvid_ast::EffectRow,
            &corvid_ast::TypeRef,
        ) = if let Some(&tool) = self.tools_by_id.get(&def_id) {
            (&tool.params, tool.effect, &tool.effect_row, &tool.return_ty)
        } else if let Some(&op) = self.operations_by_id.get(&def_id) {
            (&op.params, op.effect, &op.effect_row, &op.return_ty)
        } else {
            panic!("tool/operation DefId not indexed");
        };

        let arg_types = self.check_args_collecting_types(tool_name, params, args, false);

        // Effect check: a `dangerous` tool must have a prior matching
        // approve — and so must a tool whose composed effect row
        // carries `trust: supervisor_required` or `trust:
        // human_required`. The approve requirement is DERIVED from
        // the trust tier so an author who declares high-trust
        // semantics but forgets the `dangerous` marker still gets
        // compile-time protection.
        let derived_trust = crate::effects::effect_row_trust_requires_approval(
            effect_row,
            self.registry,
        );
        if matches!(effect, Effect::Dangerous) || derived_trust.is_some() {
            let authorized = self
                .approvals
                .iter()
                .any(|a| snake_case(&a.label) == tool_name && a.arity == args.len());
            if !authorized {
                // Discriminate between two registry rows:
                //   - `approval.token_lexical_only`: an approve with the
                //     right label+arity exists somewhere in this
                //     agent's body but is out of lexical scope at this
                //     call site (e.g. inside a sibling `if` branch).
                //   - `approval.dangerous_call_requires_token` /
                //     `approval.trust_tier_requires_token`: no
                //     matching approve exists anywhere in this agent.
                // The two distinct guarantee_ids let users know whether
                // their fix is "move the approve to the right scope"
                // or "add an approve at all" — the launch claim
                // promises both properties separately, so the
                // diagnostic must too.
                let approve_exists_out_of_scope = self
                    .approvals_seen_in_agent
                    .iter()
                    .any(|a| snake_case(&a.label) == tool_name && a.arity == args.len());
                let is_dangerous = matches!(effect, Effect::Dangerous);
                let guarantee_id = if approve_exists_out_of_scope {
                    "approval.token_lexical_only"
                } else if is_dangerous {
                    "approval.dangerous_call_requires_token"
                } else {
                    "approval.trust_tier_requires_token"
                };
                // `dangerous` keeps its established diagnostic; the
                // derived requirement gets its own error naming the
                // deriving effect + tier so the obligation is
                // traceable to the declaration that created it.
                let kind = if is_dangerous {
                    TypeErrorKind::UnapprovedDangerousCall {
                        tool: tool_name.to_string(),
                        expected_approve_label: pascal_case(tool_name),
                        arity: args.len(),
                    }
                } else {
                    let (deriving_effect, trust_tier) =
                        derived_trust.clone().expect("derived_trust is Some here");
                    TypeErrorKind::UnapprovedHighTrustCall {
                        tool: tool_name.to_string(),
                        expected_approve_label: pascal_case(tool_name),
                        arity: args.len(),
                        deriving_effect,
                        trust_tier,
                    }
                };
                self.errors.push(TypeError::with_guarantee(kind, span, guarantee_id));
            }
        }

        // Slice 50i — the SINK RULE: an approval-requiring call
        // (dangerous marker, or trust-derived) refuses tainted
        // arguments. This is the line that makes prompt injection a
        // compile error: attacker-influenced content cannot
        // parameterize a consequential action without an explicit
        // `trusted(...)` boundary.
        let requires_approval = matches!(effect, Effect::Dangerous)
            || crate::effects::effect_row_trust_requires_approval(
                effect_row,
                self.registry,
            )
            .is_some();
        if requires_approval {
            for (i, arg_ty) in arg_types.iter().enumerate() {
                if matches!(arg_ty, Type::Tainted(_)) {
                    self.errors.push(TypeError::with_guarantee(
                        TypeErrorKind::TaintedDangerousArgument {
                            tool: tool_name.to_string(),
                            argument_index: i + 1,
                            arg_type: arg_ty.display_name(),
                        },
                        args.get(i).map(|a| a.span()).unwrap_or(span),
                        "taint.untrusted_cannot_reach_dangerous",
                    ));
                }
            }
        }

        self.bump_effect(WeakEffect::ToolCall);
        let ret = self.type_ref_to_type(return_ty);
        let ret = self.ground_if_effect_grounded(effect_row, ret);
        self.taint_if_effect_untrusted(effect_row, ret)
    }

    /// Provenance Propagation Design X (D1 part A): if a callee's
    /// effect row carries `data: grounded`, a call to it produces a
    /// `Grounded<T>` value at runtime — so the call expression's type
    /// is `Grounded<T>`. The type system thus reflects what the
    /// runtime actually produces, which is what lets the contagion
    /// law (D1 part B — `check_binop` / `check_unop`) and the
    /// provenance-reachability analysis observe effect-induced
    /// grounding instead of being blind to it.
    ///
    /// An already-grounded return type (an explicit `Grounded<T>`
    /// annotation on the callee) is not double-wrapped.
    /// Slice 50i — the taint mirror: a callee whose effect row
    /// carries `data: untrusted` returns `Tainted<T>`. Applied
    /// after grounding so an (unusual) untrusted+grounded source is
    /// `Tainted<Grounded<T>>` — the taint stays outermost, where the
    /// sink check sees it.
    fn taint_if_effect_untrusted(&self, effect_row: &corvid_ast::EffectRow, ret: Type) -> Type {
        if matches!(ret, Type::Tainted(_)) {
            return ret;
        }
        if crate::effects::effect_row_is_untrusted(effect_row, self.registry) {
            Type::Tainted(Box::new(ret))
        } else {
            ret
        }
    }

    fn ground_if_effect_grounded(&self, effect_row: &corvid_ast::EffectRow, ret: Type) -> Type {
        if matches!(ret, Type::Grounded(_)) {
            return ret;
        }
        if crate::effects::effect_row_is_grounded(effect_row, self.registry) {
            Type::Grounded(Box::new(ret))
        } else {
            ret
        }
    }

    fn check_fixture_call(
        &mut self,
        def_id: DefId,
        fixture_name: &str,
        args: &[Expr],
        span: Span,
    ) -> Type {
        let fixture = *self
            .fixtures_by_id
            .get(&def_id)
            .expect("fixture DefId not indexed");
        self.check_args_against_params(fixture_name, &fixture.params, args);
        if !self.in_test_body {
            self.errors.push(TypeError::new(
                TypeErrorKind::NotCallable {
                    got: format!("test fixture `{fixture_name}` outside a test or mock"),
                },
                span,
            ));
        }
        self.type_ref_to_type(&fixture.return_ty)
    }

    fn check_prompt_call(&mut self, def_id: DefId, name: &str, args: &[Expr]) -> Type {
        let prompt = *self
            .prompts_by_id
            .get(&def_id)
            .expect("prompt DefId not indexed");
        let arg_types = self.check_args_collecting_types(name, &prompt.params, args, true);
        self.bump_effect(WeakEffect::Llm);
        let ret = self.type_ref_to_type(&prompt.return_ty);
        let ret = self.ground_if_effect_grounded(&prompt.effect_row, ret);
        let ret = self.taint_if_effect_untrusted(&prompt.effect_row, ret);
        // Slice 50i — PROMPT CONTAGION, the rule that models the
        // actual attack: an LLM that read attacker-controlled text
        // produces attacker-influenced output. A prompt consuming a
        // tainted argument returns Tainted<output>.
        if !matches!(ret, Type::Tainted(_))
            && arg_types.iter().any(|t| matches!(t, Type::Tainted(_)))
        {
            Type::Tainted(Box::new(ret))
        } else {
            ret
        }
    }

    fn check_agent_call(&mut self, def_id: DefId, name: &str, args: &[Expr]) -> Type {
        let agent = *self
            .agents_by_id
            .get(&def_id)
            .expect("agent DefId not indexed");
        self.check_args_against_params(name, &agent.params, args);
        self.bump_effect(WeakEffect::ToolCall);
        self.bump_effect(WeakEffect::Llm);
        self.bump_effect(WeakEffect::Approve);
        let ret = self.type_ref_to_type(&agent.return_ty);
        let ret = self.ground_if_effect_grounded(&agent.effect_row, ret);
        self.taint_if_effect_untrusted(&agent.effect_row, ret)
    }

    /// `fn` call (slice 45r): args against params, return type
    /// out. NO effect bumps — a pure function is exactly the call
    /// that leaves the weak-effect row untouched.
    fn check_fn_call(&mut self, def_id: DefId, name: &str, args: &[Expr]) -> Type {
        let f = *self.fns_by_id.get(&def_id).expect("fn DefId not indexed");
        self.check_args_against_params(name, &f.params, args);
        self.type_ref_to_type(&f.return_ty)
    }

    fn check_builtin_constructor_call(
        &mut self,
        builtin: BuiltIn,
        name: &Ident,
        args: &[Expr],
        expected: Option<&Type>,
    ) -> Type {
        match builtin {
            BuiltIn::Ok => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::Result(Box::new(Type::Unknown), Box::new(Type::Unknown));
                }
                let ok_ty = self.check_expr(&args[0]);
                let err_ty = match &self.current_return {
                    Some(Type::Result(_, err)) => (**err).clone(),
                    _ => Type::Unknown,
                };
                Type::Result(Box::new(ok_ty), Box::new(err_ty))
            }
            BuiltIn::Err => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::Result(Box::new(Type::Unknown), Box::new(Type::Unknown));
                }
                let err_ty = self.check_expr(&args[0]);
                let ok_ty = match &self.current_return {
                    Some(Type::Result(ok, _)) => (**ok).clone(),
                    _ => Type::Unknown,
                };
                Type::Result(Box::new(ok_ty), Box::new(err_ty))
            }
            BuiltIn::Some => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::Option(Box::new(Type::Unknown));
                }
                let expected_inner = match expected {
                    Some(Type::Option(inner)) => Some(&**inner),
                    _ => None,
                };
                let inner_ty = self.check_expr_as(&args[0], expected_inner);
                let final_inner_ty = match expected_inner {
                    Some(exp) if inner_ty.is_assignable_to(exp) => {
                        // D5: `Some(g)` where `g: Grounded<T>` and the
                        // expected slot is `Option<T>` silently strips.
                        self.record_if_grounded_coercion(&inner_ty, exp, args[0].span());
                        exp.clone()
                    }
                    _ => inner_ty,
                };
                Type::Option(Box::new(final_inner_ty))
            }
            BuiltIn::Range => {
                if args.len() != 2 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 2,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::List(Box::new(Type::Int));
                }
                for arg in args {
                    let t = self.check_expr_as(arg, Some(&Type::Int));
                    if !t.is_assignable_to(&Type::Int) {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::TypeMismatch {
                                expected: "Int".into(),
                                got: t.display_name(),
                                context: "argument to `range`".into(),
                            },
                            arg.span(),
                        ));
                    }
                }
                Type::List(Box::new(Type::Int))
            }
            BuiltIn::WeakNew => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::Weak(Box::new(Type::Unknown), WeakEffectRow::any());
                }
                let target_ty = self.check_expr(&args[0]);
                if !is_weakable_type(&target_ty) && !matches!(target_ty, Type::Unknown) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::InvalidWeakNewTarget {
                            got: target_ty.display_name(),
                        },
                        args[0].span(),
                    ));
                }
                let row = match expected {
                    Some(Type::Weak(_, row)) => *row,
                    _ => WeakEffectRow::any(),
                };
                Type::Weak(Box::new(target_ty), row)
            }
            BuiltIn::WeakUpgrade => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::Option(Box::new(Type::Unknown));
                }
                let weak_ty = self.check_expr(&args[0]);
                let refreshed_at = self.refresh_frontier_for_expr(&args[0], &weak_ty);
                match weak_ty {
                    Type::Weak(inner, row) => {
                        let invalidating = self
                            .effect_frontier
                            .invalidating_effects_since(&refreshed_at, row);
                        if !invalidating.is_empty() {
                            self.errors.push(TypeError::new(
                                TypeErrorKind::WeakUpgradeAcrossEffects {
                                    effects: invalidating,
                                },
                                args[0].span(),
                            ));
                        } else {
                            self.refresh_after_upgrade(&args[0]);
                        }
                        Type::Option(inner)
                    }
                    Type::Unknown => Type::Option(Box::new(Type::Unknown)),
                    other => {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::InvalidWeakUpgradeTarget {
                                got: other.display_name(),
                            },
                            args[0].span(),
                        ));
                        Type::Option(Box::new(Type::Unknown))
                    }
                }
            }
            BuiltIn::StreamResumeToken => {
                if args.len() != 1 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: name.name.clone(),
                            expected: 1,
                            got: args.len(),
                        },
                        name.span,
                    ));
                    for arg in args {
                        let _ = self.check_expr(arg);
                    }
                    return Type::ResumeToken(Box::new(Type::Unknown));
                }
                match self.check_expr(&args[0]) {
                    Type::Stream(inner) => Type::ResumeToken(inner),
                    Type::Unknown => Type::ResumeToken(Box::new(Type::Unknown)),
                    other => {
                        self.errors.push(TypeError::new(
                            TypeErrorKind::TypeMismatch {
                                expected: "Stream<T>".into(),
                                got: other.display_name(),
                                context: "resume_token argument".into(),
                            },
                            args[0].span(),
                        ));
                        Type::ResumeToken(Box::new(Type::Unknown))
                    }
                }
            }
            BuiltIn::StreamMerge => self.check_stream_merge_call(name, args),
            BuiltIn::Resume => self.check_resume_call(name, args),
            BuiltIn::Ask => self.check_ask_call(name, args),
            BuiltIn::Choose => self.check_choose_call(name, args),
            BuiltIn::None => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::NotCallable {
                        got: "Option".into(),
                    },
                    name.span,
                ));
                for arg in args {
                    let _ = self.check_expr(arg);
                }
                Type::Unknown
            }
            BuiltIn::Page => self.check_page_call(name, args),
            _ => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::NotCallable {
                        got: name.name.clone(),
                    },
                    name.span,
                ));
                for arg in args {
                    let _ = self.check_expr(arg);
                }
                Type::Unknown
            }
        }
    }

    /// `Page(items, next_cursor)` (slice 52c-2) — constructs a
    /// cursor-paginated response envelope. `items: List<Item>` and
    /// `next_cursor: Option<String>`; `has_more` is derived at
    /// construction from `next_cursor`'s presence. Returns
    /// `Page<Item>`, mirroring the `Ok`/`Some` builtin constructors.
    fn check_page_call(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if args.len() != 2 {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.name.clone(),
                    expected: 2,
                    got: args.len(),
                },
                name.span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Type::Page(Box::new(Type::Unknown));
        }
        let items_ty = self.check_expr(&args[0]);
        let item_ty = match &items_ty {
            Type::List(inner) => (**inner).clone(),
            Type::Unknown => Type::Unknown,
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "List<Item>".into(),
                        got: other.display_name(),
                        context: "Page items".into(),
                    },
                    args[0].span(),
                ));
                Type::Unknown
            }
        };
        let cursor_ty = self.check_expr(&args[1]);
        let cursor_ok = matches!(&cursor_ty, Type::Option(inner) if matches!(**inner, Type::String))
            || matches!(cursor_ty, Type::Unknown);
        if !cursor_ok {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: "Option<String>".into(),
                    got: cursor_ty.display_name(),
                    context: "Page next_cursor".into(),
                },
                args[1].span(),
            ));
        }
        Type::Page(Box::new(item_ty))
    }

    fn check_ask_call(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if args.len() != 2 {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.name.clone(),
                    expected: 2,
                    got: args.len(),
                },
                name.span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Type::Unknown;
        }

        let prompt_ty = self.check_expr(&args[0]);
        if !matches!(prompt_ty, Type::String | Type::Unknown) {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: "String".into(),
                    got: prompt_ty.display_name(),
                    context: "ask prompt".into(),
                },
                args[0].span(),
            ));
        }

        self.bump_effect(WeakEffect::Human);
        self.type_expr_to_type(&args[1])
    }

    fn check_choose_call(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if args.len() != 1 {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.name.clone(),
                    expected: 1,
                    got: args.len(),
                },
                name.span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Type::Unknown;
        }

        self.bump_effect(WeakEffect::Human);
        match self.check_expr(&args[0]) {
            Type::List(inner) => *inner,
            Type::Unknown => Type::Unknown,
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "List<T>".into(),
                        got: other.display_name(),
                        context: "choose options".into(),
                    },
                    args[0].span(),
                ));
                Type::Unknown
            }
        }
    }

    fn type_expr_to_type(&mut self, expr: &Expr) -> Type {
        let Expr::Ident { name, .. } = expr else {
            self.errors.push(TypeError::new(
                TypeErrorKind::TypeMismatch {
                    expected: "type name".into(),
                    got: "value expression".into(),
                    context: "ask return type".into(),
                },
                expr.span(),
            ));
            return Type::Unknown;
        };

        match self.bindings.get(&name.span) {
            Some(Binding::BuiltIn(BuiltIn::Int)) => Type::Int,
            Some(Binding::BuiltIn(BuiltIn::Float)) => Type::Float,
            Some(Binding::BuiltIn(BuiltIn::String)) => Type::String,
            Some(Binding::BuiltIn(BuiltIn::Bool)) => Type::Bool,
            Some(Binding::BuiltIn(BuiltIn::Nothing)) => Type::Nothing,
            Some(Binding::Decl(def_id)) if self.symbols.get(*def_id).kind == DeclKind::Type => {
                Type::Struct(*def_id)
            }
            _ => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "type name".into(),
                        got: name.name.clone(),
                        context: "ask return type".into(),
                    },
                    name.span,
                ));
                Type::Unknown
            }
        }
    }

    fn check_resume_call(&mut self, name: &Ident, args: &[Expr]) -> Type {
        if args.len() != 2 {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.name.clone(),
                    expected: 2,
                    got: args.len(),
                },
                name.span,
            ));
            for arg in args {
                let _ = self.check_expr(arg);
            }
            return Type::Stream(Box::new(Type::Unknown));
        }

        let prompt_ty = match &args[0] {
            Expr::Ident {
                name: prompt_name, ..
            } => match self.bindings.get(&prompt_name.span) {
                Some(Binding::Decl(def_id))
                    if self.symbols.get(*def_id).kind == DeclKind::Prompt =>
                {
                    let prompt = *self
                        .prompts_by_id
                        .get(def_id)
                        .expect("prompt DefId not indexed");
                    self.type_ref_to_type(&prompt.return_ty)
                }
                _ => {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: "prompt declaration".into(),
                            got: "non-prompt value".into(),
                            context: "first resume argument".into(),
                        },
                        args[0].span(),
                    ));
                    Type::Stream(Box::new(Type::Unknown))
                }
            },
            _ => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "prompt declaration".into(),
                        got: "expression".into(),
                        context: "first resume argument".into(),
                    },
                    args[0].span(),
                ));
                Type::Stream(Box::new(Type::Unknown))
            }
        };

        let token_ty = self.check_expr(&args[1]);
        match (&prompt_ty, token_ty) {
            (Type::Stream(prompt_inner), Type::ResumeToken(token_inner)) => {
                if !token_inner.is_assignable_to(prompt_inner) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: format!("ResumeToken<{}>", prompt_inner.display_name()),
                            got: format!("ResumeToken<{}>", token_inner.display_name()),
                            context: "resume token".into(),
                        },
                        args[1].span(),
                    ));
                }
                Type::Stream(prompt_inner.clone())
            }
            (Type::Stream(prompt_inner), Type::Unknown) => Type::Stream(prompt_inner.clone()),
            (Type::Stream(_), other) => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "ResumeToken<T>".into(),
                        got: other.display_name(),
                        context: "resume token".into(),
                    },
                    args[1].span(),
                ));
                prompt_ty
            }
            (other, _) => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::TypeMismatch {
                        expected: "streaming prompt".into(),
                        got: other.display_name(),
                        context: "first resume argument".into(),
                    },
                    args[0].span(),
                ));
                Type::Stream(Box::new(Type::Unknown))
            }
        }
    }

    /// `target.method(args)` rewritten to a regular
    /// function call with the receiver as the first argument. The
    /// receiver's type is looked up in the methods side-table to
    /// pick the matching method DefId; from there we reuse the
    /// existing tool / prompt / agent dispatch.
    ///
    /// Errors:
    ///   - receiver isn't a struct (no methods on built-ins yet).
    ///   - method name doesn't exist on the type.
    ///   - arity mismatch (argv vs declared params, accounting for
    ///     receiver-as-first-param).
    pub(super) fn check_method_call(
        &mut self,
        target: &Expr,
        method_name: &Ident,
        args: &[Expr],
        span: Span,
    ) -> Type {
        // 1. Typecheck the receiver and require a struct type.
        let recv_ty = self.check_expr(target);
        if method_name.name == "split_by" {
            return self.check_stream_split_by_method(target, &recv_ty, method_name, args);
        }
        if method_name.name == "ordered_by" {
            return self.check_stream_ordered_by_method(target, recv_ty, method_name, args);
        }
        if let Type::Grounded(inner) = &recv_ty {
            if method_name.name == "unwrap_discarding_sources" {
                if args.len() != 0 {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::ArityMismatch {
                            callee: method_name.name.clone(),
                            expected: 0,
                            got: args.len(),
                        },
                        span,
                    ));
                    for a in args {
                        let _ = self.check_expr(a);
                    }
                }
                return (**inner).clone();
            }
        }
        // Builtin-method table (slice 45c): methods on built-in
        // receiver types resolve here — one shared table drives the
        // checker, the lowerer, and the interpreter.
        if let Some(sig) = crate::builtin_methods::builtin_method(&recv_ty, &method_name.name) {
            if args.len() != sig.params.len() {
                self.errors.push(TypeError::new(
                    TypeErrorKind::ArityMismatch {
                        callee: method_name.name.clone(),
                        expected: sig.params.len(),
                        got: args.len(),
                    },
                    span,
                ));
                for a in args {
                    let _ = self.check_expr(a);
                }
                return sig.ret;
            }
            // Sequential signature refinement (45j): `fold`'s
            // accumulator type comes from its checked `init`
            // argument, and `map`'s result element type from the
            // lambda argument's checked return type — neither can
            // come from the receiver alone, so the signature is
            // refined as arguments are checked, in order.
            use crate::builtin_methods::BuiltinMethodKind;
            let mut sig = sig;
            let mut first_arg_ty = Type::Unknown;
            for i in 0..args.len() {
                let param_ty = sig.params[i].clone();
                let arg = &args[i];
                let got = self.check_expr_as(arg, Some(&param_ty));
                if i == 0 {
                    first_arg_ty = got.clone();
                }
                if sig.kind == BuiltinMethodKind::ListFold && i == 0 {
                    if let Type::Function { params, ret, .. } = &mut sig.params[1] {
                        if let Some(acc) = params.get_mut(0) {
                            *acc = got.clone();
                        }
                        **ret = got.clone();
                    }
                    sig.ret = got.clone();
                }
                if !got.is_assignable_to(&param_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: param_ty.display_name(),
                            got: got.display_name(),
                            context: format!("argument to `{}`", method_name.name),
                        },
                        arg.span(),
                    ));
                }
            }
            if sig.kind == BuiltinMethodKind::ListMap {
                if let Type::Function { ret, .. } = &first_arg_ty {
                    sig.ret = Type::List(ret.clone());
                }
            }
            // Option/Result refinement (45l): `ok_or`'s error type
            // is its argument's; `map_err`'s is the lambda's checked
            // return type.
            if sig.kind == BuiltinMethodKind::OptionOkOr {
                if let Type::Result(ok, _) = &sig.ret {
                    sig.ret = Type::Result(ok.clone(), Box::new(first_arg_ty.clone()));
                }
            }
            if sig.kind == BuiltinMethodKind::ResultMapErr {
                if let (Type::Result(ok, _), Type::Function { ret, .. }) =
                    (&sig.ret, &first_arg_ty)
                {
                    sig.ret = Type::Result(ok.clone(), ret.clone());
                }
            }
            return sig.ret;
        }

        let recv_def_id = match recv_ty {
            Type::Struct(id) => id,
            other => {
                self.errors.push(TypeError::new(
                    TypeErrorKind::NotCallable {
                        got: format!(
                            "method `{}` on receiver of type `{}` — no builtin method with this name (String.length() shipped in 45c; method batches land in 45d/45e/45f/45l), and user methods work on user-declared struct types via `extend`",
                            method_name.name,
                            other.display_name()
                        ),
                    },
                    target.span(),
                ));
                // Still typecheck remaining args for diagnostics.
                for a in args {
                    let _ = self.check_expr(a);
                }
                return Type::Unknown;
            }
        };

        // 2. Look up the method.
        let method = match self
            .methods
            .get(&recv_def_id)
            .and_then(|m| m.get(&method_name.name))
        {
            Some(m) => m.clone(),
            None => {
                let type_name = self.symbols.get(recv_def_id).name.clone();
                self.errors.push(TypeError::new(
                    TypeErrorKind::NotCallable {
                        got: format!("no method `{}` on type `{type_name}`", method_name.name),
                    },
                    method_name.span,
                ));
                for a in args {
                    let _ = self.check_expr(a);
                }
                return Type::Unknown;
            }
        };

        // 3. Build the effective argument list: receiver prepended.
        //    Then dispatch by method kind, reusing the existing
        //    free-call paths.
        let mut effective_args: Vec<Expr> = Vec::with_capacity(args.len() + 1);
        effective_args.push(target.clone());
        effective_args.extend_from_slice(args);

        match method.kind {
            MethodKind::Tool => {
                self.check_tool_call(method.def_id, &method_name.name, &effective_args, span)
            }
            MethodKind::Prompt => {
                self.check_prompt_call(method.def_id, &method_name.name, &effective_args)
            }
            MethodKind::Agent => {
                self.check_agent_call(method.def_id, &method_name.name, &effective_args)
            }
        }
    }

    /// `TypeName(field0, field1, ...)` — construct a struct. Field
    /// values must be assignable to each field's declared type.
    /// Returns `Struct(def_id)`.
    /// Sum-variant construction (slice 45h): `Approved("alice")`.
    /// Fields check positionally against the variant declaration;
    /// the value's type is the OWNING sum type.
    pub(super) fn check_variant_constructor(
        &mut self,
        variant_id: DefId,
        name: &str,
        args: &[Expr],
    ) -> Type {
        let Some((owner_id, idx)) = self.variant_owners.get(&variant_id).copied() else {
            for a in args {
                let _ = self.check_expr(a);
            }
            return Type::Unknown;
        };
        let Some(ty_decl) = self.types_by_id.get(&owner_id).copied() else {
            for a in args {
                let _ = self.check_expr(a);
            }
            return Type::Struct(owner_id);
        };
        let Some(variant) = ty_decl.variants.get(idx as usize) else {
            return Type::Struct(owner_id);
        };
        if args.len() != variant.fields.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.to_string(),
                    expected: variant.fields.len(),
                    got: args.len(),
                },
                args.first().map(|a| a.span()).unwrap_or(variant.span),
            ));
        }
        for (i, arg) in args.iter().enumerate() {
            if let Some(field) = variant.fields.get(i) {
                let field_ty = self.type_ref_to_type(&field.ty);
                let arg_ty = self.check_expr_as(arg, Some(&field_ty));
                if !arg_ty.is_assignable_to(&field_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: field_ty.display_name(),
                            got: arg_ty.display_name(),
                            context: format!("field `{}` of `{name}`", field.name.name),
                        },
                        arg.span(),
                    ));
                }
            } else {
                let _ = self.check_expr(arg);
            }
        }
        Type::Struct(owner_id)
    }

    pub(super) fn check_struct_constructor(
        &mut self,
        def_id: DefId,
        name: &str,
        args: &[Expr],
    ) -> Type {
        let ty_decl = *self
            .types_by_id
            .get(&def_id)
            .expect("type DefId not indexed");

        // A type alias (45n) is transparent — it has no constructor
        // of its own. Construct the target type instead.
        if ty_decl.alias.is_some() {
            self.errors.push(TypeError::new(
                TypeErrorKind::StructLiteralInvalid {
                    type_name: name.to_string(),
                    message: "a type alias is not a constructor — construct the target type"
                        .into(),
                },
                ty_decl.span,
            ));
            for a in args {
                let _ = self.check_expr(a);
            }
            return Type::Unknown;
        }

        if args.len() != ty_decl.fields.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: name.to_string(),
                    expected: ty_decl.fields.len(),
                    got: args.len(),
                },
                args.first().map(|a| a.span()).unwrap_or(ty_decl.span),
            ));
        }
        for (i, arg) in args.iter().enumerate() {
            if let Some(field) = ty_decl.fields.get(i) {
                let field_ty = self.type_ref_to_type(&field.ty);
                let arg_ty = self.check_expr_as(arg, Some(&field_ty));
                if !arg_ty.is_assignable_to(&field_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: field_ty.display_name(),
                            got: arg_ty.display_name(),
                            context: format!("field `{}` of `{name}`", field.name.name),
                        },
                        arg.span(),
                    ));
                }
                self.record_if_grounded_coercion(&arg_ty, &field_ty, arg.span());
            } else {
                let _ = self.check_expr(arg);
            }
        }
        Type::Struct(def_id)
    }

    fn check_args_against_params(&mut self, callee_name: &str, params: &[Param], args: &[Expr]) {
        self.check_args_collecting_types(callee_name, params, args, false);
    }

    /// Slice 50i: the taint-aware form. Returns each argument's
    /// checked type so callers can apply flow rules (prompt output
    /// contagion, the dangerous-sink refusal). `accepts_tainted`
    /// lets PROMPTS consume `Tainted<T>` where `T` is expected —
    /// analyzing untrusted content is their job (their output then
    /// carries the taint); everything else requires an explicit
    /// `trusted(...)` or a `Tainted<T>`-typed parameter.
    fn check_args_collecting_types(
        &mut self,
        callee_name: &str,
        params: &[Param],
        args: &[Expr],
        _accepts_tainted: bool,
    ) -> Vec<Type> {
        if params.len() != args.len() {
            self.errors.push(TypeError::new(
                TypeErrorKind::ArityMismatch {
                    callee: callee_name.to_string(),
                    expected: params.len(),
                    got: args.len(),
                },
                args.first().map(|a| a.span()).unwrap_or(Span::new(0, 0)),
            ));
        }
        let mut arg_types = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            if let Some(param) = params.get(i) {
                let param_ty = self.type_ref_to_type(&param.ty);
                let arg_ty = self.check_expr_as(arg, Some(&param_ty));
                // `Tainted<T>` where `T` is expected is type-compatible
                // for arg-checking purposes: the taint SINK rule (in
                // `check_tool_call`) owns the refusal, so we suppress
                // the redundant type-mismatch here whether the callee
                // accepts taint (prompts) or refuses it (dangerous
                // tools). Otherwise every blocked injection reports
                // two errors for one cause.
                let tainted_of_assignable = matches!(
                    &arg_ty, Type::Tainted(inner) if inner.is_assignable_to(&param_ty)
                );
                if !tainted_of_assignable && !arg_ty.is_assignable_to(&param_ty) {
                    self.errors.push(TypeError::new(
                        TypeErrorKind::TypeMismatch {
                            expected: param_ty.display_name(),
                            got: arg_ty.display_name(),
                            context: format!("argument {} to `{callee_name}`", i + 1),
                        },
                        arg.span(),
                    ));
                }
                self.record_if_grounded_coercion(&arg_ty, &param_ty, arg.span());
                arg_types.push(arg_ty);
            } else {
                arg_types.push(self.check_expr(arg));
            }
        }
        arg_types
    }

    pub(super) fn bump_effect(&mut self, effect: WeakEffect) {
        self.effect_frontier = self.effect_frontier.bumped(effect);
    }

    pub(super) fn update_weak_local_on_assignment(
        &mut self,
        local_id: LocalId,
        value: &Expr,
        local_ty: &Type,
    ) {
        match local_ty {
            Type::Weak(_, _) => {
                let refreshed = self.refresh_frontier_for_expr(value, local_ty);
                self.weak_refresh.insert(local_id, refreshed);
            }
            _ => {
                self.weak_refresh.remove(&local_id);
            }
        }
    }

    fn refresh_frontier_for_expr(&self, expr: &Expr, ty: &Type) -> EffectFrontier {
        match expr {
            Expr::Ident { name, .. } => match self.bindings.get(&name.span) {
                Some(Binding::Local(local_id)) if matches!(ty, Type::Weak(_, _)) => self
                    .weak_refresh
                    .get(local_id)
                    .copied()
                    .unwrap_or(self.effect_frontier),
                _ => self.effect_frontier,
            },
            _ => self.effect_frontier,
        }
    }

    fn refresh_after_upgrade(&mut self, expr: &Expr) {
        if let Expr::Ident { name, .. } = expr {
            if let Some(Binding::Local(local_id)) = self.bindings.get(&name.span) {
                self.weak_refresh.insert(*local_id, self.effect_frontier);
            }
        }
    }

    pub(super) fn merge_weak_refresh(
        &self,
        entry: &HashMap<LocalId, EffectFrontier>,
        left: &HashMap<LocalId, EffectFrontier>,
        right: &HashMap<LocalId, EffectFrontier>,
    ) -> HashMap<LocalId, EffectFrontier> {
        let mut merged = HashMap::new();
        for (local_id, ty) in &self.local_types {
            if !matches!(ty, Type::Weak(_, _)) {
                continue;
            }
            let entry_state = entry.get(local_id).copied().unwrap_or_default();
            let left_state = left.get(local_id).copied().unwrap_or(entry_state);
            let right_state = right.get(local_id).copied().unwrap_or(entry_state);
            merged.insert(*local_id, left_state.meet_min(right_state));
        }
        merged
    }
}
