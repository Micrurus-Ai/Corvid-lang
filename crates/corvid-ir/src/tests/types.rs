use super::*;

    #[test]
    fn lowers_grounded_type_refs_to_ir_grounded_types() {
        let src = "\
effect retrieval:
    data: grounded

tool grounded_echo(name: String) -> Grounded<String> uses retrieval

pub extern \"c\"
agent grounded_lookup(name: String) -> Grounded<String>:
    return grounded_echo(name)
";
        let ir = lower_src(src);
        assert!(matches!(
            &ir.tools[0].return_ty,
            corvid_types::Type::Grounded(inner) if matches!(&**inner, corvid_types::Type::String)
        ));
        assert!(matches!(
            &ir.agents[0].return_ty,
            corvid_types::Type::Grounded(inner) if matches!(&**inner, corvid_types::Type::String)
        ));
    }

    #[test]
    fn return_value_at_grounded_coercion_site_lowers_to_unwrap_grounded() {
        // Provenance Propagation D5 (slice 7b): the typechecker
        // recorded the return value's span in
        // `Checked.grounded_coercion_sites` because `fetch(id)` is
        // `Grounded<String>` but the agent returns plain `String`. IR
        // lowering must wrap the value in `UnwrapGrounded` so the
        // discard is IR-visible — `@grounded_pure` (slice 9) walks
        // for this node to fail the moat. Without slice 7b the
        // typechecker recorded the site but the IR was silent.
        let src = "\
effect retrieval:
    data: grounded

tool fetch(id: String) -> String uses retrieval

agent leak(id: String) -> String:
    return fetch(id)
";
        let ir = lower_src(src);
        let agent = &ir.agents[0];
        let return_stmt = agent
            .body
            .stmts
            .iter()
            .find_map(|s| match s {
                crate::types::IrStmt::Return { value: Some(e), .. } => Some(e),
                _ => None,
            })
            .expect("return statement");
        match &return_stmt.kind {
            crate::types::IrExprKind::UnwrapGrounded { value } => {
                assert!(
                    matches!(value.ty, corvid_types::Type::Grounded(_)),
                    "inner value should still be `Grounded<T>` so codegen \
                     can route on the wrapped type; got {:?}",
                    value.ty,
                );
                assert!(
                    matches!(return_stmt.ty, corvid_types::Type::String),
                    "outer `UnwrapGrounded` type should be the stripped \
                     inner, not `Grounded<String>`; got {:?}",
                    return_stmt.ty,
                );
            }
            other => panic!(
                "expected return value to be `UnwrapGrounded(Call)`, got {other:?}"
            ),
        }
    }

    #[test]
    fn no_unwrap_grounded_when_slot_is_already_grounded() {
        // Returning `Grounded<String>` into a `-> Grounded<String>`
        // slot is not a coercion. The typechecker records nothing
        // and the IR must not insert a discard.
        let src = "\
effect retrieval:
    data: grounded

tool fetch(id: String) -> String uses retrieval

agent keep(id: String) -> Grounded<String>:
    return fetch(id)
";
        let ir = lower_src(src);
        let agent = &ir.agents[0];
        let return_stmt = agent
            .body
            .stmts
            .iter()
            .find_map(|s| match s {
                crate::types::IrStmt::Return { value: Some(e), .. } => Some(e),
                _ => None,
            })
            .expect("return statement");
        assert!(
            !matches!(
                return_stmt.kind,
                crate::types::IrExprKind::UnwrapGrounded { .. }
            ),
            "no discard expected when slot accepts `Grounded<String>`; got {:?}",
            return_stmt.kind,
        );
    }

    #[test]
    fn lowers_stream_partial_prompt_return_type() {
        let src = "\
type Plan:
    title: String
    body: String

prompt plan(topic: String) -> Stream<Partial<Plan>>:
    \"Plan {topic}\"
";
        let ir = lower_src(src);
        let prompt = &ir.prompts[0];
        match &prompt.return_ty {
            corvid_types::Type::Stream(inner) => match &**inner {
                corvid_types::Type::Partial(partial_inner) => {
                    assert!(matches!(&**partial_inner, corvid_types::Type::Struct(_)));
                }
                other => panic!("expected Partial<T>, got {other:?}"),
            },
            other => panic!("expected Stream<T>, got {other:?}"),
        }
    }
