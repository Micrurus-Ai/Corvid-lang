//! Phase 33S4 — CI guard for the "Talking to the outside world"
//! book chapter pipeline.
//!
//! The chapter at `docs/book/18-talking-to-the-outside-world.md`
//! describes an end-to-end HTTP → typed-decoder JSON → SQLite
//! pipeline using zero Python glue — the load-bearing
//! acceptance claim of the 33S + 33R5b umbrella. This test
//! lifts the chapter's `src/main.cor` body verbatim and runs it
//! through the same driver pipeline `corvid run` uses, against
//! a wiremock-served loopback HTTP endpoint (routed via
//! reqwest's `.resolve()` DNS override — the no-shortcut
//! pattern 33S2b established).
//!
//! Acceptance:
//!
//! - The chapter's source compiles cleanly through
//!   `compile_to_ir_with_config_at_path`.
//! - The pipeline runs end-to-end through `run_ir_with_runtime`.
//! - The Corvid program does an HTTP GET, decodes the JSON
//!   response into a `User` struct via the typed-decoder
//!   convention, opens a `:memory:` SQLite database, creates a
//!   table, parameter-binds an INSERT, runs a SELECT, returns
//!   `Ok(rows_affected)` (0 for the SELECT envelope).
//!
//! When this test passes, the chapter is verified: the
//! advertised zero-glue pipeline ACTUALLY runs end-to-end.
//! When the test breaks, the chapter is wrong — the author
//! must either fix the chapter or admit the umbrella didn't
//! ship what was promised.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, IoToolPolicy, Runtime};
use corvid_vm::Value;
use std::fs;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Stage a fresh project at `dir` whose `src/std/` is a vendored
/// copy of the workspace stdlib (incl. all four executing-I/O
/// modules) and whose `corvid.toml` carries the standard
/// `[io] root` + `[http] allow` shape the chapter demonstrates.
fn stage_book_pipeline_project(dir: &Path, main_source: &str) -> std::path::PathBuf {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root resolves to two parents up");
    let src = dir.join("src");
    let std_dir = src.join("std");
    fs::create_dir_all(&std_dir).unwrap();
    for file in ["effects.cor", "io.cor", "http.cor", "db.cor", "json.cor"] {
        fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    fs::write(
        dir.join("corvid.toml"),
        "[io]\nroot = \".\"\n\n[http]\nallow = [\"api.example.com\"]\n",
    )
    .unwrap();
    let main_path = src.join("main.cor");
    fs::write(&main_path, main_source).unwrap();
    main_path
}

/// Build a reqwest client that resolves `api.example.com` to the
/// loopback wiremock socket. Same pattern as
/// `executing_http_through_driver.rs::loopback_resolving_client`.
fn loopback_resolving_client(host: &str, server: &MockServer) -> reqwest::Client {
    let addr: std::net::SocketAddr = *server.address();
    reqwest::Client::builder()
        .resolve(host, addr)
        .build()
        .expect("reqwest client with .resolve override should build")
}

/// 33S4 — the quickstart's executing-I/O example also compiles
/// and runs end-to-end. The `02-quickstart.md` chapter advertises
/// a small `io_read_text` snippet as the first real executing-I/O
/// example a new user sees; if it stops compiling or stops
/// reading the file, the quickstart is broken.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quickstart_executing_io_snippet_compiles_and_reads_the_file() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_book_pipeline_project(
        project.path(),
        r#"
import "./std/io" use io_read_text

agent main() -> Result<String, String>:
    file = io_read_text("note.txt")
    return Ok(file.contents)
"#,
    );
    // Stage `note.txt` under the project's [io] root.
    fs::write(
        project.path().join("note.txt"),
        "Corvid ships a typed effect system.",
    )
    .unwrap();

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("the quickstart's io_read_text snippet must compile");

    let io_policy = IoToolPolicy::new(Some("."), Some(project.path()));
    let runtime = Runtime::builder().io_policy(io_policy).build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("the quickstart's io_read_text snippet must run end-to-end");

    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::String(s) => assert_eq!(
                s.as_ref(),
                "Corvid ships a typed effect system.",
                "the quickstart's read result must match the file we wrote"
            ),
            other => panic!("expected Ok(String); got Ok({other:?})"),
        },
        other => panic!("expected ResultOk; got {other:?}"),
    }
}

/// 33S4 load-bearing acceptance — the book chapter pipeline
/// runs end-to-end. HTTP GET against a wiremock-served loopback
/// endpoint (DNS-routed from `api.example.com`); the response
/// body is `{"id": 7, "email": "alice@example.com"}`; the
/// typed-decoder `decode_user_from_json` converts it into a
/// `User` struct via the generic
/// `serde_json::from_str + json_to_value` dispatch path; the
/// SQLite pipeline opens `:memory:`, CREATEs, INSERTs the typed
/// param-bound row, and SELECTs it back; the agent returns
/// `Ok(rows_affected)` which is 0 for the SELECT envelope.
///
/// The Corvid source is the EXACT pipeline shape the book
/// chapter advertises — verifying the chapter is honest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn book_chapter_no_python_pipeline_runs_end_to_end_through_real_corvid_program() {
    let project = tempfile::tempdir().expect("tempdir");

    // wiremock — serves the JSON User payload the chapter
    // claims is the API's response shape.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"id": 7, "email": "alice@example.com"}"#),
        )
        .mount(&server)
        .await;

    // The Corvid source: the chapter's pipeline as code. Any
    // deviation from this shape means the chapter's claim "this
    // exact program runs end-to-end" is broken.
    let main_path = stage_book_pipeline_project(
        project.path(),
        r#"
effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

import "./std/http" use http_get
import "./std/db" use db_open, db_execute, db_query, db_param_int, db_param_text

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent ingest_user(url: String, db_path: String) -> Result<Int, String>:
    response = http_get(url)
    user = decode_user_from_json(response.body)?
    handle = db_open(db_path)
    db_execute(handle, "CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, email TEXT NOT NULL)", [])
    db_execute(handle, "INSERT INTO users(id, email) VALUES (?, ?)", [db_param_int(user.id), db_param_text(user.email)])
    rows = db_query(handle, "SELECT id FROM users WHERE id = ?", [db_param_int(user.id)])
    return Ok(rows[0].rows_affected)

agent main() -> Result<Int, String>:
    return ingest_user("http://api.example.com/users/1", ":memory:")
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("the book chapter pipeline source must compile through the driver");

    // Build the runtime with the chapter's policies + the
    // loopback-routing HTTP client. This is the same shape
    // `corvid run` would produce against a real network endpoint
    // matching the allowlisted host.
    let io_policy = IoToolPolicy::new(Some("."), Some(project.path()));
    let http_policy = HttpEgressPolicy::new(Some(&["api.example.com".to_string()]));
    let http_client =
        HttpClient::with_reqwest_client(loopback_resolving_client("api.example.com", &server));
    let runtime = Runtime::builder()
        .io_policy(io_policy)
        .http_policy(http_policy)
        .http_client(http_client)
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime).await.expect(
        "the book chapter pipeline must run end-to-end through corvid run — if this errors, \
         the chapter's `Talking to the outside world` claim is broken",
    );

    // The agent returns Result<Int, String>; the Int is the
    // envelope's `rows_affected` (0 for the SELECT envelope).
    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::Int(0) => {}
            other => panic!(
                "expected the pipeline to return Ok(0); got Ok({other:?}). \
                 The chapter advertises `rows[0].rows_affected` returns 0 for \
                 the SELECT envelope; if this is different, the chapter is wrong."
            ),
        },
        Value::ResultErr(inner) => {
            let msg = match inner.get() {
                Value::String(s) => s.to_string(),
                other => format!("{other:?}"),
            };
            panic!(
                "the pipeline returned Err — the no-Python pipeline broke somewhere: {msg}. \
                 Likely culprits: HTTP allowlist (host must be `api.example.com`), JSON shape \
                 mismatch (response body should be {{\"id\": Int, \"email\": String}}), or \
                 SQLite path confinement (`:memory:` bypasses, all other paths go through \
                 [io] root)."
            );
        }
        other => panic!(
            "expected main to return ResultOk; got {other:?} (this would mean the agent's \
             declared return type Result<Int, String> doesn't match the runtime's wrap shape)"
        ),
    }
}
