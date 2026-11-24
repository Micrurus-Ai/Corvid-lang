//! B03 — a Corvid program imports std/uuid, generates UUIDs through
//! the executing tools, and checks them with `parse_uuid`. The
//! checker refuses the generators inside `@deterministic` agents, as
//! it refuses every tool call there.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::fs;

fn project_with_uuid_module() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("tempdir");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    fs::copy(repo.join("std").join("uuid.cor"), project.path().join("src").join("std").join("uuid.cor")).unwrap();
    project
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generated_uuids_parse_back_to_themselves() {
    let project = project_with_uuid_module();
    let source = r#"
import "./std/uuid" use uuid_v4, uuid_v7

agent main() -> String:
    random = uuid_v4()
    ordered = uuid_v7()
    later = uuid_v7()
    canonical = random.parse_uuid() == Ok(random) and ordered.parse_uuid() == Ok(ordered)
    versions = random.substring(14, 15) == "4" and ordered.substring(14, 15) == "7"
    distinct = random != ordered and ordered != later
    if canonical and versions and distinct and random.length() == 36:
        return "UUIDS WORK"
    return "MISMATCH " + random + " " + ordered
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();
    let ir = compile_to_ir_with_config_at_path(source, &main_path, None).expect("uuid source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("uuid program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "UUIDS WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn uuid_generation_is_refused_in_deterministic_agents() {
    let project = project_with_uuid_module();
    let source = r#"
import "./std/uuid" use uuid_v4

@deterministic
agent new_key() -> String:
    return uuid_v4()
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();
    assert!(
        compile_to_ir_with_config_at_path(source, &main_path, None).is_err(),
        "a tool call inside @deterministic must not compile"
    );
}
