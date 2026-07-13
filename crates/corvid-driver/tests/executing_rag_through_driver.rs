//! Slice 46g — end-to-end test for the executing retrieval surface:
//! a real Corvid program ingests a document into a SQLite index and
//! retrieves it by term-scored lexical search (no embedder
//! configured — the honest degradation path), with the index path
//! confined by the `[io] root` policy.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{IoToolPolicy, Runtime};
use corvid_vm::Value;
use std::fs;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rag_ingest_and_search_end_to_end() {
    let project = tempfile::tempdir().expect("tempdir");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    for module in ["rag.cor", "effects.cor"] {
        fs::copy(
            repo.join("std").join(module),
            project.path().join("src").join("std").join(module),
        )
        .unwrap();
    }

    let source = r#"
import "./std/rag" use rag_ingest, rag_search, RagChunkEnvelope

agent main() -> String:
    doc = "Corvid is an AI-native language. Effects are typed. Replay is deterministic. Budgets are compile-checked."
    ingested = rag_ingest("index.sqlite", "doc-1", "notes", doc, 60)
    count = ingested.unwrap_or(0)
    found: Result<List<RagChunkEnvelope>, String> = rag_search("index.sqlite", "deterministic replay", 2)
    hits: List<RagChunkEnvelope> = found.unwrap_or([])
    if count > 0 and hits.length() > 0:
        first = hits[0]
        if first.provenance_key != "" and first.text.contains("Replay"):
            return "RAG RETRIEVES"
        return "WRONG CHUNK"
    return "MISMATCH"
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();

    let ir = compile_to_ir_with_config_at_path(source, &main_path, None)
        .expect("46g e2e source must compile");
    let runtime = Runtime::builder()
        .io_policy(IoToolPolicy::new(Some("."), Some(project.path())))
        .build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("46g e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "RAG RETRIEVES"),
        other => panic!("expected String, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rag_path_policy_fails_closed() {
    // Without an [io] root, the index path must be REFUSED — the
    // failure arrives as an Err value, not a trap.
    let project = tempfile::tempdir().expect("tempdir");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    for module in ["rag.cor", "effects.cor"] {
        fs::copy(
            repo.join("std").join(module),
            project.path().join("src").join("std").join(module),
        )
        .unwrap();
    }
    let source = r#"
import "./std/rag" use rag_ingest

agent main() -> String:
    result = rag_ingest("index.sqlite", "d", "s", "text", 50)
    if result.is_err():
        return "FAILS CLOSED"
    return "POLICY HOLE"
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();
    let ir = compile_to_ir_with_config_at_path(source, &main_path, None)
        .expect("source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "FAILS CLOSED"),
        other => panic!("expected String, got {other:?}"),
    }
}
