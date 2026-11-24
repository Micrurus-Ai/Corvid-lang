//! IR signatures and record fields carry the type the checker
//! resolved, with aliases expanded whether they are local or declared in
//! another module. IR lowering reads these types
//! from the checker (`Checked::type_refs`) rather than resolving type
//! references a second time.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_ir::IrFile;
use corvid_runtime::Runtime;
use corvid_types::Type;
use corvid_vm::Value;

const MODELS: &str = "
public type Customer:
    id: Int

public type Customers = List<Customer>
";

fn compile(main: &str) -> IrFile {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("models.cor"), MODELS).unwrap();
    let path = dir.path().join("main.cor");
    std::fs::write(&path, main).unwrap();
    compile_to_ir_with_config_at_path(main, &path, None).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"))
}

fn param_type<'a>(ir: &'a IrFile, agent: &str) -> &'a Type {
    &ir.agents.iter().find(|a| a.name == agent).unwrap_or_else(|| panic!("no agent {agent}")).params[0].ty
}

fn is_list_of_record(ty: &Type, record: &str, ir: &IrFile) -> bool {
    match ty {
        Type::List(inner) => match inner.as_ref() {
            Type::Struct(id) => ir.types.iter().any(|t| t.id == *id && t.name == record),
            Type::ImportedStruct(imported) => imported.name == record,
            _ => false,
        },
        _ => false,
    }
}

const MAIN: &str = r#"import "./models" as m
import "./models" use Customers

type Order:
    id: Int

type Orders = List<Order>

type Batch:
    orders: Orders
    customers: m.Customers

agent local(orders: Orders) -> Int:
    return orders.length()

agent qualified(customers: m.Customers) -> Int:
    return customers.length()

agent lifted(customers: Customers) -> Int:
    return customers.length()

"#;

#[test]
fn a_local_alias_lowers_to_its_target() {
    let ir = compile(MAIN);
    assert!(is_list_of_record(param_type(&ir, "local"), "Order", &ir), "{:?}", param_type(&ir, "local"));
}

#[test]
fn an_alias_from_another_module_lowers_to_its_target_in_both_import_spellings() {
    let ir = compile(MAIN);
    for agent in ["qualified", "lifted"] {
        assert!(
            is_list_of_record(param_type(&ir, agent), "Customer", &ir),
            "{agent}: {:?}",
            param_type(&ir, agent)
        );
    }
}

#[test]
fn record_fields_typed_with_aliases_lower_to_their_targets() {
    let ir = compile(MAIN);
    let batch = ir.types.iter().find(|t| t.name == "Batch").expect("Batch is lowered");
    let field = |name: &str| &batch.fields.iter().find(|f| f.name == name).unwrap().ty;
    assert!(is_list_of_record(field("orders"), "Order", &ir), "{:?}", field("orders"));
    assert!(is_list_of_record(field("customers"), "Customer", &ir), "{:?}", field("customers"));
}

/// The runtime consequence: a record whose field is typed with an
/// alias decodes through the alias's target, not an empty record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_record_with_an_alias_typed_field_decodes() {
    let ir = compile(&format!(
        "{MAIN}\nagent main() -> String:\n    return match Batch.from_json(\"{{\\\"orders\\\": [{{\\\"id\\\": 7}}], \\\"customers\\\": [{{\\\"id\\\": 1}}, {{\\\"id\\\": 2}}]}}\"):\n        Ok(batch) -> batch.orders[0].id.to_string() + \" \" + batch.customers.length().to_string()\n        Err(problem) -> problem.path + \": \" + problem.message\n"
    ));
    let out = run_ir_with_runtime(&ir, None, vec![], &Runtime::builder().build()).await.expect("runs");
    match out {
        Value::String(text) => assert_eq!(text.as_ref(), "7 2"),
        other => panic!("expected a String, got {other:?}"),
    }
}

/// The ABI descriptor reads the same checker table as IR lowering, so
/// an alias-typed parameter is described by its target, not as a
/// record named after the alias.
#[test]
fn the_abi_describes_an_alias_typed_parameter_by_its_target() {
    // A C-exported agent takes scalars and scalar-field records, so the
    // alias here names a scalar.
    let source = "type Cents = Int\n\npub extern \"c\"\nagent double(amount: Cents) -> Int:\n    return amount * 2\n";
    let abi = corvid_driver::compile_to_abi_with_config(source, "main.cor", "2026-11-23T00:00:00Z", None)
        .unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
    let agent = abi.agents.iter().find(|a| a.name == "double").expect("double is exported");
    let described = format!("{:?}", agent.params[0].ty);
    let returned = format!("{:?}", agent.return_type);
    assert_eq!(described, returned, "the alias must be described as its target, Int");
    assert!(!described.contains("Cents"), "the alias name must not appear as a record: {described}");
}