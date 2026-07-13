//! Slice 33S2b — end-to-end tests that prove a real Corvid
//! program (compiled through the driver, run through the
//! interpreter) reaches the executing HTTP-client dispatch and
//! that the policy + injected client wire together correctly.
//!
//! Architecture (the no-shortcut approach):
//!
//! The `HttpEgressPolicy::check` parses the URL string only —
//! `api.example.com` is a public-looking host to the policy
//! regardless of what DNS would resolve it to. SSRF only fires
//! on literal RFC1918 / loopback / link-local hosts in the URL
//! string. So end-to-end tests can use `http://api.example.com/`
//! as the URL (SSRF passes), put `api.example.com` in the
//! allowlist (gate passes), and then use a `reqwest::Client`
//! built with `.resolve("api.example.com", loopback_addr)` to
//! route the actual TCP connection to a `wiremock::MockServer`
//! listening on a loopback port. Production behavior is identical
//! to a real network call; only the transport endpoint differs.
//!
//! This is materially better than a test-only SSRF carve-out:
//! the SSRF guarantee remains genuinely unconditional in
//! production code, and the test still exercises every layer
//! (URL parsing, SSRF check, allowlist check, real HTTP send,
//! response marshalling).

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::{HttpClient, HttpEgressPolicy, Runtime};
use corvid_vm::Value;
use std::fs;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Stage a fresh project at `dir` whose `src/std/` is a vendored
/// copy of the workspace stdlib and whose `corvid.toml` carries
/// the supplied `[http]` / `[io]` sections verbatim. Returns the
/// path to the project's `src/main.cor` (where the test should
/// write its source).
fn stage_project(dir: &Path, corvid_toml_body: &str, main_source: &str) -> std::path::PathBuf {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root resolves to two parents up from this crate's manifest");
    let src = dir.join("src");
    let std_dir = src.join("std");
    fs::create_dir_all(&std_dir).unwrap();
    for file in ["effects.cor", "io.cor", "http.cor"] {
        fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    fs::write(dir.join("corvid.toml"), corvid_toml_body).unwrap();
    let main_path = src.join("main.cor");
    fs::write(&main_path, main_source).unwrap();
    main_path
}

/// Build a `reqwest::Client` that resolves `api.example.com` to
/// the given loopback socket. The URL passed to `http_get` /
/// `http_post_json` stays `http://api.example.com/...`, the
/// policy sees a public-looking host (SSRF passes), the
/// allowlist gate accepts `api.example.com`, and the actual TCP
/// connection terminates at `wiremock::MockServer` on loopback.
fn loopback_resolving_client(host: &str, server: &MockServer) -> reqwest::Client {
    let addr: std::net::SocketAddr = *server.address();
    reqwest::Client::builder()
        .resolve(host, addr)
        .build()
        .expect("reqwest client with .resolve override should build")
}

/// 33S2b load-bearing acceptance — compile + run a real Corvid
/// program that calls `http_get` and assert the HTTP egress
/// dispatch returned the wiremock-served status code. This is
/// the proof that the executing HTTP-client surface end-to-end
/// works from a real `.cor` source through the driver pipeline
/// through the runtime dispatch through the real reqwest client
/// to a real (loopback) HTTP responder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_corvid_program_performs_get_through_executing_http_dispatch() {
    let project = tempfile::tempdir().expect("tempdir");

    // wiremock — listens on a loopback port, serves /status with
    // a deterministic 200 response.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/status"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    // Lay out the project. corvid.toml allows the public-looking
    // host (which the policy will see in the URL string); the
    // injected reqwest client routes that host to the loopback
    // wiremock at the TCP layer.
    let main_path = stage_project(
        project.path(),
        "[io]\nroot = \".\"\n\n[http]\nallow = [\"api.example.com\"]\n",
        r#"
import "./std/http" use http_get

agent main() -> Result<Int, String>:
    response = http_get("http://api.example.com/status")?
    return Ok(response.status)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("real corvid program should compile");

    let policy = HttpEgressPolicy::new(Some(&["api.example.com".to_string()]));
    let client = HttpClient::with_reqwest_client(loopback_resolving_client(
        "api.example.com",
        &server,
    ));
    let runtime = Runtime::builder()
        .http_policy(policy)
        .http_client(client)
        .build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("executing GET should round-trip through the dispatch");

    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::Int(200) => {}
            other => panic!(
                "expected `main` to return Ok(200) from wiremock's response; \
                 got Ok({other:?}). The dispatch may have swallowed the \
                 response or marshalled it incorrectly."
            ),
        },
        other => panic!(
            "expected `main` to return Ok(200) from wiremock's response; \
             got {other:?}. The dispatch may have swallowed the response \
             or marshalled it incorrectly."
        ),
    }
}

/// 33S2b structural-property proof — a `http_get` call to a
/// loopback URL is rejected by the SSRF block BEFORE the
/// allowlist is consulted, regardless of allowlist contents.
/// We deliberately put `127.0.0.1` in the allowlist to prove
/// the SSRF block is the FLOOR — even if a misconfigured
/// project tries to allow loopback, the structural rule
/// refuses it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ssrf_block_rejects_loopback_url_even_when_allowlist_contains_it() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        // Deliberately mis-configured: 127.0.0.1 in the allow
        // list. The structural SSRF block still fires.
        "[io]\nroot = \".\"\n\n[http]\nallow = [\"127.0.0.1\"]\n",
        r#"
import "./std/http" use http_get

agent main() -> Result<Int, String>:
    response = http_get("http://127.0.0.1:9999/path")?
    return Ok(response.status)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("real corvid program should compile");

    let policy = HttpEgressPolicy::new(Some(&["127.0.0.1".to_string()]));
    let runtime = Runtime::builder().http_policy(policy).build();

    // Slice 47h: the SSRF rejection is an Err VALUE the program
    // observes (here propagated by `?`), not a trap. The run
    // itself completes; the full diagnostic must survive into
    // the Err payload.
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("the run completes; the SSRF rejection is the returned Err value");

    let detail = expect_result_err_string(result);
    assert!(
        detail.contains("SSRF") || detail.contains("private / loopback / link-local"),
        "the Err value must name the SSRF block as the cause; got: {detail}"
    );
    assert!(
        detail.contains("structural property") || detail.contains("never reachable"),
        "the Err value must explain SSRF is structural (not configurable); got: {detail}"
    );
}

/// 33S2b fail-closed proof — calling `http_get` with no `[http]
/// allow` configured at all refuses the request with a
/// diagnostic naming the missing config. Mirrors the
/// fail-closed contract of `[io] root` from 33S1.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_http_allowlist_fails_closed_with_actionable_diagnostic() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        // No [http] section at all — the loader should produce
        // an unconfigured HttpEgressPolicy, and the dispatch
        // should refuse with the missing-config diagnostic.
        "[io]\nroot = \".\"\n",
        r#"
import "./std/http" use http_get

agent main() -> Result<Int, String>:
    response = http_get("http://api.example.com/foo")?
    return Ok(response.status)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("real corvid program should compile");

    // The runtime gets the same `HttpEgressPolicy::unset()` the
    // loader would produce for a corvid.toml with no [http]
    // section.
    let runtime = Runtime::builder()
        .http_policy(HttpEgressPolicy::unset())
        .build();

    // Slice 47h: fail-closed still holds — but the refusal is an
    // Err VALUE carrying the actionable diagnostic, not a trap.
    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("the run completes; the fail-closed refusal is the returned Err value");

    let detail = expect_result_err_string(result);
    assert!(
        detail.contains("[http] allow"),
        "diagnostic must name `[http] allow`; got: {detail}"
    );
    assert!(
        detail.contains("CORVID_HTTP_ALLOW") || detail.contains("env"),
        "diagnostic must mention the CORVID_HTTP_ALLOW env override; got: {detail}"
    );
    assert!(
        detail.contains("fails closed") || detail.contains("security model"),
        "diagnostic must explain the fail-closed contract; got: {detail}"
    );
}

/// Slice 47h — policy rejections surface as Err VALUES. Unwrap a
/// `main() -> Result<_, String>` run result down to the Err
/// message so tests can assert on the diagnostic content.
fn expect_result_err_string(result: Value) -> String {
    match result {
        Value::ResultErr(inner) => match inner.get() {
            Value::String(s) => s.to_string(),
            other => panic!("expected Err(String); got Err({other:?})"),
        },
        other => panic!(
            "expected the program to return a ResultErr carrying the policy \
             diagnostic; got {other:?}"
        ),
    }
}
