//! B06 — a Corvid program reads the clock with `time_now()`, which is
//! a traced tool, and computes with the `Instant` and `Duration` values
//! it gets back. The checker refuses the clock read inside
//! `@deterministic` agents, as it refuses every tool call there.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::fs;

fn project_with_time_module() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("tempdir");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    for module in ["time.cor", "effects.cor"] {
        fs::copy(repo.join("std").join(module), project.path().join("src").join("std").join(module)).unwrap();
    }
    project
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_clock_returns_an_instant_that_computes_exactly() {
    let project = project_with_time_module();
    let source = r#"
import "./std/time" use time_now

agent main() -> String:
    start = time_now()
    later = time_now()
    elapsed = later - start
    zero = start - start
    week = "PT168H".parse_duration().unwrap_or(zero)
    deadline = start + week
    text_round_trips = start.to_string().parse_instant() == Ok(start)
    plausible = start.epoch_milliseconds() > 1700000000000
    if elapsed >= zero and deadline - start == week and text_round_trips and plausible:
        return "INSTANTS WORK"
    return "MISMATCH " + start.to_string() + " " + elapsed.to_string()
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();
    let ir = compile_to_ir_with_config_at_path(source, &main_path, None).expect("time source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("time program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "INSTANTS WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[test]
fn the_clock_is_refused_in_deterministic_agents() {
    let project = project_with_time_module();
    let source = r#"
import "./std/time" use time_now

@deterministic
agent stamp() -> String:
    return time_now().to_string()
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();
    assert!(
        compile_to_ir_with_config_at_path(source, &main_path, None).is_err(),
        "a clock read inside @deterministic must not compile"
    );
}
