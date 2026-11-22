//! Persist and replace embedded documents through separate Corvid CLI processes.
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use std::process::Command;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_replacement_preserves_an_index_on_failure() {
    async fn embed(Json(request): Json<Value>) -> Json<Value> {
        assert_eq!(request["model"], "local-embed");
        let texts = request["input"].as_array().unwrap();
        let vectors: Vec<_> = texts
            .iter()
            .map(|t| {
                let text = t.as_str().unwrap();
                if text == "broken" {
                    return json!([]);
                }
                if text.contains("orchid") {
                    json!([0.0, 1.0])
                } else {
                    json!([1.0, 0.0])
                }
            })
            .collect();
        Json(json!({"embeddings":vectors}))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async {
        axum::serve(listener, Router::new().route("/api/embed", post(embed)))
            .await
            .unwrap()
    });
    let tmp = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_corvid");
    let created = Command::new(bin)
        .current_dir(tmp.path())
        .args(["new", "library"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let dir = tmp.path().join("library");
    let config = dir.join("corvid.toml");
    let mut toml = std::fs::read_to_string(&config).unwrap();
    toml.push_str(&format!(
        "\n[rag]\nembedder = \"ollama\"\nmodel = \"local-embed\"\nendpoint = \"{endpoint}\"\n"
    ));
    std::fs::write(config, toml).unwrap();
    let run = |body: &str| {
        let code = format!("import \"std:rag\" use rag_ingest, rag_search, RagChunkEnvelope\n\nagent main() -> String:\n{body}\n");
        std::fs::write(dir.join("src/main.cor"), code).unwrap();
        let output = Command::new(bin)
            .current_dir(&dir)
            .args(["run", "src/main.cor"])
            .env("CORVID_IO_ROOT", &dir)
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap()
    };
    let ingest = |id: &str, source: &str, text: &str, width: u32| {
        run(&format!(
        "    result = rag_ingest(\"index.sqlite\", \"{id}\", \"{source}\", \"{text}\", {width})\n    return match result:\n        Ok(n) -> n.to_string()\n        Err(message) -> message"))
    };
    let search = || {
        run(
            r#"    found: Result<List<RagChunkEnvelope>, String> = rag_search("index.sqlite", "vehicle", 9)
    hits: List<RagChunkEnvelope> = found.unwrap_or([])
    first = hits[0]
    return first.source + "|" + first.text + "|" + hits.length().to_string() + "|" + first.start_char.to_string() + "|" + first.end_char.to_string() + "|" + first.provenance_key"#,
        )
    };
    assert_eq!(ingest("flower", "garden", "orchid", 30).trim(), "\"1\"");
    let count = ingest("note", "old-note", "abcdefghi", 4);
    assert_eq!(count.trim(), "\"3\"");
    let old = search();
    assert!(old.contains("old-note|") && old.contains("|4|"), "{old}");
    assert!(ingest("note", "new-note", "broken", 30)
        .to_lowercase()
        .contains("embedding"));
    assert_eq!(search(), old);

    // Force the last SQL insert to fail after the replacement has begun.
    let db = rusqlite::Connection::open(dir.join("index.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_vector BEFORE INSERT ON rag_chunk_embeddings BEGIN SELECT RAISE(ABORT,'vector write failed'); END").unwrap();
    assert!(ingest("note", "new-note", "café", 30).contains("vector write failed"));
    assert_eq!(search(), old);
    db.execute_batch("DROP TRIGGER fail_vector").unwrap();
    assert_eq!(ingest("note", "new-note", "café", 30).trim(), "\"1\"");
    let updated = search();
    assert!(
        updated.contains("new-note|café|2|0|4|rag_"),
        "{updated}"
    );
    // The query has no terms in common with the documents. A lexical-only
    // service would return no hits here.
    std::fs::write(
        dir.join("src/main.cor"),
        r#"import "std:rag" use rag_search, RagChunkEnvelope

server library:
    route GET "/find" -> json List<RagChunkEnvelope>:
        found: Result<List<RagChunkEnvelope>, String> = rag_search("index.sqlite", "vehicle", 1)
        return found.unwrap_or([])
"#,
    )
    .unwrap();
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut server = Server(
        Command::new(bin)
            .current_dir(&dir)
            .args(["serve", "src/main.cor", "--listen", &address.to_string()])
            .env("CORVID_IO_ROOT", &dir)
            .env("NO_PROXY", "127.0.0.1")
            .spawn()
            .unwrap(),
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let hits: Value = loop {
        match client.get(format!("http://{address}/find")).send().await {
            Ok(response) => break response.error_for_status().unwrap().json().await.unwrap(),
            Err(_) => {
                assert!(
                    server.0.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    };
    assert_eq!(hits[0]["source"], "new-note");
    assert_eq!(hits[0]["text"], "caf\u{e9}");
    assert_eq!(hits[0]["start_char"], 0);
    assert_eq!(hits[0]["end_char"], 4);
    assert!(hits[0]["provenance_key"]
        .as_str()
        .unwrap()
        .starts_with("rag_"));
    drop(server);
    assert_eq!(ingest("note", "empty", "", 30).trim(), "\"0\"");
    assert!(search().contains("garden|orchid|1|"));
    provider.abort();
}
