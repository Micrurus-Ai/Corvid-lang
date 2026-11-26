//! The Application Contract advertises an alias as the type it stands
//! for, including an alias imported from another module whose record
//! the contract renames because the root declares a record of the same
//! name. A client or SDK generated from the contract meets no name the
//! contract does not declare.

use corvid_driver::compile_to_application_contract_with_config;

#[test]
fn an_imported_alias_is_advertised_as_the_renamed_type_it_stands_for() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("models.cor"),
        "public type Customer:\n    name: String\n\npublic type Customers = List<Customer>\npublic type Feed = Stream<Customer>\n",
    )
    .expect("write models");
    let source = r#"import "./models" as m

public type Customer:
    id: Int

public type Roster:
    members: m.Customers

tool watch_customers(roster: Roster) -> m.Feed

public agent roster(members: m.Customers) -> Roster:
    return Roster(members)

public agent watch(roster: Roster) -> m.Feed:
    return watch_customers(roster)
"#;
    let main = dir.path().join("main.cor");
    std::fs::write(&main, source).expect("write main");
    let contract = compile_to_application_contract_with_config(source, &main.to_string_lossy(), "test", None)
        .unwrap_or_else(|errors| panic!("contract builds: {errors:#?}"));

    let names: Vec<&str> = contract.types.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"Customer") && names.contains(&"Customer_2"), "{names:?}");
    assert!(!names.contains(&"Customers") && !names.contains(&"Feed"), "an alias is not a type of its own: {names:?}");
    let imported = contract.types.iter().find(|t| t.name == "Customer_2").unwrap();
    assert_eq!(imported.fields[0].name, "name");

    let roster_type = contract.types.iter().find(|t| t.name == "Roster").unwrap();
    assert_eq!(roster_type.fields[0].type_name, "List<Customer_2>");
    let roster = contract.agents.iter().find(|a| a.name == "roster").unwrap();
    assert_eq!(roster.inputs[0].type_name, "List<Customer_2>");

    let watch = contract.agents.iter().find(|a| a.name == "watch").unwrap();
    assert_eq!(watch.output_type, "Stream<Customer_2>");
    assert!(watch.capabilities.streaming, "an imported alias of a stream streams");
    assert_eq!(watch.capabilities.pagination.as_ref().map(|p| p.item_type.as_str()), Some("Customer_2"));
}
