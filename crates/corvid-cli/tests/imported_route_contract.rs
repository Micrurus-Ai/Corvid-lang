use serde_json::{json, Value};
use std::{fs, process::Command};

#[test]
fn contract_resolves_reachable_records_in_their_declaring_module() {
    let dir = tempfile::tempdir().unwrap();
    for (name, source) in [
        (
            "left.cor",
            "public type Item:\n    title: String\n\npublic type Unused:\n    key: String\n",
        ),
        ("right.cor", "public type Item:\n    count: Int\n"),
        (
            "shared.cor",
            r#"import "./left" use Item as Entry
public type Search:
    entries: List<Entry>
"#,
        ),
        (
            "main.cor",
            r#"import "./shared" use Search as Batch
import "./right" as right

server api:
    route POST "/batch" body Batch -> json right.Item:
        return right.Item(3)
"#,
        ),
    ] {
        fs::write(dir.path().join(name), source).unwrap();
    }
    let emit = |format| {
        let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(dir.path())
            .args(["contract", format, "main.cor", "--out", "-"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let spec = emit("openapi");
    let schemas = &spec["components"]["schemas"];
    let route = &spec["paths"]["/batch"]["post"];
    let schema = |reference: &Value| {
        &schemas[reference["$ref"]
            .as_str()
            .unwrap()
            .strip_prefix("#/components/schemas/")
            .unwrap()]
    };
    let request = schema(&route["requestBody"]["content"]["application/json"]["schema"]);
    let entry = schema(&request["properties"]["entries"]["items"]);
    assert_eq!(entry["properties"]["title"]["type"], "string");
    let response = schema(&route["responses"]["200"]["content"]["application/json"]["schema"]);
    assert_eq!(response["properties"]["count"]["type"], "integer");
    assert_eq!(schemas.as_object().unwrap().len(), 3);
    let contract = emit("app");
    assert_eq!(contract["types"].as_array().unwrap().len(), 3);
    assert_ne!(contract["routes"][0]["body_type"], json!(null));
    assert_eq!(emit("openapi"), spec);
}

#[test]
fn route_contract_includes_private_dependencies_of_a_public_alias() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.cor");
    fs::write(
        &main,
        r#"
type Note:
    text: String

public type Notes = List<Note>
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["contract", "openapi"])
        .arg(main)
        .args(["--out", "-"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let spec: Value = serde_json::from_slice(&output.stdout).unwrap();
    let schemas = &spec["components"]["schemas"];
    assert_eq!(schemas["Notes"]["type"], "array");
    assert_eq!(
        schemas["Notes"]["items"]["$ref"],
        "#/components/schemas/Note"
    );
    assert_eq!(schemas["Note"]["properties"]["text"]["type"], "string");
}
