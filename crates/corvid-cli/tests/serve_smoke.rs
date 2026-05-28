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
