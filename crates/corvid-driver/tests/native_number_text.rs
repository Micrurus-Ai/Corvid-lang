//! A `Float` interpolated into a prompt reads the same on both tiers.
//! Native code formatted it with C's `%.17g`, so a prompt about `0.1`
//! asked the model about `0.10000000000000001`, and the recorded prompt
//! differed from the interpreter's. Both now write the JSON number text
//! (`corvid_runtime::number_text::float_json`).

use std::process::Command;
use std::sync::Arc;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{MockAdapter, Runtime};
use corvid_trace_schema::{read_events_from_path, TraceEvent};

const PROGRAM: &str = r#"
prompt describe(price: Float, ratio: Float) -> String:
    """Price {price}, ratio {ratio}"""

agent main() -> String:
    return describe(0.1, 2.0)
"#;

fn rendered_prompt(trace: &std::path::Path) -> String {
    read_events_from_path(trace)
        .unwrap()
        .into_iter()
        .find_map(|event| match event {
            TraceEvent::LlmCall { rendered, .. } => rendered,
            _ => None,
        })
        .expect("the prompt call is recorded with its text")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_float_in_a_prompt_reads_the_same_on_both_tiers() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("main.cor");
    std::fs::write(&source, PROGRAM).unwrap();
    let ir = compile_to_ir_with_config_at_path(PROGRAM, &source, None).expect("program compiles");
    let traces = tempfile::tempdir().unwrap();

    let runtime = Runtime::builder()
        .llm(Arc::new(MockAdapter::new("mock-1").reply("describe", serde_json::json!("ok"))))
        .default_model("mock-1")
        .trace_to(traces.path())
        .build();
    run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("interpreter run");
    let interpreted = rendered_prompt(runtime.tracer().path());
    assert_eq!(interpreted, "Price 0.1, ratio 2.0");

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .unwrap();
    assert!(status.success());
    std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    let binary = build_or_get_cached_native(&source, PROGRAM, &ir, None).expect("native build").path;
    let native_trace = traces.path().join("native.jsonl");
    let output = Command::new(&binary)
        .env("CORVID_TRACE_PATH", &native_trace)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_RESPONSE", "ok")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(rendered_prompt(&native_trace), interpreted, "both tiers send the model the same text");
}

/// A text argument reads as its JSON form in a prompt, quoted and
/// escaped, on both tiers: the grammar's rule ("any typed parameter
/// renders as its JSON form"), which native code broke by inserting the
/// raw text.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_text_argument_reads_as_its_json_form_on_both_tiers() {
    let program = "prompt greet(name: String, count: Int) -> String:\n    \"\"\"Greet {name} ({count})\"\"\"\n\nagent main() -> String:\n    return greet(\"Ann \\\"A\\\" \\\\ new\\nline\\ttab é\", 2) + greet(\"plain\", 3)\n";
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let source = src.join("main.cor");
    std::fs::write(&source, program).unwrap();
    let ir = compile_to_ir_with_config_at_path(program, &source, None).expect("program compiles");
    let traces = tempfile::tempdir().unwrap();

    let runtime = Runtime::builder()
        .llm(Arc::new(MockAdapter::new("mock-1").reply("greet", serde_json::json!("ok"))))
        .default_model("mock-1")
        .trace_to(traces.path())
        .build();
    run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect("interpreter run");
    let interpreted = all_rendered(runtime.tracer().path());
    assert_eq!(interpreted.len(), 2);
    assert!(interpreted[1].contains("Greet \"plain\" (3)"), "{interpreted:?}");

    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .unwrap();
    assert!(status.success());
    std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    let binary = build_or_get_cached_native(&source, program, &ir, None).expect("native build").path;
    let native_trace = traces.path().join("native.jsonl");
    let output = Command::new(&binary)
        .env("CORVID_TRACE_PATH", &native_trace)
        .env("CORVID_MODEL", "mock-1")
        .env("CORVID_TEST_MOCK_LLM", "1")
        .env("CORVID_TEST_MOCK_LLM_RESPONSE", "ok")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(all_rendered(&native_trace), interpreted, "both tiers send the model the same text");
}

fn all_rendered(trace: &std::path::Path) -> Vec<String> {
    read_events_from_path(trace)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event {
            TraceEvent::LlmCall { rendered, .. } => rendered,
            _ => None,
        })
        .collect()
}
