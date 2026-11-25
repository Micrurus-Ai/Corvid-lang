//! `corvid run --target native` for a program that calls a connector
//! (native dispatch N1): the driver checks the connectors before the
//! binary starts and hands it the selected mode and the project's
//! `[http] allow` list, so the compiled program sends the same request
//! the interpreter would.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

const PROGRAM: &str = r#"
effect http_write:
    cost: 1.0

type NewCharge:
    amount: Int

type Charge:
    id: String

connector payments:
    base_url: "http://api.payments.example"
    timeout: 10s
    redirects: never
    modes: [real]
    operation charge(new: NewCharge) -> Charge uses http_write:
        POST "/v1/charges" body new

agent main() -> String:
    return charge(NewCharge(500)).id
"#;

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

/// Links the binary against the release `corvid-test-tools` staticlib,
/// which bundles the current runtime, as the native codegen tests do.
fn current_runtime_staticlib() -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "corvid-test-tools", "--release"])
        .current_dir(corvid_test_paths::workspace_root())
        .status()
        .expect("build corvid-test-tools");
    assert!(status.success(), "building corvid-test-tools failed");
    corvid_test_paths::test_tools_staticlib()
}

/// A loopback HTTP proxy standing in for the provider: it answers one
/// request with a charge and hands back what it received.
fn provider() -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (socket, _) = listener.accept().expect("a request reaches the provider");
        let mut reader = BufReader::new(socket);
        let mut request = String::new();
        let mut length = 0usize;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            request.push_str(&line);
            if line == "\r\n" {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        request.push_str(&String::from_utf8(body).unwrap());
        let reply = r#"{"id":"ch_native"}"#;
        write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        )
        .unwrap();
        request
    });
    (url, handle)
}

#[test]
fn corvid_run_native_sends_a_connector_request_under_the_project_policy() {
    let staticlib = current_runtime_staticlib();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.cor"), PROGRAM).unwrap();
    std::fs::write(project.path().join("corvid.toml"), "[http]\nallow = [\"api.payments.example\"]\n").unwrap();
    let (proxy, provider) = provider();

    let output = Command::new(corvid_bin())
        .current_dir(project.path())
        .args(["run", "src/main.cor", "--target", "native", "--mode", "real"])
        .env("CORVID_RUNTIME_STATICLIB_OVERRIDE", &staticlib)
        .env("HTTP_PROXY", &proxy)
        .env("http_proxy", &proxy)
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .env_remove("CORVID_HTTP_ALLOW")
        .env_remove("CORVID_CONNECTOR_MODE")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout.trim(), "ch_native", "stderr: {stderr}");
    assert!(!stderr.contains("running via interpreter"), "the program ran natively: {stderr}");
    let request = provider.join().unwrap();
    assert!(request.starts_with("POST http://api.payments.example/v1/charges "), "{request}");
    assert!(request.ends_with(r#"{"amount":500}"#), "{request}");
}

#[test]
fn corvid_run_native_refuses_a_host_outside_the_project_policy() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(project.path().join("src/main.cor"), PROGRAM).unwrap();
    std::fs::write(project.path().join("corvid.toml"), "[http]\nallow = [\"api.other.example\"]\n").unwrap();

    let output = Command::new(corvid_bin())
        .current_dir(project.path())
        .args(["run", "src/main.cor", "--target", "native", "--mode", "real"])
        .env_remove("CORVID_HTTP_ALLOW")
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("api.payments.example"), "the refusal names the host: {stderr}");
}
