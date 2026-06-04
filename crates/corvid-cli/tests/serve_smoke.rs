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
