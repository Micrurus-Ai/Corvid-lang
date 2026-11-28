//! B11-1 — assistant authority, in source. `delegation: up_to <duration>`
//! in the identity block caps how long a person may grant an assistant
//! authority (from one minute to 720 hours, with no default), and
//! `assistants: allow "..."` lets an assistant run an action without
//! asking each time. The contract carries both.

use corvid_driver::{compile_to_application_contract_with_config, compile_to_ir};

fn program(delegation: &str, mode: &str) -> String {
    format!(
        r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        editor: "cards.edit"
{delegation}
action label_card(card: Int, label: String, by: Caller) -> String requires permission("cards.edit"):
    assistants: {mode} "Add a label to a card."
    return label
"#
    )
}

fn errors(source: &str) -> Vec<String> {
    match compile_to_ir(source) {
        Ok(_) => Vec::new(),
        Err(errors) => errors.into_iter().map(|e| e.message).collect(),
    }
}

fn contract(source: &str) -> serde_json::Value {
    let contract = compile_to_application_contract_with_config(source, "app.cor", "test", None)
        .unwrap_or_else(|e| panic!("must compile: {e:#?}"));
    serde_json::to_value(contract).unwrap()
}

#[test]
fn the_contract_carries_the_cap_and_the_mode() {
    let source = program("    delegation: up_to 8 hours\n", "allow");
    assert!(errors(&source).is_empty(), "{:#?}", errors(&source));
    let contract = contract(&source);
    assert_eq!(contract["identities"][0]["delegation_up_to_secs"], 8 * 3600);
    let action = contract["actions"].as_array().unwrap().iter().find(|a| a["name"] == "label_card").expect("action listed");
    assert_eq!(action["assistants"]["mode"], "allow", "{action:#}");
}

#[test]
fn without_the_line_no_grant_can_be_issued() {
    let contract = contract(&program("", "confirm"));
    assert!(contract["identities"][0].get("delegation_up_to_secs").is_none(), "no default lifetime");
}

#[test]
fn a_cap_is_between_a_minute_and_thirty_days() {
    for (cap, said) in [
        ("30 seconds", "1 minute"),
        ("721 hours", "never be permanent"),
        ("30 days", "no fixed length"),
    ] {
        let found = errors(&program(&format!("    delegation: up_to {cap}\n"), "confirm"));
        assert!(found.iter().any(|e| e.contains(said)), "`{cap}`: {found:#?}");
    }
    let missing = errors(&program("    delegation: 8 hours\n", "confirm"));
    assert!(missing.iter().any(|e| e.contains("`up_to`")), "{missing:#?}");
}
