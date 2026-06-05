//! Serve smoke for the reference apps — slice `35V2-P42-E-LR-app-deploy-smoke-ci`.
//!
//! This is the real "smoke-deploys in CI" gate: the deploy manifests run
//! `corvid serve <app>/src/main.cor`, so this test does exactly that for
//! each of the five reference apps — spawns the built `corvid` binary as
//! a server, waits for `/healthz`, GETs the app's `/schema` route, and
//! asserts a 200 with the app's manifest envelope. No Docker required;
//! it validates the same command the containers run, in the existing
//! `cargo test` CI job, cross-platform.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

/// Minimal HTTP/1.1 GET over a raw socket. Returns `(status, body)`.
/// Uses `Connection: close` so the whole response arrives before EOF.
fn http_get(port: u16, path: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status: u16 = raw
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, body))
}

/// Minimal HTTP/1.1 POST over a raw socket. Returns `(status, body)`.
fn http_post(port: u16, path: &str, json_body: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    let body_bytes = json_body.as_bytes();
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    )
    .ok()?;
    stream.write_all(body_bytes).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status: u16 = raw
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, body))
}

/// Poll `/healthz` until it answers 200 or the deadline passes.
fn wait_until_ready(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some((200, _)) = http_get(port, "/healthz") {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Kills the served process on drop so a failed assertion never leaks a
/// listener.
struct ServedApp(Child);
impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn reference_apps_serve_their_schema_route() {
    // (app dir, port, a substring the `/schema` JSON must contain)
    let apps = [
        ("personal_executive_agent", 8190u16, "personal_executive_agent"),
        ("personal_knowledge_agent", 8191, "personal_knowledge_agent"),
        ("finance_operations_agent", 8192, "finance_operations_agent"),
        ("customer_support_agent", 8193, "customer_support_agent"),
        ("code_maintenance_agent", 8194, "code_maintenance_agent"),
    ];

    for (app, port, needle) in apps {
        let main = repo_root()
            .join("examples")
            .join("backend")
            .join(app)
            .join("src")
            .join("main.cor");
        assert!(main.exists(), "{app}: missing {}", main.display());

        let child = Command::new(corvid_bin())
            .arg("serve")
            .arg(&main)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
            .current_dir(repo_root())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("{app}: spawn corvid serve: {e}"));
        let _guard = ServedApp(child);

        assert!(
            wait_until_ready(port),
            "{app}: server did not become ready on :{port}"
        );

        let (status, body) =
            http_get(port, "/schema").unwrap_or_else(|| panic!("{app}: GET /schema failed"));
        assert_eq!(status, 200, "{app}: GET /schema status (body={body})");
        assert!(
            body.contains(needle),
            "{app}: /schema body missing `{needle}`: {body}"
        );
        // Every app's schema manifest reports its migration table count.
        assert!(
            body.contains("table_count"),
            "{app}: /schema body missing `table_count`: {body}"
        );
    }
}

/// Slice `35V2-P42-E0-serve-5` end-to-end gate: a POST to an
/// approval-gated route MUST answer `202 Accepted` with an
/// `approval_id`, and the admin endpoints MUST report the pending
/// approval. This replaces the prior `E0-serve-4` `403 approval_required`
/// behavior with the async-approval model the ROADMAP slice spec
/// names.
///
/// Hermetic: spawns `corvid serve` on a minimal handcrafted source so
/// the test is not coupled to any specific reference app's body type.
/// Uses port `8195` so it can't collide with the 5-app smoke above
/// (ports `8190..=8194`).
#[test]
fn approval_gated_post_answers_202_and_admin_endpoint_lists_the_pending_id() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    // Approve label `SendMessage` normalizes (snake_case) to the tool
    // name `send_message` — verified by the checker at
    // `crates/corvid-types/src/checker/call.rs:127` (the rule documented
    // in 20m-A's `docs/internals/effect-spec/03-typing-rules.md` §6.1).
    let source = r#"type SendReq:
    body: String

type SendReceipt:
    delivered: Bool

effect send_external:
    cost: $0.0
    trust: human_required
    data: external

tool send_message(req: SendReq) -> SendReceipt dangerous uses send_external

agent execute_send(req: SendReq) -> SendReceipt uses send_external:
    approve SendMessage(req)
    return send_message(req)

server test_serve_5_api:
    route POST "/send" body SendReq -> json SendReceipt uses send_external:
        return execute_send(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8195;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server did not become ready on :{port}"
    );

    // 1. POST → 202 with approval_id.
    let (status, body) = http_post(
        port,
        "/send",
        r#"{"body":"hello reviewer, please decide"}"#,
    )
    .expect("POST /send failed");
    assert_eq!(
        status, 202,
        "POST /send must answer 202 (not 403, not 500). got status={status} body=`{body}`. \
         If this fails with 403, the QueueApprover wiring regressed back to ProgrammaticApprover::always_no(); \
         if 500, the approval-queue create() leg errored — read the response body for the queue error."
    );
    let resp: serde_json::Value =
        serde_json::from_str(&body).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id` string");
    assert!(
        !approval_id.is_empty(),
        "approval_id must be non-empty: body={body}"
    );
    assert_eq!(
        resp.get("status").and_then(|v| v.as_str()),
        Some("pending"),
        "202 body must report status=`pending`: {body}"
    );

    // 2. GET /__approvals → list contains the approval id.
    let (list_status, list_body) =
        http_get(port, "/__approvals").expect("GET /__approvals failed");
    assert_eq!(list_status, 200, "GET /__approvals must answer 200");
    assert!(
        list_body.contains(approval_id),
        "GET /__approvals must list the just-queued approval id `{approval_id}`: {list_body}"
    );

    // 3. GET /__approvals/<id> → returns the queued record.
    let (one_status, one_body) =
        http_get(port, &format!("/__approvals/{approval_id}")).expect("GET /__approvals/<id> failed");
    assert_eq!(one_status, 200, "GET /__approvals/<id> must answer 200");
    let one: serde_json::Value =
        serde_json::from_str(&one_body).expect("GET /__approvals/<id> body must be valid JSON");
    assert_eq!(one.get("id").and_then(|v| v.as_str()), Some(approval_id));
    assert_eq!(one.get("status").and_then(|v| v.as_str()), Some("pending"));
    assert_eq!(
        one.get("action").and_then(|v| v.as_str()),
        Some("SendMessage"),
        "GET /__approvals/<id> must report the approve label as `action`: {one_body}"
    );

    // 4. GET /__approvals/<unknown> → 404.
    let (missing_status, _) = http_get(port, "/__approvals/this-id-does-not-exist-anywhere")
        .expect("GET /__approvals/<missing> failed");
    assert_eq!(
        missing_status, 404,
        "GET /__approvals/<missing> must answer 404"
    );
}

/// Slice `serve-6` end-to-end gate: a reviewer POST-ing to
/// `/__approvals/:id/approve` MUST transition the queued approval,
/// re-execute the original agent, and return the agent's result;
/// a `POST /__approvals/:id/deny` MUST mark the approval denied
/// and drop the pending invocation without re-running anything.
/// Both endpoints MUST 404 on unknown ids and 409 on already-
/// decided ids.
///
/// Hermetic: same minimal handcrafted source as the
/// `E0-serve-5` test but with a more interesting receipt return
/// so the re-execution result is observable from the response.
/// Uses port `8196` so it can't collide with the 5-app smoke
/// (8190-8194) or the serve-5 test (8195).
#[test]
fn approval_transition_endpoints_approve_re_executes_and_deny_drops_pending() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type SendReq:
    body: String

type SendReceipt:
    echoed_body: String
    delivered: Bool

effect send_external:
    cost: $0.0
    trust: human_required
    data: external

tool send_message(req: SendReq) -> SendReceipt dangerous uses send_external

agent execute_send(req: SendReq) -> SendReceipt uses send_external:
    approve SendMessage(req)
    return SendReceipt(req.body, true)
"#;
    // NB: the `execute_send` body explicitly constructs the
    // receipt rather than going through `send_message` (which would
    // need a host tool registration). The dangerous-call type-
    // checker still requires the `approve` boundary, so the queued
    // state is exercised — only the post-approval branch differs
    // from the E0-serve-5 test, returning a deterministic receipt
    // the test can observe end-to-end.
    let source = format!(
        "{source}\nserver test_serve_6_api:\n    route POST \"/send\" body SendReq -> json SendReceipt uses send_external:\n        return execute_send(body)\n"
    );
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8196;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server did not become ready on :{port}"
    );

    // --- /approve path ---
    let (post_status, post_body) = http_post(
        port,
        "/send",
        r#"{"body":"hello, please approve"}"#,
    )
    .expect("POST /send failed");
    assert_eq!(post_status, 202, "POST /send should answer 202: {post_body}");
    let resp: serde_json::Value =
        serde_json::from_str(&post_body).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id`")
        .to_string();

    let (approve_status, approve_body) = http_post(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
    )
    .expect("POST /__approvals/<id>/approve failed");
    assert_eq!(
        approve_status, 200,
        "POST /__approvals/<id>/approve must answer 200 (transition succeeded + agent re-executed). got status={approve_status} body=`{approve_body}`. \
         If 404: the queue runtime forgot the approval id mid-test (cross-process leak?). If 409: the pending invocation wasn't captured (the dispatch-handler stash regressed). If 500: the re-execution itself errored — read the response body."
    );
    let approve_resp: serde_json::Value =
        serde_json::from_str(&approve_body).expect("/approve body must be valid JSON");
    assert_eq!(
        approve_resp.get("status").and_then(|v| v.as_str()),
        Some("approved")
    );
    let result = approve_resp
        .get("result")
        .expect("/approve body must carry a `result` from the re-executed agent");
    // The agent re-executes with the original args, so the receipt
    // body should reflect the original POST body.
    assert_eq!(
        result.get("echoed_body").and_then(|v| v.as_str()),
        Some("hello, please approve"),
        "re-executed agent's result must echo the original POST body. body={approve_body}"
    );
    assert_eq!(result.get("delivered").and_then(|v| v.as_bool()), Some(true));

    // GET on the same id after approval → status: approved.
    let (one_status, one_body) =
        http_get(port, &format!("/__approvals/{approval_id}")).expect("GET fetch failed");
    assert_eq!(one_status, 200);
    let one: serde_json::Value = serde_json::from_str(&one_body).unwrap();
    assert_eq!(one.get("status").and_then(|v| v.as_str()), Some("approved"));

    // A second /approve must 409 (already decided).
    let (replay_status, _) =
        http_post(port, &format!("/__approvals/{approval_id}/approve"), "")
            .expect("POST replay /approve failed");
    assert_eq!(
        replay_status, 409,
        "a second /approve on the same id must answer 409 (already decided)"
    );

    // --- /deny path ---
    let (post2_status, post2_body) = http_post(
        port,
        "/send",
        r#"{"body":"this one will be denied"}"#,
    )
    .expect("second POST /send failed");
    assert_eq!(post2_status, 202);
    let resp2: serde_json::Value = serde_json::from_str(&post2_body).unwrap();
    let approval_id_2 = resp2
        .get("approval_id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();
    assert_ne!(
        approval_id, approval_id_2,
        "second approval id must differ from the first (the QueueApprover's AtomicU64 sequence guarantees this)"
    );

    let (deny_status, deny_body) = http_post(
        port,
        &format!("/__approvals/{approval_id_2}/deny"),
        "",
    )
    .expect("POST /deny failed");
    assert_eq!(
        deny_status, 200,
        "POST /__approvals/<id>/deny must answer 200: {deny_body}"
    );
    let deny_resp: serde_json::Value = serde_json::from_str(&deny_body).unwrap();
    assert_eq!(
        deny_resp.get("status").and_then(|v| v.as_str()),
        Some("denied")
    );

    // GET → status: denied.
    let (denied_status, denied_body) =
        http_get(port, &format!("/__approvals/{approval_id_2}")).expect("GET denied fetch failed");
    assert_eq!(denied_status, 200);
    let denied: serde_json::Value = serde_json::from_str(&denied_body).unwrap();
    assert_eq!(
        denied.get("status").and_then(|v| v.as_str()),
        Some("denied")
    );

    // /approve and /deny on unknown ids → 404.
    let (missing_approve, _) =
        http_post(port, "/__approvals/this-id-does-not-exist/approve", "")
            .expect("POST /approve unknown failed");
    assert_eq!(missing_approve, 404);
    let (missing_deny, _) =
        http_post(port, "/__approvals/this-id-does-not-exist/deny", "")
            .expect("POST /deny unknown failed");
    assert_eq!(missing_deny, 404);
}

/// Build the tiny `serve_tools_fixture` cdylib at
/// `tests/fixtures/serve_tools_fixture/` and return the path to the
/// resulting platform-specific shared library. The fixture exports a
/// single `__corvid_tool_echo_string` symbol matching the
/// `CorvidToolFn` ABI — when `corvid serve --with-tools-cdylib`
/// dlopens it, the symbol is dlsym'd, registered via
/// `corvid_register_tool`, and bridged into the interpreter's
/// `ToolRegistry` so an in-app `echo_string(value)` call dispatches
/// through to the fixture's implementation.
fn build_serve_tools_fixture() -> PathBuf {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("serve_tools_fixture");
    let manifest = fixture_dir.join("Cargo.toml");
    assert!(
        manifest.exists(),
        "fixture manifest not found at {} — workspace layout regressed?",
        manifest.display()
    );

    let status = Command::new(env!("CARGO"))
        .args(["build", "--release", "--manifest-path"])
        .arg(&manifest)
        .status()
        .expect("spawn cargo build for serve_tools_fixture");
    assert!(
        status.success(),
        "cargo build of serve_tools_fixture cdylib failed (exit {:?})",
        status.code()
    );

    let target_dir = fixture_dir.join("target").join("release");
    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        vec![target_dir.join("serve_tools_fixture.dll")]
    } else if cfg!(target_os = "macos") {
        vec![target_dir.join("libserve_tools_fixture.dylib")]
    } else {
        vec![target_dir.join("libserve_tools_fixture.so")]
    };
    for path in &candidates {
        if path.exists() {
            return path.clone();
        }
    }
    panic!(
        "fixture cdylib not found at any expected path: {:?}",
        candidates
    );
}

/// 33Q1b end-to-end gate. The anonymous-2026-06-04 round-2 trial
/// report P1.1 said "adding the tool to `tools.py` does not help —
/// the interpreter serve path doesn't load it." This test pins the
/// fix: a Corvid app declares a `dangerous` tool, an approve-gated
/// agent, and a POST route. A `tools.py` sits next to the source
/// with an `@tool("echo_string")`-decorated async implementation.
/// `corvid serve` autodetects `tools.py`, embeds Python via PyO3,
/// imports the module (running the decorators), and bridges each
/// registered Python coroutine into the runtime's `ToolRegistry`.
///
/// PYTHONPATH is set to include `runtime/python/` so the user's
/// `from corvid_runtime import tool` import resolves to the local
/// `corvid_runtime` package without requiring a global pip install.
///
/// Port `8198` so it can't collide with the other serve tests
/// (8190-8197).
#[test]
fn serve_autoloads_tools_py_and_dispatches_approval_gated_tool_through_python() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();
    let src_dir = project_root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let src_path = src_dir.join("main.cor");

    // Corvid source: identical shape to the cdylib test but the
    // tool implementation will come from tools.py instead of the
    // fixture cdylib. The approve label `EchoString` snake_case-
    // normalizes to `echo_string` so the dangerous-call checker
    // accepts the approval.
    let source = r#"type EchoReq:
    value: String

type EchoReceipt:
    echoed: String

effect echo_external:
    cost: $0.0
    trust: human_required
    data: external

tool echo_string(value: String) -> String dangerous uses echo_external

agent execute_echo(req: EchoReq) -> EchoReceipt uses echo_external:
    approve EchoString(req.value)
    echoed = echo_string(req.value)
    return EchoReceipt(echoed)

server test_serve_q1b_api:
    route POST "/echo" body EchoReq -> json EchoReceipt uses echo_external:
        return execute_echo(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    // tools.py at project root — that's where the autoloader's
    // walk-up-one-level rule expects it (next to `src/`).
    let tools_py = r#"from corvid_runtime import tool


@tool("echo_string")
async def echo_string(value: str) -> str:
    return value
"#;
    std::fs::write(project_root.join("tools.py"), tools_py).unwrap();

    // PYTHONPATH points at the local corvid_runtime package so
    // `from corvid_runtime import tool` resolves without a pip
    // install. Repo root + `runtime/python/`.
    let python_path = repo_root().join("runtime").join("python");
    assert!(
        python_path.is_dir(),
        "runtime/python missing at {} — corvid_runtime package layout regressed?",
        python_path.display()
    );

    let port: u16 = 8198;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .env("PYTHONPATH", &python_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server did not become ready on :{port} \
         (probable cause: tools.py autoloader failed — re-run with stderr \
         inherited to see the import traceback; common failures: \
         `corvid_runtime` package not importable (PYTHONPATH wrong) or \
         tools.py raised at import time)"
    );

    // 1. POST /echo → 202 + approval_id (approve boundary queues).
    let probe_value = "hello from tools.py";
    let post_body = format!(r#"{{"value":"{probe_value}"}}"#);
    let (post_status, post_body_resp) =
        http_post(port, "/echo", &post_body).expect("POST /echo failed");
    assert_eq!(
        post_status, 202,
        "POST /echo must answer 202: {post_body_resp}"
    );
    let resp: serde_json::Value =
        serde_json::from_str(&post_body_resp).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id`")
        .to_string();

    // 2. POST /__approvals/<id>/approve → 200 + result.echoed == probe_value.
    //    If the autoloader didn't wire echo_string into the runtime,
    //    this answers 500 "no handler registered for tool `echo_string`"
    //    and reproduces P1.1's regression.
    let (approve_status, approve_body) = http_post(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
    )
    .expect("POST /__approvals/<id>/approve failed");
    assert_eq!(
        approve_status, 200,
        "POST /__approvals/<id>/approve must answer 200 — the tools.py \
         echo_string handler must run via PyO3 after approval. got \
         status={approve_status} body=`{approve_body}`. If 500 with `no \
         handler registered for tool`: the tools.py autoloader regressed \
         (file not found, Python import failed, _TOOL_IMPLS read failed, \
         or the GIL-acquiring bridge broke)."
    );
    let approve_resp: serde_json::Value =
        serde_json::from_str(&approve_body).expect("/approve body must be valid JSON");
    let result = approve_resp
        .get("result")
        .expect("/approve body must carry a `result` envelope");
    assert_eq!(
        result.get("echoed").and_then(|v| v.as_str()),
        Some(probe_value),
        "re-executed agent's `echoed` field must equal the original POST \
         `value`, proving the tools.py @tool(\"echo_string\") implementation \
         was actually invoked via PyO3 + asyncio.run end-to-end. body={approve_body}"
    );
}

/// 33Q6 end-to-end gate. Maintainer-as-reviewer-2026-06-05 P1.1
/// caught that the 33Q1b tools.py autoloader failed on every
/// release-installed reviewer's first attempt because
/// `corvid_runtime` is NOT on PyPI (the scaffold's "Next steps"
/// directive `pip install corvid-runtime` was broken). 33Q6 ships
/// the `corvid_runtime` package alongside the binary in the
/// release tarball under `<binary_parent>/../runtime-py/` and
/// teaches `install_python_tools` to auto-detect it before
/// importing tools.py.
///
/// This test pins the autodetection: spawns `corvid serve` with
/// **NO PYTHONPATH set** and asserts the same end-to-end
/// scaffold + tools.py + approval-gated round-trip the prior 33Q1b
/// test exercised, but without the operator-facing PYTHONPATH
/// override. The dev-layout branch of `find_bundled_corvid_runtime`
/// resolves `target/debug/corvid.exe -> ../../runtime/python`
/// during `cargo test`, so we get the autodetect-via-workspace
/// path covered.
///
/// Port `8200` so it can't collide with the other serve tests
/// (8190-8199).
#[test]
fn serve_autoloads_tools_py_via_bundled_corvid_runtime_without_pythonpath() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();
    let src_dir = project_root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let src_path = src_dir.join("main.cor");

    // Same shape as the 33Q1b test — a `dangerous` tool that
    // tools.py implements, an approve-gated agent calling it, a
    // POST route bound to the agent. We need the round-trip to
    // observe the actual tool implementation running.
    let source = r#"type EchoReq:
    value: String

type EchoReceipt:
    echoed: String

effect echo_external:
    cost: $0.0
    trust: human_required
    data: external

tool echo_string(value: String) -> String dangerous uses echo_external

agent execute_echo(req: EchoReq) -> EchoReceipt uses echo_external:
    approve EchoString(req.value)
    echoed = echo_string(req.value)
    return EchoReceipt(echoed)

server test_serve_q6_api:
    route POST "/echo" body EchoReq -> json EchoReceipt uses echo_external:
        return execute_echo(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let tools_py = r#"from corvid_runtime import tool


@tool("echo_string")
async def echo_string(value: str) -> str:
    return value
"#;
    std::fs::write(project_root.join("tools.py"), tools_py).unwrap();

    let port: u16 = 8200;
    // NO .env("PYTHONPATH", ...) — that's the load-bearing
    // assertion 33Q6 makes: bundled corvid_runtime resolves
    // automatically.
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server did not become ready on :{port} — probable cause: \
         33Q6 bundled-corvid_runtime autodetection regressed. The CLI \
         should resolve `find_bundled_corvid_runtime` to the dev-layout \
         workspace path during `cargo test` (`target/debug/corvid` -> \
         `../../runtime/python`). If this fails, re-check the path math \
         in `crates/corvid-runtime/src/python_tools.rs`."
    );

    // Round-trip: POST → 202, /approve → 200 with echoed value
    // matches the original POST body. Proves the bundled
    // corvid_runtime was importable AND tools.py registered the
    // handler AND the handler was reachable end-to-end.
    let probe_value = "hello via bundled corvid_runtime";
    let post_body = format!(r#"{{"value":"{probe_value}"}}"#);
    let (post_status, post_body_resp) =
        http_post(port, "/echo", &post_body).expect("POST /echo failed");
    assert_eq!(post_status, 202, "POST /echo must answer 202: {post_body_resp}");
    let resp: serde_json::Value =
        serde_json::from_str(&post_body_resp).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id`")
        .to_string();

    let (approve_status, approve_body) = http_post(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
    )
    .expect("POST /approve failed");
    assert_eq!(
        approve_status, 200,
        "POST /approve must answer 200 — bundled corvid_runtime must be \
         importable so tools.py registers echo_string: {approve_body}"
    );
    let approve_resp: serde_json::Value =
        serde_json::from_str(&approve_body).expect("/approve body must be valid JSON");
    let result = approve_resp
        .get("result")
        .expect("/approve body must carry a `result`");
    assert_eq!(
        result.get("echoed").and_then(|v| v.as_str()),
        Some(probe_value),
        "the bundled corvid_runtime tools.py autoload must dispatch the \
         user's coroutine end-to-end. body={approve_body}"
    );
}

/// 33Q1a end-to-end gate. The anonymous-2026-06-04 round-2 trial
/// report P1.1 documented that `corvid serve` had no tool-handler
/// registration mechanism: an approval-gated POST whose handler
/// reached a tool call returned 500 `no handler registered for tool
/// <name>` on `/approve`, AND consumed the approval anyway.
///
/// This test pins the fix end-to-end: a Corvid app declares a
/// `dangerous` tool, an agent that calls it after an `approve`
/// boundary, and a POST route bound to the agent. The fixture
/// cdylib at `tests/fixtures/serve_tools_fixture/` implements the
/// tool. `corvid serve --with-tools-cdylib <fixture>` dlopens the
/// cdylib, registers `__corvid_tool_echo_string` into the runtime's
/// C-ABI registry, and bridges it through `dispatch_host_tool` into
/// the interpreter's `ToolRegistry`. The full round-trip:
///
/// 1. POST `/echo` body `{"value":"<msg>"}` → 202 + `approval_id`.
/// 2. POST `/__approvals/<id>/approve` → 200 + body whose
///    `result.echoed` field equals the original `<msg>`, proving
///    the cdylib's tool was actually invoked.
///
/// If P1.1's "no handler registered" regression returns, the
/// `/approve` POST in step 2 answers 500 (not 200) and this test
/// fails loudly.
///
/// Port `8197` so it can't collide with the other serve tests
/// (8190-8196).
#[test]
fn serve_with_tools_cdylib_dispatches_approval_gated_tool_through_fixture() {
    let cdylib_path = build_serve_tools_fixture();

    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    // The agent runs `approve EchoCall(value)` then `return echo_string(value)`.
    // The `approve` label `EchoCall` snake_case-normalizes to `echo_call`,
    // which is the action name we'll see on the queued approval. The tool
    // declaration matches the fixture's `__corvid_tool_echo_string`.
    //
    // Note the receipt type `EchoReceipt` wraps the tool's plain `String`
    // return so the route's `-> json EchoReceipt` shape is observable in
    // the test as `result.echoed` — easier to assert than parsing a bare
    // JSON string from the response body.
    // The approve label MUST snake_case-normalize to the tool name
    // for the dangerous-call typechecker to accept the approval — so
    // `EchoString` (matches `echo_string`), not `EchoCall`. The
    // label's args must also match the tool's params shape — here
    // `value: String`, supplied as `req.value`.
    let source = r#"type EchoReq:
    value: String

type EchoReceipt:
    echoed: String

effect echo_external:
    cost: $0.0
    trust: human_required
    data: external

tool echo_string(value: String) -> String dangerous uses echo_external

agent execute_echo(req: EchoReq) -> EchoReceipt uses echo_external:
    approve EchoString(req.value)
    echoed = echo_string(req.value)
    return EchoReceipt(echoed)

server test_serve_q1a_api:
    route POST "/echo" body EchoReq -> json EchoReceipt uses echo_external:
        return execute_echo(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8197;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--with-tools-cdylib")
        .arg(&cdylib_path)
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server did not become ready on :{port} \
         (probable cause: --with-tools-cdylib loader rejected the fixture; \
         re-run with stderr inherited to see the startup log)"
    );

    // 1. POST /echo → 202 + approval_id.
    let probe_value = "hello from the friends-and-family round";
    let post_body = format!(r#"{{"value":"{probe_value}"}}"#);
    let (post_status, post_body_resp) =
        http_post(port, "/echo", &post_body).expect("POST /echo failed");
    assert_eq!(
        post_status, 202,
        "POST /echo must answer 202 (the approve boundary queues the call): {post_body_resp}"
    );
    let resp: serde_json::Value =
        serde_json::from_str(&post_body_resp).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id`")
        .to_string();

    // 2. POST /__approvals/<id>/approve → 200 + result.echoed == probe_value.
    //    THIS is the load-bearing assertion: without --with-tools-cdylib
    //    wiring, /approve would answer 500 "no handler registered for
    //    tool `echo_string`" AND drop the approval — the P1.1 regression
    //    documented in `docs/external-trials/33m-trial-anonymous-2026-06-04.md`.
    let (approve_status, approve_body) = http_post(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
    )
    .expect("POST /__approvals/<id>/approve failed");
    assert_eq!(
        approve_status, 200,
        "POST /__approvals/<id>/approve must answer 200 — the cdylib's \
         echo_string handler must run after approval. got status={approve_status} \
         body=`{approve_body}`. If 500 with `no handler registered for tool`: \
         the --with-tools-cdylib wiring regressed (cdylib path not loaded, \
         symbol name wrong, or dispatch_host_tool bridge broken). If 500 \
         with other text: read the body — the fixture itself may have \
         errored."
    );
    let approve_resp: serde_json::Value =
        serde_json::from_str(&approve_body).expect("/approve body must be valid JSON");
    assert_eq!(
        approve_resp.get("status").and_then(|v| v.as_str()),
        Some("approved"),
        "/approve body status field must be `approved`: {approve_body}"
    );
    let result = approve_resp
        .get("result")
        .expect("/approve body must carry a `result` envelope from the re-executed agent");
    assert_eq!(
        result.get("echoed").and_then(|v| v.as_str()),
        Some(probe_value),
        "re-executed agent's `echoed` field must equal the original POST `value`, \
         proving the fixture cdylib's __corvid_tool_echo_string was actually invoked \
         end-to-end. body={approve_body}"
    );
}

/// 33Q2 end-to-end gate. The anonymous-2026-06-04 round-2 trial
/// report P1.2 documented that a handler error under
/// `/__approvals/<id>/approve` consumed the approval anyway — a
/// transient handler failure permanently burned a human approval,
/// an approval-budget-integrity bug, not just UX.
///
/// This test pins the leave-pending fix: a Corvid app declares a
/// `dangerous` tool the cdylib does NOT implement (so the runtime
/// produces `UnknownTool` when reached after approval), an
/// approve-gated agent that calls it, and a POST route. The flow:
///
/// 1. POST `/broken` → 202 + approval_id.
/// 2. POST `/__approvals/<id>/approve` → 500. The 500 body MUST
///    carry `approval_status: "pending"` + `retry.possible: true` +
///    the `detail` field naming the runtime error. The approval
///    MUST NOT transition to `approved`.
/// 3. GET `/__approvals/<id>` → 200 with `status: "pending"` +
///    `last_handler_error` populated + `retry_possible: true`. The
///    reviewer can see WHY their grant didn't take effect.
/// 4. POST `/__approvals/<id>/approve` AGAIN → still 500, still
///    pending. The reviewer's authorization is preserved across
///    retry attempts.
/// 5. POST `/__approvals/<id>/deny` → 200, terminates the pending
///    invocation. After deny, `/approve` answers 409 (already
///    decided) — the reviewer's safety valve to exit a permanently-
///    broken loop.
///
/// Adversarial: across multiple /approve retries against a
/// permanently-broken handler, the approval state never transitions
/// to `approved`. A handler-error cannot expose any approval-bypass
/// primitive (the `ProgrammaticApprover::always_yes` bypass runtime
/// is local to each approve_approval call and never escapes).
///
/// Port `8199` so it can't collide with the other serve tests
/// (8190-8198).
#[test]
fn serve_approval_is_preserved_when_handler_errors_and_terminates_only_on_deny() {
    // We re-use the cdylib fixture (echo_string) — but the Corvid
    // app declares a SECOND tool `permanently_broken_tool` that the
    // fixture does NOT implement. The 33Q1a loader logs it as
    // "declared in app but missing from cdylib" and the runtime
    // hits `UnknownTool` when the agent reaches that tool call after
    // approval, which is the controlled handler-error we need.
    let cdylib_path = build_serve_tools_fixture();

    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type BrokenReq:
    value: String

type BrokenReceipt:
    result: String

effect broken_external:
    cost: $0.0
    trust: human_required
    data: external

tool permanently_broken_tool(value: String) -> String dangerous uses broken_external

agent execute_broken(req: BrokenReq) -> BrokenReceipt uses broken_external:
    approve PermanentlyBrokenTool(req.value)
    out = permanently_broken_tool(req.value)
    return BrokenReceipt(out)

server test_serve_q2_api:
    route POST "/broken" body BrokenReq -> json BrokenReceipt uses broken_external:
        return execute_broken(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8199;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--with-tools-cdylib")
        .arg(&cdylib_path)
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(wait_until_ready(port), "server did not become ready on :{port}");

    // 1. POST /broken → 202 + approval_id.
    let probe_value = "this approval will be retried then denied";
    let post_body = format!(r#"{{"value":"{probe_value}"}}"#);
    let (post_status, post_body_resp) =
        http_post(port, "/broken", &post_body).expect("POST /broken failed");
    assert_eq!(post_status, 202, "POST /broken must answer 202: {post_body_resp}");
    let resp: serde_json::Value =
        serde_json::from_str(&post_body_resp).expect("202 body must be valid JSON");
    let approval_id = resp
        .get("approval_id")
        .and_then(|v| v.as_str())
        .expect("202 body must carry an `approval_id`")
        .to_string();

    // 2. POST /approve → 500 with approval_status: pending, retry.possible: true.
    //    Pre-33Q2 this answered 500 too, but the approval got silently
    //    consumed AND the pending invocation removed — leaving the
    //    reviewer with no way to recover.
    let approve_url = format!("/__approvals/{approval_id}/approve");
    let (approve_status, approve_body) =
        http_post(port, &approve_url, "").expect("POST /approve failed");
    assert_eq!(
        approve_status, 500,
        "POST /approve must answer 500 when the handler errors: {approve_body}"
    );
    let approve_resp: serde_json::Value =
        serde_json::from_str(&approve_body).expect("/approve body must be valid JSON");
    assert_eq!(
        approve_resp.get("error").and_then(|v| v.as_str()),
        Some("approved_execution_failed"),
        "500 body must use the `approved_execution_failed` error code: {approve_body}"
    );
    assert_eq!(
        approve_resp.get("approval_status").and_then(|v| v.as_str()),
        Some("pending"),
        "500 body MUST carry `approval_status: pending` so the reviewer \
         knows the approval was NOT consumed — the load-bearing assertion \
         for 33Q2's leave-pending behaviour: {approve_body}"
    );
    let retry = approve_resp
        .get("retry")
        .expect("500 body must carry a `retry` envelope describing the retry path");
    assert_eq!(
        retry.get("possible").and_then(|v| v.as_bool()),
        Some(true),
        "retry.possible MUST be true so the reviewer's client knows to \
         try /approve again or /deny: {approve_body}"
    );

    // 3. GET /__approvals/<id> → status: pending + last_handler_error captured.
    let get_url = format!("/__approvals/{approval_id}");
    let (get_status, get_body) =
        http_get(port, &get_url).expect("GET /__approvals/<id> failed");
    assert_eq!(get_status, 200, "GET /__approvals/<id> must answer 200");
    let one: serde_json::Value =
        serde_json::from_str(&get_body).expect("GET body must be valid JSON");
    assert_eq!(
        one.get("status").and_then(|v| v.as_str()),
        Some("pending"),
        "GET /__approvals/<id> must report status=pending after a failed \
         /approve — proves the approval was not consumed: {get_body}"
    );
    let last_err = one
        .get("last_handler_error")
        .and_then(|v| v.as_str())
        .expect("GET /__approvals/<id> must include `last_handler_error` after a failed /approve");
    assert!(
        !last_err.is_empty(),
        "last_handler_error must not be empty — operator needs the failure \
         signal to decide whether to retry or deny: {get_body}"
    );
    assert_eq!(
        one.get("retry_possible").and_then(|v| v.as_bool()),
        Some(true),
        "GET must report retry_possible=true so the reviewer's client can \
         render the retry option: {get_body}"
    );

    // 4. POST /approve AGAIN → still 500, still pending (adversarial:
    //    no number of retries against a permanently-broken handler
    //    can flip the approval state to `approved`).
    let (approve2_status, approve2_body) =
        http_post(port, &approve_url, "").expect("POST /approve retry failed");
    assert_eq!(
        approve2_status, 500,
        "second /approve against the still-broken handler must also \
         answer 500: {approve2_body}"
    );
    let approve2_resp: serde_json::Value =
        serde_json::from_str(&approve2_body).expect("retry body must be valid JSON");
    assert_eq!(
        approve2_resp.get("approval_status").and_then(|v| v.as_str()),
        Some("pending"),
        "second /approve attempt must STILL report `pending` — handler \
         errors cannot expose any path that transitions the approval to \
         `approved` without a successful handler invocation: {approve2_body}"
    );

    // GET again — still pending, last_handler_error STILL populated
    // (and refreshed to whatever the second attempt produced).
    let (get2_status, get2_body) =
        http_get(port, &get_url).expect("second GET /__approvals/<id> failed");
    assert_eq!(get2_status, 200);
    let two: serde_json::Value = serde_json::from_str(&get2_body).unwrap();
    assert_eq!(
        two.get("status").and_then(|v| v.as_str()),
        Some("pending"),
        "after the second /approve attempt the approval STILL stays \
         pending: {get2_body}"
    );

    // 5. POST /deny → 200, approval terminates as denied. This is the
    //    reviewer's safety valve to exit a permanently-broken loop.
    let deny_url = format!("/__approvals/{approval_id}/deny");
    let (deny_status, deny_body) = http_post(port, &deny_url, "").expect("POST /deny failed");
    assert_eq!(deny_status, 200, "POST /deny must answer 200: {deny_body}");
    let deny_resp: serde_json::Value = serde_json::from_str(&deny_body).unwrap();
    assert_eq!(
        deny_resp.get("status").and_then(|v| v.as_str()),
        Some("denied"),
        "/deny must transition to denied: {deny_body}"
    );

    // After deny, /approve answers 409 (already decided) — the
    // approval can no longer be retried because the reviewer
    // explicitly terminated it.
    let (approve3_status, _) =
        http_post(port, &approve_url, "").expect("post-deny /approve failed");
    assert_eq!(
        approve3_status, 409,
        "/approve after /deny must answer 409 — the reviewer's explicit \
         deny is terminal"
    );
}

/// 33Q9 acceptance — maintainer-as-reviewer-2026-06-05 P2.1.
/// `corvid serve` pre-33Q9 labeled every `Dispatch::Body` route as
/// `approval-gated -> 202 + queued` regardless of whether the agent
/// actually had an `approve` boundary. A reviewer planning client-
/// side polling logic against the label would write the wrong code
/// for half of their routes. This test pins the corrected label-
/// per-route-shape map by writing a Corvid source with two POST
/// routes — one whose agent approves, one whose agent doesn't —
/// and asserting the startup banner shows distinct labels for each.
///
/// The test uses pipe-captured stderr (`Stdio::piped()`) and reads
/// it after the server becomes ready. No HTTP requests needed —
/// the assertion is purely on the printed banner.
///
/// Port `8201` so it can't collide with the other serve tests
/// (8190-8200).
#[test]
fn serve_startup_banner_distinguishes_routes_with_and_without_approve() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    // Two POST routes:
    // - /reset:   agent execute_reset HAS `approve EchoString(...)` — label expected: "approval-gated -> 202 + queued"
    // - /classify: agent execute_classify has NO `approve` — label expected: just "(body)"
    let source = r#"type EchoReq:
    value: String

type EchoReceipt:
    echoed: String

effect echo_external:
    cost: $0.0
    trust: human_required
    data: external

tool echo_string(value: String) -> String dangerous uses echo_external

agent execute_reset(req: EchoReq) -> EchoReceipt uses echo_external:
    approve EchoString(req.value)
    echoed = echo_string(req.value)
    return EchoReceipt(echoed)

agent execute_classify(req: EchoReq) -> EchoReceipt:
    return EchoReceipt(req.value)

server test_serve_q9_api:
    route POST "/reset" body EchoReq -> json EchoReceipt uses echo_external:
        return execute_reset(body)
    route POST "/classify" body EchoReq -> json EchoReceipt:
        return execute_classify(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8201;
    let mut child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));

    // Wait for server to be ready, then drain stdout. We use a
    // separate var for the child so the ServedApp guard can take
    // ownership after we've captured the banner output.
    if !wait_until_ready(port) {
        let _ = child.kill();
        let _ = child.wait();
        panic!("server did not become ready on :{port}");
    }

    // The banner is written to stdout (println! goes there). Take
    // the stdout pipe and read until the marker line "GET    /__approvals"
    // shows up — that's the last banner line printed before serve
    // enters its main loop.
    use std::io::{BufRead, BufReader};
    let stdout = child.stdout.take().expect("child stdout taken");
    let mut banner_lines: Vec<String> = Vec::new();
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = line.expect("read stdout line");
        let done = line.contains("/deny");
        banner_lines.push(line);
        if done {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();

    let banner = banner_lines.join("\n");

    // Assertion 1: /reset (agent with `approve`) gets the
    // approval-gated label.
    assert!(
        banner.contains("POST   /reset")
            && banner
                .lines()
                .any(|l| l.contains("/reset")
                    && l.contains("approval-gated -> 202 + queued")),
        "POST /reset MUST be labeled `approval-gated -> 202 + queued` \
         (its agent execute_reset has an `approve` boundary). \
         banner=\n{banner}"
    );

    // Assertion 2: /classify (agent WITHOUT `approve`) does NOT
    // get the approval-gated label. THIS is the load-bearing 33Q9
    // assertion — pre-33Q9 it was unconditionally labeled
    // approval-gated even though execute_classify never queues.
    assert!(
        banner.contains("POST   /classify")
            && banner
                .lines()
                .any(|l| l.contains("/classify")
                    && !l.contains("approval-gated")
                    && l.contains("(body)")),
        "POST /classify MUST be labeled `(body)` WITHOUT \
         `approval-gated` (its agent execute_classify has NO approve \
         boundary). Pre-33Q9 every body-dispatch route was \
         unconditionally labeled approval-gated — that's the \
         regression the maintainer trial caught. banner=\n{banner}"
    );
}
