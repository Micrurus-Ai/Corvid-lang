//! End-to-end pins for the executing secrets + cache surfaces.
//!
//! Secrets carry a replay-safe trace contract: the program receives
//! the real value, the recorded ToolResult carries a redacted copy,
//! and replay re-executes the env read instead of substituting (the
//! db_query read-passthrough rule). The cache tools drive the shared
//! in-memory CacheRuntime with (namespace, subject) addressing and
//! invalidation by invalidation key or provenance key.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::path::Path;

/// Stage a project whose `src/std/` vendors the modules these tests
/// import, so `import "./std/secrets"` resolves through the normal
/// module pipeline — the executing_io_through_driver pattern.
fn stage_project(dir: &Path, main_source: &str) -> std::path::PathBuf {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root resolves to two parents up");
    let src = dir.join("src");
    let std_dir = src.join("std");
    std::fs::create_dir_all(&std_dir).unwrap();
    for file in ["effects.cor", "secrets.cor", "cache.cor"] {
        std::fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    std::fs::write(dir.join("corvid.toml"), "[io]\nroot = \".\"\n").unwrap();
    let main_path = src.join("main.cor");
    std::fs::write(&main_path, main_source).unwrap();
    main_path
}

fn compile_to_ir(source: &str) -> Result<corvid_ir::IrFile, Vec<corvid_driver::Diagnostic>> {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(project.path(), source);
    let result = compile_to_ir_with_config_at_path(source, &main_path, None);
    // Leak the tempdir: the IR is fully lowered before this returns,
    // so dropping the dir is safe — but leaking keeps span-to-source
    // debugging possible if a test fails mid-investigation.
    std::mem::forget(project);
    result
}

fn expect_ok_string(result: Value) -> String {
    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::String(s) => s.to_string(),
            other => panic!("expected Ok(String); got Ok({other:?})"),
        },
        other => panic!("expected ResultOk; got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secret_read_returns_real_value_to_the_program() {
    std::env::set_var("CORVID_48A_E2E_SECRET", "hunter2-value");
    let source = "
import \"./std/secrets\" use secret_read

agent main() -> Result<String, String>:
    secret = secret_read(\"CORVID_48A_E2E_SECRET\")?
    return Ok(secret.value)
";
    let ir = compile_to_ir(source).expect("secrets source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("secret read must run");
    assert_eq!(expect_ok_string(out), "hunter2-value");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secret_read_missing_is_ok_with_present_false() {
    std::env::remove_var("CORVID_48A_E2E_MISSING");
    let source = "
import \"./std/secrets\" use secret_read

agent main() -> Result<Bool, String>:
    secret = secret_read(\"CORVID_48A_E2E_MISSING\")?
    return Ok(secret.present)
";
    let ir = compile_to_ir(source).expect("secrets source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("missing secret must not trap");
    match out {
        Value::ResultOk(inner) => match inner.get() {
            Value::Bool(false) => {}
            other => panic!("missing secret must be Ok(present=false); got {other:?}"),
        },
        other => panic!("expected ResultOk; got {other:?}"),
    }
}

/// The load-bearing trace contract: the recorded ToolResult for a
/// secret_read carries the redacted marker, never the value — while
/// the program (asserted above) receives the real value.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secret_value_never_lands_in_the_trace() {
    std::env::set_var("CORVID_48A_TRACE_SECRET", "super-sekret-42");
    let source = "
import \"./std/secrets\" use secret_read

agent main() -> Result<String, String>:
    secret = secret_read(\"CORVID_48A_TRACE_SECRET\")?
    return Ok(secret.name)
";
    let ir = compile_to_ir(source).expect("secrets source must compile");
    let trace_dir = tempfile::tempdir().expect("trace dir");
    let tracer = corvid_runtime::tracing::Tracer::open(trace_dir.path(), "48a-secret-trace");
    let runtime = Runtime::builder().build().with_tracer(tracer);
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("traced secret read must run");
    assert_eq!(expect_ok_string(out), "CORVID_48A_TRACE_SECRET");

    let trace_path = trace_dir.path().join("48a-secret-trace.jsonl");
    let trace = std::fs::read_to_string(&trace_path).expect("trace file exists");
    assert!(
        !trace.contains("super-sekret-42"),
        "the trace must NEVER contain the secret value; trace:\n{trace}"
    );
    assert!(
        trace.contains("<redacted:42>"),
        "the trace must carry the redacted marker; trace:\n{trace}"
    );
    assert!(
        trace.contains("\"value_redacted\":true"),
        "the recorded envelope must flag the redaction; trace:\n{trace}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_put_get_roundtrip_and_invalidation() {
    let source = "
import \"./std/cache\" use cache_put, cache_get, cache_invalidate

agent main() -> Result<String, String>:
    cache_put(\"answers\", \"greeting\", \"hello-cached\", \"greetings\", \"doc:hello\")?
    found = cache_get(\"answers\", \"greeting\")?
    if not found.hit:
        return Err(\"expected a cache hit\")
    evicted = cache_invalidate(\"greetings\")?
    if evicted != 1:
        return Err(\"expected exactly one eviction\")
    gone = cache_get(\"answers\", \"greeting\")?
    if gone.hit:
        return Err(\"expected a miss after invalidation\")
    return Ok(found.value)
";
    let ir = compile_to_ir(source).expect("cache source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("cache roundtrip must run");
    assert_eq!(expect_ok_string(out), "hello-cached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_provenance_invalidation_evicts_derived_entries() {
    let source = "
import \"./std/cache\" use cache_put, cache_get, cache_invalidate_provenance

agent main() -> Result<Int, String>:
    cache_put(\"summaries\", \"doc-a\", \"summary of a\", \"\", \"doc:a\")?
    cache_put(\"summaries\", \"doc-b\", \"summary of b\", \"\", \"doc:a\")?
    cache_put(\"summaries\", \"doc-c\", \"summary of c\", \"\", \"doc:c\")?
    evicted = cache_invalidate_provenance(\"doc:a\")?
    survivor = cache_get(\"summaries\", \"doc-c\")?
    if not survivor.hit:
        return Err(\"unrelated provenance must survive\")
    return Ok(evicted)
";
    let ir = compile_to_ir(source).expect("cache source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("provenance invalidation must run");
    match out {
        Value::ResultOk(inner) => match inner.get() {
            Value::Int(2) => {}
            other => panic!("both doc:a-derived entries must evict; got {other:?}"),
        },
        other => panic!("expected ResultOk; got {other:?}"),
    }
}

/// Overwriting the same (namespace, subject) address must not
/// accumulate duplicates — the second put wins.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_put_overwrites_same_address() {
    let source = "
import \"./std/cache\" use cache_put, cache_get

agent main() -> Result<String, String>:
    cache_put(\"kv\", \"slot\", \"first\", \"\", \"src:1\")?
    cache_put(\"kv\", \"slot\", \"second\", \"\", \"src:2\")?
    found = cache_get(\"kv\", \"slot\")?
    return Ok(found.value)
";
    let ir = compile_to_ir(source).expect("cache source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("overwrite must run");
    assert_eq!(expect_ok_string(out), "second");
}
