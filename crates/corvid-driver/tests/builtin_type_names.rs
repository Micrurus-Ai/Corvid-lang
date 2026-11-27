//! A type imported under a built-in's name, the case that broke every
//! reference app when `Actor` became a built-in: the app imported its own
//! `Actor` from a module, and the built-in won. The imported type now wins
//! in the importing file, with a warning naming the built-in it hides.

use corvid_driver::compile_to_ir_with_config_at_path;

#[test]
fn an_imported_type_named_like_a_built_in_is_the_imported_type() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("people.cor"),
        "public type Caller:\n    nickname: String\n",
    )
    .unwrap();
    let main = dir.path().join("main.cor");
    let source = "import \"./people\" use Caller\n\nagent greet(who: Caller) -> String:\n    return who.nickname\n";
    std::fs::write(&main, source).unwrap();
    let ir = compile_to_ir_with_config_at_path(source, &main, None)
        .unwrap_or_else(|errors| panic!("the imported `Caller` and its field must be found: {errors:#?}"));
    let greet = ir.agents.iter().find(|a| a.name == "greet").expect("greet");
    assert!(
        !matches!(greet.params[0].ty, corvid_types::Type::RouteParams(_)),
        "the parameter is the imported record, not the built-in caller: {:?}",
        greet.params[0].ty
    );
}
