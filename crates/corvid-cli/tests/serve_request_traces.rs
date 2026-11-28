//! Lineage L3b through a real `corvid serve`: a program that declares
//! `traces: location, keep` records each request as its own run, returns
//! the run id in `Corvid-Run`, and roots the run's lineage at the route.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct ServedApp {
    child: Child,
    port: u16,
    traces: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const PROGRAM: &str = r#"type Card:
    title: String

traces:
    location: env("CORVID_TEST_TRACE_DIR")
    keep: 720 hours

server board:
    route POST "/cards" body Card -> json String:
        return "added " + body.title
"#;

fn corvid() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

fn serve() -> ServedApp {
    let dir = tempfile::tempdir().unwrap();
    let traces = tempfile::tempdir().unwrap();
    let src = dir.path().join("main.cor");
    std::fs::write(&src, PROGRAM).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let child = Command::new(corvid())
        .args(["serve", src.to_str().unwrap(), "--listen", &format!("127.0.0.1:{port}")])
        // A directory that does not exist yet, so the server must create
        // it, privately.
        .env("CORVID_TEST_TRACE_DIR", traces.path().join("runs"))
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let app = ServedApp { child, port, traces, _dir: dir };
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

/// POST `body` to `/cards`; the response's status line and headers.
fn add_card(app: &ServedApp, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", app.port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(
        stream,
        "POST /cards HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    raw
}

fn run_header(response: &str) -> Option<String> {
    response
        .lines()
        .find_map(|line| line.to_ascii_lowercase().strip_prefix("corvid-run:").map(|_| line[11..].trim().to_string()))
}

#[test]
fn each_request_is_its_own_run_named_in_the_response_and_rooted_at_its_route() {
    let app = serve();
    let first = add_card(&app, r#"{"title":"Ship it"}"#);
    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    let run = run_header(&first).unwrap_or_else(|| panic!("no Corvid-Run header:\n{first}"));
    let second = run_header(&add_card(&app, r#"{"title":"Again"}"#)).unwrap();
    assert_ne!(run, second, "each request is a run of its own");

    let trace_path = app.traces.path().join("runs").join(format!("{run}.jsonl"));
    let trace = std::fs::read_to_string(&trace_path).unwrap_or_else(|e| panic!("{}: {e}", trace_path.display()));
    assert!(trace.contains("route.request") && trace.contains("/cards"), "the run names its route:\n{trace}");
    assert!(trace.contains("Ship it") && !trace.contains("Again"), "the run holds only its own request");

    let lineage = Command::new(corvid()).args(["trace", "lineage", trace_path.to_str().unwrap()]).output().unwrap();
    let tree = String::from_utf8_lossy(&lineage.stdout);
    assert!(tree.contains("POST /cards"), "the lineage is rooted at the route:\n{tree}");
}

#[cfg(unix)]
#[test]
fn the_traces_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let app = serve();
    let mode = std::fs::metadata(app.traces.path().join("runs")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
