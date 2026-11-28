//! Verified webhooks (B16) through a real `corvid serve`.
//!
//! Each test starts the server on a program with `verified` routes and
//! sends signed deliveries the way providers do: the signature is
//! computed here over the exact bytes sent, with a fresh timestamp, and
//! every refusal is checked to answer 401 before the handler runs.

use corvid_crypto::{hmac_sha256, Bytes, HmacKey};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STRIPE_KEY: &str = "whsec_corvid_test_4eC39HqLyjWDarjtT1zdp7dc";
const GITHUB_KEY: &str = "corvid-github-test-key";

const PROGRAM: &str = r#"type PaymentEvent:
    id: String

traces: none

server payments:
    route POST "/webhooks/stripe" body PaymentEvent verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes) -> json String:
        return body.id
    route POST "/webhooks/github" body Bytes verified github(secret: "GITHUB_WEBHOOK_SECRET", replays: accepted) -> json Int:
        return body.length()
"#;

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

struct ServedApp {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn command(dir: &tempfile::TempDir, port: u16, secrets: &[(&str, &str)]) -> Command {
    let src = dir.path().join("main.cor");
    std::fs::write(&src, PROGRAM).unwrap();
    let mut cmd = Command::new(corvid_bin());
    cmd.arg("serve").arg(&src).arg("--listen").arg(format!("127.0.0.1:{port}")).current_dir(dir.path());
    cmd.env_remove("STRIPE_WEBHOOK_SECRET").env_remove("GITHUB_WEBHOOK_SECRET");
    for (name, value) in secrets {
        cmd.env(name, value);
    }
    cmd
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

fn serve() -> ServedApp {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let child = command(&dir, port, &[("STRIPE_WEBHOOK_SECRET", STRIPE_KEY), ("GITHUB_WEBHOOK_SECRET", GITHUB_KEY)])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let app = ServedApp { child, port, _dir: dir };
    let deadline = Instant::now() + Duration::from_secs(60);
    while get(port, "/openapi.json").is_none() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

fn get(port: u16, path: &str) -> Option<(u16, Value)> {
    request(port, "GET", path, &[], b"")
}

fn request(port: u16, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> Option<(u16, Value)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).ok()?;
    stream.write_all(body).ok()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw).ok()?;
    let status = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let reply = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    Some((status, serde_json::from_str(reply).unwrap_or(Value::String(reply.to_string()))))
}

fn hmac_hex(key: &str, message: &[u8]) -> String {
    hmac_sha256(&Bytes::new(message), HmacKey::Bytes(&Bytes::from_utf8_text(key))).to_hex()
}

fn now_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// A delivery Stripe would send for `body` at `timestamp`.
fn stripe_signature(timestamp: u64, body: &str) -> String {
    format!("t={timestamp},v1={}", hmac_hex(STRIPE_KEY, format!("{timestamp}.{body}").as_bytes()))
}

fn deliver_stripe(app: &ServedApp, signature: Option<String>, body: &str) -> (u16, Value) {
    let mut headers = vec![("Content-Type", "application/json".to_string())];
    headers.extend(signature.map(|s| ("Stripe-Signature", s)));
    request(app.port, "POST", "/webhooks/stripe", &headers, body.as_bytes()).expect("request completes")
}

/// The provider's payload carries fields the program does not declare.
const EVENT: &str = r#"{"id":"evt_1QZ","object":"event","type":"payment_intent.succeeded","data":{"amount":2000}}"#;

#[test]
fn a_signed_stripe_delivery_runs_the_handler_and_every_forgery_is_refused_first() {
    let app = serve();
    let fresh = now_s();
    let (status, reply) = deliver_stripe(&app, Some(stripe_signature(fresh, EVENT)), EVENT);
    assert_eq!((status, reply), (200, Value::String("evt_1QZ".into())), "a genuine delivery verifies, and undeclared fields are skipped");

    let refused = |(status, reply): (u16, Value)| (status, reply["error"].as_str().unwrap_or_default().to_string());
    let invalid = (401, "webhook_signature_invalid".to_string());
    let tampered = EVENT.replacen("2000", "9000", 1);
    assert_eq!(refused(deliver_stripe(&app, Some(stripe_signature(fresh, EVENT)), &tampered)), invalid, "a changed body byte");
    let signed_now = stripe_signature(fresh, EVENT);
    let moved = signed_now.replacen(&format!("t={fresh}"), &format!("t={}", fresh + 1), 1);
    assert_eq!(refused(deliver_stripe(&app, Some(moved), EVENT)), invalid, "a changed timestamp");
    assert_eq!(refused(deliver_stripe(&app, None, EVENT)), invalid, "no signature header");
    let other_key = format!("t={fresh},v1={}", hmac_hex("whsec_someone_elses_key", format!("{fresh}.{EVENT}").as_bytes()));
    assert_eq!(refused(deliver_stripe(&app, Some(other_key), EVENT)), invalid, "signed with another key");

    // The recorded request from 2024 is genuinely signed, but its time is
    // outside the five-minute window: a replay.
    let recorded = r#"{"id":"evt_1QZ","type":"payment_intent.succeeded","data":{"object":{"id":"pi_3Q","amount":2000,"currency":"usd"}}}"#;
    let replayed = "t=1732480000,v1=9d4aa1ff5638dd3708a9c29539870fc8fc470059f7feeb7ff559ab641d7f6dce".to_string();
    assert_eq!(refused(deliver_stripe(&app, Some(replayed), recorded)), (401, "webhook_expired".to_string()));
    let rolled = format!("t={fresh},v1={},v1={}", "0".repeat(64), hmac_hex(STRIPE_KEY, format!("{fresh}.{EVENT}").as_bytes()));
    assert_eq!(deliver_stripe(&app, Some(rolled), EVENT).0, 200, "a key being rolled verifies with either signature");
}

#[test]
fn a_bytes_body_reaches_the_handler_exactly_as_it_arrived() {
    let app = serve();
    let body: &[u8] = b"\x00raw \xffbytes, not JSON";
    let headers = |signature: String| vec![("X-Hub-Signature-256", format!("sha256={signature}")), ("Content-Type", "application/octet-stream".to_string())];
    let signature = hmac_sha256(&Bytes::new(body), HmacKey::Bytes(&Bytes::from_utf8_text(GITHUB_KEY))).to_hex();
    let (status, reply) = request(app.port, "POST", "/webhooks/github", &headers(signature.clone()), body).unwrap();
    assert_eq!((status, reply), (200, Value::from(body.len())), "the handler saw every byte");
    let (status, _) = request(app.port, "POST", "/webhooks/github", &headers(signature), b"other bytes").unwrap();
    assert_eq!(status, 401);
}

#[test]
fn the_published_surfaces_mark_a_verified_route_and_never_its_secret() {
    let app = serve();
    let (_, spec) = get(app.port, "/openapi.json").unwrap();
    let stripe = &spec["paths"]["/webhooks/stripe"]["post"];
    assert_eq!(stripe["x-corvid-verified"]["scheme"], "stripe");
    assert_eq!(stripe["x-corvid-verified"]["within_secs"], 300);
    assert!(stripe["parameters"].as_array().unwrap().iter().any(|p| p["in"] == "header" && p["name"] == "Stripe-Signature"));
    assert!(stripe["responses"]["401"].is_object());
    let github = &spec["paths"]["/webhooks/github"]["post"];
    assert!(github["requestBody"]["content"]["application/octet-stream"].is_object(), "{github}");
    assert!(github["x-corvid-verified"].get("within_secs").is_none(), "GitHub signs no time");
    let (_, contract) = get(app.port, "/.well-known/corvid").unwrap();
    let text = contract.to_string();
    assert!(text.contains("\"verified\""), "the contract marks verified routes: {text}");
    for secret in ["STRIPE_WEBHOOK_SECRET", "GITHUB_WEBHOOK_SECRET", STRIPE_KEY, GITHUB_KEY] {
        assert!(!text.contains(secret) && !spec.to_string().contains(secret), "{secret} is never published");
    }
}

#[test]
fn a_verified_route_without_its_secret_refuses_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = command(&dir, free_port(), &[("GITHUB_WEBHOOK_SECRET", GITHUB_KEY)])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the server started without the Stripe secret");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stderr = String::new();
    child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(!status.success());
    assert!(stderr.contains("STRIPE_WEBHOOK_SECRET") && stderr.contains("not set"), "{stderr}");
}
