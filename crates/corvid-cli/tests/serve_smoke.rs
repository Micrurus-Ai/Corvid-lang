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

/// Minimal HTTP/1.1 GET carrying a `Cookie` header. Returns `(status,
/// body)` — for exercising session-authenticated routes.
fn http_get_with_cookie(port: u16, path: &str, cookie: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
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

/// Minimal HTTP/1.1 POST with extra request headers (Cookie /
/// X-CSRF-Token) — for authenticated approval transitions (52f-4b).
fn http_post_with_headers(
    port: u16,
    path: &str,
    json_body: &str,
    headers: &[(String, String)],
) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let body_bytes = json_body.as_bytes();
    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body_bytes.len()
    );
    for (name, value) in headers {
        req.push_str(&format!("{name}: {value}\r\n"));
    }
    req.push_str("\r\n");
    stream.write_all(req.as_bytes()).ok()?;
    stream.write_all(body_bytes).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status: u16 = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, body))
}

/// A fixed CSRF HMAC secret for reviewer tests — set as
/// `CORVID_CSRF_SECRET` on the served process so a pre-computed CSRF
/// token verifies (52f-4b).
const TEST_CSRF_SECRET: &str = "corvid-test-csrf-secret-0123456789";

/// An identity block declaring a `reviewer` role that grants
/// `approvals.decide`, prepended to sources that exercise approval
/// transitions. Its tenant matches the approval queue's default
/// (`serve-default`), so a reviewer in that tenant may decide.
const IDENTITY_WITH_REVIEWER: &str = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("serve-default")
    roles:
        reviewer: "approvals.decide"

"#;

/// Add the reviewer serve env vars to a `Command` under construction and
/// return it, so an existing `.arg(...)` chain can be wrapped.
fn with_reviewer_env<'a>(cmd: &'a mut Command, data_dir: &std::path::Path) -> &'a mut Command {
    cmd.envs(reviewer_serve_env(data_dir))
}

/// Env vars a reviewer-authenticated serve needs: a durable data dir (so
/// the seeded session store is the one serve opens), a fixed CSRF secret,
/// and OAuth credentials for the identity block's provider.
fn reviewer_serve_env(data_dir: &std::path::Path) -> Vec<(String, String)> {
    vec![
        ("CORVID_SERVE_DATA_DIR".into(), data_dir.display().to_string()),
        ("CORVID_CSRF_SECRET".into(), TEST_CSRF_SECRET.into()),
        ("CORVID_OAUTH_GOOGLE_CLIENT_ID".into(), "test-client-id".into()),
        ("CORVID_OAUTH_GOOGLE_CLIENT_SECRET".into(), "test-client-secret".into()),
    ]
}

/// Seed a reviewer actor + session (tenant `serve-default`, holding the
/// `reviewer` role) into the durable auth store BEFORE the server opens
/// it — exactly the state a real login produces. Returns the request
/// headers (`Cookie` + `X-CSRF-Token`) a reviewer presents on a decision.
fn seed_reviewer_headers(data_dir: &std::path::Path) -> Vec<(String, String)> {
    use corvid_runtime::{mint_csrf_token, AuthActor, SessionAuthRuntime, SessionCreate};
    std::fs::create_dir_all(data_dir).unwrap();
    let auth = SessionAuthRuntime::open(data_dir.join("auth.sqlite")).unwrap();
    auth.upsert_actor(AuthActor {
        id: "reviewer-1".into(),
        tenant_id: "serve-default".into(),
        display_name: "Reviewer".into(),
        actor_kind: "user".into(),
        auth_method: "oauth".into(),
        assurance_level: "aal1".into(),
        role_fingerprint: String::new(),
        permission_fingerprint: String::new(),
        created_ms: 0,
        updated_ms: 0,
    })
    .unwrap();
    auth.grant_actor_role("reviewer-1", "reviewer", 1).unwrap();
    let binding = "csrf-reviewer";
    auth.create_session(SessionCreate {
        id: "sess-reviewer".into(),
        actor_id: "reviewer-1".into(),
        tenant_id: "serve-default".into(),
        raw_token: "reviewer-token".into(),
        issued_ms: 1,
        expires_ms: 9_000_000_000_000,
        csrf_binding_id: binding.into(),
    })
    .unwrap();
    let csrf = mint_csrf_token(binding, TEST_CSRF_SECRET.as_bytes()).unwrap();
    vec![
        (
            "Cookie".to_string(),
            format!("corvid_session=reviewer-token; corvid_csrf={csrf}"),
        ),
        ("X-CSRF-Token".to_string(), csrf),
    ]
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
        "{IDENTITY_WITH_REVIEWER}{source}\nserver test_serve_6_api:\n    route POST \"/send\" body SendReq -> json SendReceipt uses send_external:\n        return execute_send(body)\n"
    );
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8196;
    // 52f-4b: approvals are now decided by a VERIFIED reviewer. Seed a
    // reviewer session (holding `approvals.decide`) into the durable
    // store the server opens.
    let data_dir = dir.path().join("state");
    let reviewer = seed_reviewer_headers(&data_dir);
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .envs(reviewer_serve_env(&data_dir))
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

    let (approve_status, approve_body) = http_post_with_headers(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
        &reviewer,
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

    let (deny_status, deny_body) = http_post_with_headers(
        port,
        &format!("/__approvals/{approval_id_2}/deny"),
        "",
        &reviewer,
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
    std::fs::write(&src_path, format!("{IDENTITY_WITH_REVIEWER}{source}")).unwrap();

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
    let data_dir = dir.path().join("state");
    let reviewer = seed_reviewer_headers(&data_dir);
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .env("PYTHONPATH", &python_path)
        .envs(reviewer_serve_env(&data_dir))
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
    let (approve_status, approve_body) = http_post_with_headers(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
        &reviewer,
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
    std::fs::write(&src_path, format!("{IDENTITY_WITH_REVIEWER}{source}")).unwrap();

    let tools_py = r#"from corvid_runtime import tool


@tool("echo_string")
async def echo_string(value: str) -> str:
    return value
"#;
    std::fs::write(project_root.join("tools.py"), tools_py).unwrap();

    let port: u16 = 8200;
    let data_dir = dir.path().join("state");
    let reviewer = seed_reviewer_headers(&data_dir);
    // NO .env("PYTHONPATH", ...) — that's the load-bearing
    // assertion 33Q6 makes: bundled corvid_runtime resolves
    // automatically. The reviewer env vars do not affect that.
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(repo_root())
        .envs(reviewer_serve_env(&data_dir))
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

    let (approve_status, approve_body) = http_post_with_headers(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
        &reviewer,
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
    std::fs::write(&src_path, format!("{IDENTITY_WITH_REVIEWER}{source}")).unwrap();

    let port: u16 = 8197;
    let data_dir = dir.path().join("state");
    let reviewer = seed_reviewer_headers(&data_dir);
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .arg("--with-tools-cdylib")
        .arg(&cdylib_path)
        .current_dir(repo_root())
        .envs(reviewer_serve_env(&data_dir))
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
    let (approve_status, approve_body) = http_post_with_headers(
        port,
        &format!("/__approvals/{approval_id}/approve"),
        "",
        &reviewer,
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
    std::fs::write(&src_path, format!("{IDENTITY_WITH_REVIEWER}{source}")).unwrap();

    let port: u16 = 8199;
    let data_dir = dir.path().join("state");
    let reviewer = seed_reviewer_headers(&data_dir);
    let child = with_reviewer_env(
        Command::new(corvid_bin())
            .arg("serve")
            .arg(&src_path)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
            .arg("--with-tools-cdylib")
            .arg(&cdylib_path),
        &data_dir,
    )
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
        http_post_with_headers(port, &approve_url, "", &reviewer).expect("POST /approve failed");
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
        http_post_with_headers(port, &approve_url, "", &reviewer).expect("POST /approve retry failed");
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
    let (deny_status, deny_body) = http_post_with_headers(port, &deny_url, "", &reviewer).expect("POST /deny failed");
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
    //
    // Slice 52a removed the dispatch-shape classifier (every route now
    // executes through the same synthetic-handler-agent path), so the
    // banner no longer prints a `(body)`/`(literal)` shape label — only
    // the approval tag distinguishes routes. The 33Q9 property is
    // preserved: a non-approve route carries NO approval-gated tag.
    assert!(
        banner.contains("POST   /classify")
            && banner
                .lines()
                .any(|l| l.contains("/classify") && !l.contains("approval-gated")),
        "POST /classify MUST NOT be labeled `approval-gated` (its agent \
         execute_classify has NO approve boundary). Pre-33Q9 every \
         body-dispatch route was unconditionally labeled approval-gated \
         — that's the regression the maintainer trial caught. banner=\n{banner}"
    );
}

/// 33Q10 acceptance — maintainer-as-reviewer-2026-06-05 P2.2.
/// Pre-33Q10, `corvid serve` 500 response bodies leaked IR byte-span
/// ranges (`[1227..1269]`) into the `detail` field via
/// `InterpError`'s Display impl. Internal compiler artifacts in a
/// client-facing surface — clients can't act on a byte-span in
/// source they don't have. The fix added
/// `RunError::user_facing_detail()` that strips the span prefix
/// before serialization.
///
/// This test pins the fix: it deliberately POSTs to a route whose
/// agent calls a tool with NO registered handler (the natural
/// 500-producing path during incremental development) and asserts
/// the resulting body's `detail` field contains the human-readable
/// message ("no handler registered for tool ...") WITHOUT any
/// `[<digits>..<digits>]` span-range prefix.
///
/// Port `8202` so it can't collide with the other serve tests
/// (8190-8201).
#[test]
fn serve_500_response_strips_ir_byte_span_prefix_from_detail() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    // Agent calls `classify_anything` which is declared but has no
    // registered handler. POST will produce 500 + "no handler
    // registered" — the natural site where the IR-span prefix used
    // to leak.
    let source = r#"type ClassifyReq:
    raw: String

type ClassifyVerdict:
    label: String

effect classify_effect:
    cost: $0.01
    trust: autonomous
    data: external

tool classify_anything(raw: String) -> ClassifyVerdict uses classify_effect

agent run_classify(req: ClassifyReq) -> ClassifyVerdict uses classify_effect:
    verdict = classify_anything(req.raw)
    return verdict

server test_serve_q10_api:
    route POST "/classify" body ClassifyReq -> json ClassifyVerdict uses classify_effect:
        return run_classify(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8202;
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

    assert!(wait_until_ready(port), "server did not become ready on :{port}");

    let (status, body) = http_post(port, "/classify", r#"{"raw":"hello"}"#)
        .expect("POST /classify failed");
    assert_eq!(
        status, 500,
        "POST /classify must answer 500 (no handler for the tool): {body}"
    );

    let resp: serde_json::Value =
        serde_json::from_str(&body).expect("500 body must be valid JSON");
    assert_eq!(
        resp.get("error").and_then(|v| v.as_str()),
        Some("handler_failed"),
        "500 body must carry error=`handler_failed`: {body}"
    );

    let detail = resp
        .get("detail")
        .and_then(|v| v.as_str())
        .expect("500 body must carry a `detail` string");

    // The load-bearing 33Q10 assertion: the detail MUST NOT carry
    // an `[<n>..<n>]` IR byte-span prefix. Pre-33Q10 it looked like
    // `[1227..1269] no handler registered for tool ...` — the
    // bracketed range is an internal compiler artifact and must
    // never leak to HTTP clients.
    let leaks_span = detail.starts_with('[')
        && detail
            .chars()
            .skip(1)
            .take_while(|c| *c != ']')
            .any(|c| c == '.');
    assert!(
        !leaks_span,
        "500 body's `detail` MUST NOT start with an IR byte-span \
         prefix like `[1227..1269]`. Internal compiler artifacts \
         leaking to HTTP clients was the maintainer trial's P2.2 \
         finding. detail={detail:?}"
    );

    // Sanity: the detail still carries the actionable message
    // ("no handler registered") so we haven't just nuked all
    // diagnostic content along with the span prefix.
    assert!(
        detail.contains("no handler registered"),
        "detail must still contain the actionable message after \
         span-stripping: detail={detail:?}"
    );
    assert!(
        detail.contains("classify_anything"),
        "detail must still name the unregistered tool: detail={detail:?}"
    );
}

/// 52f authorization-enforcement gate. As of slice 52f the interpreter
/// tier implements every capability the Application Contract can
/// advertise — route execution, streaming, uploads, pagination, AND
/// authorization enforcement — so a `requires authenticated` route no
/// longer refuses to start: it STARTS and enforces. This is the last
/// Contract Closure gap closing. (The refuse-to-start *mechanism* is
/// still proven at the unit level in
/// `corvid-driver::contract_closure` with a capability toggled off.)
///
/// Adversarial: a protected route MUST reject an unauthenticated request
/// with `401` before its handler or any effect runs — the route must
/// never return its body to a caller without a session.
#[test]
fn serve_enforces_a_requires_authenticated_route_instead_of_refusing_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")

type Secret:
    value: String

server secure_api:
    route GET "/secret" -> json Secret requires authenticated:
        return Secret("classified")
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8203;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_OAUTH_GOOGLE_CLIENT_ID", "test-client-id")
        .env("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "test-client-secret")
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run corvid serve");
    let _guard = ServedApp(child);

    // The server now STARTS — the `requires authenticated` route is
    // contract-closed as of 52f.
    assert!(
        wait_until_ready(port),
        "serve must START now that authorization enforcement exists (:{port})"
    );

    // An unauthenticated request is refused with 401 — the handler never
    // runs and the classified body is never returned.
    let (status, body) = http_get(port, "/secret").expect("GET /secret failed");
    assert_eq!(status, 401, "an unauthenticated request must be 401; body={body}");
    assert!(
        !body.contains("classified"),
        "the protected body must NOT leak to an unauthenticated caller: {body}"
    );
}

/// 52c-1 streaming gate. A `Stream<T>` route response executes as
/// Server-Sent Events end-to-end: `corvid serve` consumes the
/// interpreter's stream channel and flushes each yielded value as a
/// `data: <json>` event, terminated by `event: done`. Proves the
/// `streaming` RuntimeCapability is wired (the route STARTS under
/// Contract Closure) AND that the transport carries the typed events.
#[test]
fn serve_streams_a_stream_route_as_server_sent_events() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type Tick:
    n: Int
    label: String

agent ticker() -> Stream<Tick>:
    yield Tick(1, "first")
    yield Tick(2, "second")
    yield Tick(3, "third")

server ticker_api:
    route GET "/ticks" -> json Stream<Tick>:
        return ticker()
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8204;
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
        "streaming app did not become ready on :{port} — if it refused to \
         start, the `streaming` RuntimeCapability regressed to false"
    );

    let (status, body) = http_get(port, "/ticks").expect("GET /ticks failed");
    assert_eq!(status, 200, "SSE response status (body={body})");
    // Each yielded Tick is a `data:` event carrying its JSON; the stream
    // terminates with `event: done`.
    for needle in [
        "data: {",
        "\"n\":1",
        "\"label\":\"first\"",
        "\"n\":2",
        "\"n\":3",
        "event: done",
    ] {
        assert!(
            body.contains(needle),
            "SSE body must contain `{needle}`: body=\n{body}"
        );
    }
}

/// Minimal HTTP/1.1 multipart/form-data POST of a single file part.
/// Returns `(status, body)`.
fn http_post_multipart(
    port: u16,
    path: &str,
    filename: &str,
    content_type: &str,
    file_bytes: &[u8],
) -> Option<(u16, String)> {
    let boundary = "----corvidtestboundary7MA4YWxkTrZu0gW";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: multipart/form-data; boundary={boundary}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .ok()?;
    stream.write_all(&body).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status: u16 = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let resp_body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((status, resp_body))
}

/// 52c-2 upload gate. An `Upload<Csv>` body route parses the multipart
/// request, enforces the format's accepted MIME, and exposes the file
/// through `body.filename()` / `body.size()` / `body.text()`. A wrong
/// MIME is a structured `400`.
#[test]
fn serve_parses_a_multipart_upload_and_enforces_mime() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type ImportReceipt:
    filename: String
    bytes_len: Int
    preview: String

agent take_import(body: Upload<Csv>) -> ImportReceipt:
    return ImportReceipt(body.filename(), body.size(), body.text())

server import_api:
    route POST "/import" body Upload<Csv> -> json ImportReceipt:
        return take_import(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8205;
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
    assert!(wait_until_ready(port), "upload app did not become ready on :{port}");

    let csv = b"id,name\n1,alice\n2,bob\n";
    let (status, body) =
        http_post_multipart(port, "/import", "data.csv", "text/csv", csv).expect("POST failed");
    assert_eq!(status, 200, "valid CSV upload status (body={body})");
    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(resp.get("filename").and_then(|v| v.as_str()), Some("data.csv"));
    assert_eq!(resp.get("bytes_len").and_then(|v| v.as_i64()), Some(csv.len() as i64));
    assert!(
        resp.get("preview").and_then(|v| v.as_str()).unwrap_or("").contains("alice"),
        "body.text() must decode the CSV: {body}"
    );

    // Wrong MIME → 400 unsupported_media_type.
    let (bad_status, bad_body) =
        http_post_multipart(port, "/import", "data.csv", "application/pdf", csv)
            .expect("POST failed");
    assert_eq!(bad_status, 400, "wrong MIME must be 400: {bad_body}");
    assert!(
        bad_body.contains("unsupported_media_type"),
        "wrong MIME body must name the error: {bad_body}"
    );
}

/// 52c-2 upload coverage: uploads are NOT CSV-only. A binary
/// `Upload<Pdf>` preserves its exact bytes, and an `Upload<Audio>`
/// route accepts `audio/mpeg` — the format→MIME set is the contract's
/// single source of truth (`default_mime_for_format`), so the runtime
/// enforces exactly the media types the contract advertises for every
/// well-known format, not just the few a hand-copied list happened to
/// include.
#[test]
fn serve_uploads_support_binary_and_non_csv_formats() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type UpReceipt:
    filename: String
    size: Int
    mime: String

agent take_pdf(body: Upload<Pdf>) -> UpReceipt:
    return UpReceipt(body.filename(), body.size(), body.content_type())

agent take_audio(body: Upload<Audio>) -> UpReceipt:
    return UpReceipt(body.filename(), body.size(), body.content_type())

server up_api:
    route POST "/pdf" body Upload<Pdf> -> json UpReceipt:
        return take_pdf(body)
    route POST "/audio" body Upload<Audio> -> json UpReceipt:
        return take_audio(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8207;
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
    assert!(wait_until_ready(port), "upload app did not become ready on :{port}");

    // A binary PDF with embedded non-UTF-8 bytes — the byte count must
    // survive the multipart → List<Int> round-trip exactly.
    let pdf: &[u8] = b"%PDF-1.4\n\x00\x01\x02\xff\xfe binary\n";
    let (status, body) =
        http_post_multipart(port, "/pdf", "doc.pdf", "application/pdf", pdf).expect("POST failed");
    assert_eq!(status, 200, "binary PDF upload status (body={body})");
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        resp.get("size").and_then(|v| v.as_i64()),
        Some(pdf.len() as i64),
        "binary bytes must be preserved exactly: {body}"
    );
    assert_eq!(resp.get("mime").and_then(|v| v.as_str()), Some("application/pdf"));

    // Audio/mpeg must be accepted — the divergence fix (serve reused the
    // contract's format→MIME map) added Audio/Video, which a hand-copied
    // list had omitted (silently accepting ANY type for those formats).
    let mp3: &[u8] = b"ID3\x03\x00\x00 audio";
    let (a_status, a_body) =
        http_post_multipart(port, "/audio", "clip.mp3", "audio/mpeg", mp3).expect("POST failed");
    assert_eq!(a_status, 200, "audio upload status (body={a_body})");

    // And a PDF sent to the audio route is refused (wrong media type).
    let (bad, bad_body) =
        http_post_multipart(port, "/audio", "doc.pdf", "application/pdf", pdf).expect("POST failed");
    assert_eq!(bad, 400, "wrong media type must be 400: {bad_body}");
    assert!(bad_body.contains("unsupported_media_type"));
}

/// 52c-2 pagination gate. A `Page<Item>` response route built with
/// `Page(items, next_cursor)` serves the standard cursor envelope
/// `{items, next_cursor, has_more}`, with `next_cursor` unwrapped from
/// the `Option` and `has_more` derived.
#[test]
fn serve_returns_a_page_cursor_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"type Item:
    id: String
    name: String

type ItemQuery:
    cursor: String
    limit: Int

server items_api:
    route GET "/items" query ItemQuery -> json Page<Item>:
        a = Item("i1", "first")
        b = Item("i2", "second")
        return Page([a, b], Some(query.cursor))
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8206;
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
    assert!(wait_until_ready(port), "pagination app did not become ready on :{port}");

    let (status, body) =
        http_get(port, "/items?cursor=abc123&limit=10").expect("GET /items failed");
    assert_eq!(status, 200, "page response status (body={body})");
    let resp: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    assert_eq!(
        resp.get("next_cursor").and_then(|v| v.as_str()),
        Some("abc123"),
        "next_cursor must be the unwrapped cursor string: {body}"
    );
    assert_eq!(
        resp.get("has_more").and_then(|v| v.as_bool()),
        Some(true),
        "has_more must derive from Some(cursor): {body}"
    );
    let items = resp.get("items").and_then(|v| v.as_array()).expect("items array");
    assert_eq!(items.len(), 2, "envelope must carry both items: {body}");
    assert_eq!(items[0].get("id").and_then(|v| v.as_str()), Some("i1"));
}

/// Minimal HTTP/1.1 GET returning the FULL raw response (status line +
/// headers + body), so a test can assert on the `Location` /
/// `Set-Cookie` headers of a redirect.
fn http_get_raw(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    Some(raw)
}

/// Slice 52e end-to-end gate: an `identity` block makes `corvid serve`
/// mount the OAuth login surface. `GET /auth/{provider}/login` must
/// 302 to the provider's authorize URL carrying the client id, an opaque
/// `state`, and a PKCE `code_challenge`; `GET /auth/session` without a
/// cookie must report `authenticated: false`; an unknown provider is a
/// 404. No live IdP is contacted — login only builds the redirect.
#[test]
fn serve_mounts_the_oauth_login_surface_and_redirects_to_the_provider() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let source = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")

server ping_api:
    route GET "/ping" -> json String:
        return "pong"
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8204;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_OAUTH_GOOGLE_CLIENT_ID", "test-client-id")
        .env("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "test-client-secret")
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn corvid serve: {e}"));
    let _guard = ServedApp(child);

    assert!(
        wait_until_ready(port),
        "server with an identity block did not become ready on :{port}"
    );

    // 1. GET /auth/google/login → 302 to Google's authorize URL.
    let raw = http_get_raw(port, "/auth/google/login").expect("GET login failed");
    let status: u16 = raw
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert_eq!(status, 302, "login must 302; raw=\n{raw}");
    let location = raw
        .lines()
        .find_map(|l| l.strip_prefix("location: ").or_else(|| l.strip_prefix("Location: ")))
        .expect("login 302 must carry a Location header");
    assert!(
        location.starts_with("https://accounts.google.com/o/oauth2/v2/auth"),
        "Location must point at Google's authorize endpoint: {location}"
    );
    assert!(location.contains("client_id=test-client-id"), "Location: {location}");
    assert!(location.contains("state="), "Location must carry state: {location}");
    assert!(
        location.contains("code_challenge=") && location.contains("code_challenge_method=S256"),
        "Location must carry a PKCE S256 challenge: {location}"
    );
    assert!(location.contains("nonce="), "OIDC login must carry a nonce: {location}");

    // 2. GET /auth/session with no cookie → 401 authenticated:false.
    let (session_status, session_body) =
        http_get(port, "/auth/session").expect("GET /auth/session failed");
    assert_eq!(session_status, 401, "unauthenticated session must be 401");
    assert!(
        session_body.contains("\"authenticated\":false"),
        "session body: {session_body}"
    );

    // 3. Unknown provider → 404.
    let unknown = http_get_raw(port, "/auth/twitter/login").expect("GET unknown login failed");
    let unknown_status: u16 = unknown
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    assert_eq!(unknown_status, 404, "unknown provider must 404; raw=\n{unknown}");
}

/// Slice 52f durability gate. With `CORVID_SERVE_DATA_DIR` set, the
/// approval queue persists to disk, so a dangerous action queued for
/// approval SURVIVES a server restart — the pending approval is still
/// listed after `corvid serve` is stopped and started again against the
/// same data directory. Without the env var the queue is in-memory (a
/// restart fails closed).
#[test]
fn a_queued_approval_survives_a_restart_with_a_durable_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let data_dir = dir.path().join("state");
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

server durable_api:
    route POST "/send" body SendReq -> json SendReceipt uses send_external:
        return execute_send(body)
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8205;

    // First run: queue an approval, then stop the server.
    let approval_id = {
        let child = Command::new(corvid_bin())
            .arg("serve")
            .arg(&src_path)
            .arg("--listen")
            .arg(format!("127.0.0.1:{port}"))
            .env("CORVID_SERVE_DATA_DIR", &data_dir)
            .current_dir(repo_root())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn corvid serve (run 1)");
        let guard = ServedApp(child);
        assert!(wait_until_ready(port), "run 1 did not become ready");

        let (status, body) = http_post(port, "/send", r#"{"body":"persist me"}"#)
            .expect("POST /send (run 1) failed");
        assert_eq!(status, 202, "POST must queue (202); body={body}");
        let resp: serde_json::Value = serde_json::from_str(&body).expect("202 body is JSON");
        let id = resp
            .get("approval_id")
            .and_then(|v| v.as_str())
            .expect("202 carries an approval_id")
            .to_string();
        drop(guard); // stop the first server
        id
    };

    // The durable database was written.
    assert!(
        data_dir.join("approvals.sqlite").exists(),
        "the durable approval database must exist at the data dir"
    );

    // Give the OS a moment to release the port before rebinding.
    std::thread::sleep(Duration::from_millis(500));

    // Second run against the SAME data dir: the approval is still there.
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_SERVE_DATA_DIR", &data_dir)
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn corvid serve (run 2)");
    let _guard = ServedApp(child);
    assert!(wait_until_ready(port), "run 2 did not become ready");

    let (list_status, list_body) =
        http_get(port, "/__approvals").expect("GET /__approvals (run 2) failed");
    assert_eq!(list_status, 200, "GET /__approvals must answer 200");
    assert!(
        list_body.contains(&approval_id),
        "the queued approval `{approval_id}` must survive the restart: {list_body}"
    );
}

/// Slice 52f end-to-end authorization gate. A role-gated route, served
/// live, ALLOWS a caller whose session holds the required role, DENIES an
/// authenticated caller who lacks it (403), and DENIES an anonymous
/// caller (401). The sessions are seeded through the same
/// `SessionAuthRuntime` the server uses (persisted to the durable data
/// dir) — exactly the state a real login would produce — so the request
/// travels the real enforcement path (`enforce_route_policy`), not a
/// test shortcut.
#[test]
fn a_role_gated_route_allows_the_right_role_and_denies_others() {
    use corvid_runtime::{AuthActor, SessionAuthRuntime, SessionCreate};

    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let data_dir = dir.path().join("state");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Seed two actors + sessions into the durable store the server opens:
    // one with the `admin` role, one with none.
    let admin_token = "admin-session-token";
    let plain_token = "plain-session-token";
    {
        let auth = SessionAuthRuntime::open(data_dir.join("auth.sqlite")).unwrap();
        let mk_actor = |id: &str| AuthActor {
            id: id.to_string(),
            tenant_id: "public".to_string(),
            display_name: id.to_string(),
            actor_kind: "user".to_string(),
            auth_method: "oauth".to_string(),
            assurance_level: "aal1".to_string(),
            role_fingerprint: String::new(),
            permission_fingerprint: String::new(),
            created_ms: 0,
            updated_ms: 0,
        };
        auth.upsert_actor(mk_actor("actor-admin")).unwrap();
        auth.upsert_actor(mk_actor("actor-plain")).unwrap();
        auth.grant_actor_role("actor-admin", "admin", 1).unwrap();
        let mk_session = |id: &str, actor: &str, token: &str| SessionCreate {
            id: id.to_string(),
            actor_id: actor.to_string(),
            tenant_id: "public".to_string(),
            raw_token: token.to_string(),
            issued_ms: 1,
            expires_ms: 9_000_000_000_000,
            csrf_binding_id: format!("csrf-{id}"),
        };
        auth.create_session(mk_session("s-admin", "actor-admin", admin_token))
            .unwrap();
        auth.create_session(mk_session("s-plain", "actor-plain", plain_token))
            .unwrap();
    }

    let source = r#"identity users:
    provider google
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        admin: "billing:read"

server admin_api:
    route GET "/admin" -> json String requires role("admin"):
        return actor.id
"#;
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8206;
    let child = Command::new(corvid_bin())
        .arg("serve")
        .arg(&src_path)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_SERVE_DATA_DIR", &data_dir)
        .env("CORVID_OAUTH_GOOGLE_CLIENT_ID", "test-client-id")
        .env("CORVID_OAUTH_GOOGLE_CLIENT_SECRET", "test-client-secret")
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn corvid serve");
    let _guard = ServedApp(child);
    assert!(wait_until_ready(port), "server did not become ready");

    // Anonymous → 401.
    let (anon_status, _) = http_get(port, "/admin").expect("GET /admin (anon) failed");
    assert_eq!(anon_status, 401, "an anonymous caller must be 401");

    // Authenticated but without the role → 403.
    let (plain_status, plain_body) =
        http_get_with_cookie(port, "/admin", &format!("corvid_session={plain_token}"))
            .expect("GET /admin (plain) failed");
    assert_eq!(
        plain_status, 403,
        "an authenticated caller without `admin` must be 403; body={plain_body}"
    );

    // The admin session → 200, and the handler sees the verified actor id.
    let (admin_status, admin_body) =
        http_get_with_cookie(port, "/admin", &format!("corvid_session={admin_token}"))
            .expect("GET /admin (admin) failed");
    assert_eq!(
        admin_status, 200,
        "the admin session must be allowed; body={admin_body}"
    );
    assert!(
        admin_body.contains("actor-admin"),
        "the handler must see the verified actor id: {admin_body}"
    );
}

/// Slice 52f-4b adversarial gate. An approval decision is now a
/// privileged, authenticated action. Against a single queued approval,
/// EVERY unauthorized decision path is refused before the approval is
/// consumed — only a verified reviewer with the `approvals.decide`
/// permission, in the approval's tenant, who is NOT the requester, with a
/// valid CSRF double-submit, may decide. Then a revoked-role reviewer is
/// refused, and a valid decision succeeds exactly once.
#[test]
fn approval_decisions_reject_every_unauthorized_path() {
    use corvid_runtime::{mint_csrf_token, AuthActor, SessionAuthRuntime, SessionCreate};

    let dir = tempfile::tempdir().unwrap();
    let src_path = dir.path().join("main.cor");
    let data_dir = dir.path().join("state");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Seed a family of sessions into the durable auth store.
    let csrf = |binding: &str| mint_csrf_token(binding, TEST_CSRF_SECRET.as_bytes()).unwrap();
    let headers = |token: &str, binding: &str| -> Vec<(String, String)> {
        let c = csrf(binding);
        vec![
            ("Cookie".to_string(), format!("corvid_session={token}; corvid_csrf={c}")),
            ("X-CSRF-Token".to_string(), c),
        ]
    };
    {
        let auth = SessionAuthRuntime::open(data_dir.join("auth.sqlite")).unwrap();
        let mk = |id: &str, tenant: &str| AuthActor {
            id: id.into(), tenant_id: tenant.into(), display_name: id.into(),
            actor_kind: "user".into(), auth_method: "oauth".into(), assurance_level: "aal1".into(),
            role_fingerprint: String::new(), permission_fingerprint: String::new(),
            created_ms: 0, updated_ms: 0,
        };
        let sess = |id: &str, actor: &str, tenant: &str, token: &str, binding: &str| SessionCreate {
            id: id.into(), actor_id: actor.into(), tenant_id: tenant.into(),
            raw_token: token.into(), issued_ms: 1, expires_ms: 9_000_000_000_000,
            csrf_binding_id: binding.into(),
        };
        // A legitimate reviewer.
        auth.upsert_actor(mk("rev", "serve-default")).unwrap();
        auth.grant_actor_role("rev", "reviewer", 1).unwrap();
        auth.create_session(sess("s-rev", "rev", "serve-default", "rev-token", "b-rev")).unwrap();
        // A reviewer to have their role revoked mid-test.
        auth.upsert_actor(mk("rev2", "serve-default")).unwrap();
        auth.grant_actor_role("rev2", "reviewer", 1).unwrap();
        auth.create_session(sess("s-rev2", "rev2", "serve-default", "rev2-token", "b-rev2")).unwrap();
        // Authenticated but WITHOUT the approvals.decide permission.
        auth.upsert_actor(mk("peon", "serve-default")).unwrap();
        auth.create_session(sess("s-peon", "peon", "serve-default", "peon-token", "b-peon")).unwrap();
        // A reviewer in a DIFFERENT tenant.
        auth.upsert_actor(mk("outsider", "other-tenant")).unwrap();
        auth.grant_actor_role("outsider", "reviewer", 1).unwrap();
        auth.create_session(sess("s-out", "outsider", "other-tenant", "out-token", "b-out")).unwrap();
        // The requester themself (serve-anonymous), even with the permission.
        auth.upsert_actor(mk("serve-anonymous", "serve-default")).unwrap();
        auth.grant_actor_role("serve-anonymous", "reviewer", 1).unwrap();
        auth.create_session(sess("s-self", "serve-anonymous", "serve-default", "self-token", "b-self")).unwrap();
    }

    let source = format!("{IDENTITY_WITH_REVIEWER}{}", r#"type SendReq:
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
    return SendReceipt(true)

server adversary_api:
    route POST "/send" body SendReq -> json SendReceipt uses send_external:
        return execute_send(body)
"#);
    std::fs::write(&src_path, source).unwrap();

    let port: u16 = 8207;
    let child = Command::new(corvid_bin())
        .arg("serve").arg(&src_path).arg("--listen").arg(format!("127.0.0.1:{port}"))
        .envs(reviewer_serve_env(&data_dir))
        .current_dir(repo_root())
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped())
        .spawn().expect("spawn corvid serve");
    let _guard = ServedApp(child);
    assert!(wait_until_ready(port), "server did not become ready");

    // Queue one approval.
    let (_s, body) = http_post(port, "/send", r#"{"body":"decide me"}"#).expect("POST /send");
    let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()
        .get("approval_id").and_then(|v| v.as_str()).unwrap().to_string();
    let url = format!("/__approvals/{id}/approve");

    // None of these consume the approval — each is refused.
    // 1. No session → 401.
    assert_eq!(http_post(port, &url, "").unwrap().0, 401, "no session must be 401");
    // 2. Forged cookie → 401.
    assert_eq!(
        http_post_with_headers(port, &url, "", &[("Cookie".into(), "corvid_session=forged".into())]).unwrap().0,
        401, "a forged cookie must be 401"
    );
    // 3. Valid session, missing CSRF → 403.
    assert_eq!(
        http_post_with_headers(port, &url, "", &[("Cookie".into(), "corvid_session=rev-token".into())]).unwrap().0,
        403, "a mutation without CSRF must be 403"
    );
    // 4. Valid session, CSRF mismatch → 403.
    assert_eq!(
        http_post_with_headers(port, &url, "", &[
            ("Cookie".into(), format!("corvid_session=rev-token; corvid_csrf={}", csrf("b-rev"))),
            ("X-CSRF-Token".into(), "wrong.deadbeef".into()),
        ]).unwrap().0,
        403, "a CSRF mismatch must be 403"
    );
    // 5. Authenticated but lacking approvals.decide → 403.
    assert_eq!(
        http_post_with_headers(port, &url, "", &headers("peon-token", "b-peon")).unwrap().0,
        403, "a reviewer without approvals.decide must be 403"
    );
    // 6. Cross-tenant reviewer → 403.
    assert_eq!(
        http_post_with_headers(port, &url, "", &headers("out-token", "b-out")).unwrap().0,
        403, "a cross-tenant reviewer must be 403"
    );
    // 7. Self-approval (the requester) → 403.
    assert_eq!(
        http_post_with_headers(port, &url, "", &headers("self-token", "b-self")).unwrap().0,
        403, "the requester must not decide their own approval"
    );

    // 8. Role revocation takes effect at once — revoke rev2's role, then
    //    their decision is refused (the session is invalidated too).
    {
        let auth = SessionAuthRuntime::open(data_dir.join("auth.sqlite")).unwrap();
        auth.revoke_actor_role("rev2", "reviewer", 2).unwrap();
    }
    let revoked_status = http_post_with_headers(port, &url, "", &headers("rev2-token", "b-rev2")).unwrap().0;
    assert!(
        revoked_status == 401 || revoked_status == 403,
        "a reviewer whose role was revoked must be denied, got {revoked_status}"
    );

    // The approval is STILL pending after all the refusals.
    let (_g, list) = http_get(port, "/__approvals").expect("GET /__approvals");
    assert!(list.contains(&id), "the approval must survive every refused decision");

    // 9. A legitimate reviewer decides → 200, exactly once; a replay 409s.
    assert_eq!(
        http_post_with_headers(port, &url, "", &headers("rev-token", "b-rev")).unwrap().0,
        200, "a verified reviewer with approvals.decide must succeed"
    );
    assert_eq!(
        http_post_with_headers(port, &url, "", &headers("rev-token", "b-rev")).unwrap().0,
        409, "a second decision on the same approval must be 409 (already decided)"
    );
}
