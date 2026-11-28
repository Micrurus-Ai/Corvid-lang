//! Large uploads held on disk (U2) through a real `corvid serve`.
//!
//! A route declared `@upload(max_mb: 80, ..., disk_over_mb: 16)` keeps a
//! 64 MB upload out of memory: the server's peak memory grows by far less
//! than the file, the file sits in a private directory while the request
//! runs, and it is gone after the request, after a refused one and after
//! a cancelled one. The methods read the same values from disk as from
//! memory. Linux only: the memory bound is read from `/proc`, and the
//! privacy check is Unix's.
#![cfg(target_os = "linux")]

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Three routes with the same upload policy: one reads only the size,
/// one the bytes and one the text.
fn program() -> String {
    let route = |path: &str, body: &str| {
        format!("    @upload(max_mb: 80, mime: \"text/plain\", disk_over_mb: 16)\n    route POST \"{path}\" body Upload<Text> -> json Receipt:\n        {body}\n")
    };
    let mut source = "type Receipt:\n    size: Int\n    digest: String\n\ntraces: none\n\nserver files:\n".to_string();
    source.push_str(&route("/size", "return Receipt(body.size(), \"\")"));
    source.push_str(&route("/digest", "return Receipt(body.size(), body.bytes().sha256().to_hex())"));
    source.push_str(&route("/text", "return Receipt(body.text().length(), \"\")"));
    source
}

struct ServedApp {
    child: Child,
    port: u16,
    data: tempfile::TempDir,
    _dir: tempfile::TempDir,
}

impl Drop for ServedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve() -> ServedApp {
    let dir = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let src = dir.path().join("main.cor");
    std::fs::write(&src, program()).unwrap();
    let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_corvid")))
        .arg("serve")
        .arg(&src)
        .arg("--listen")
        .arg(format!("127.0.0.1:{port}"))
        .env("CORVID_SERVE_DATA_DIR", data.path())
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let app = ServedApp { child, port, data, _dir: dir };
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

const BOUNDARY: &str = "corvid-upload-boundary";

/// The multipart framing around the file's content.
fn multipart_parts() -> (Vec<u8>, Vec<u8>) {
    let head = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.txt\"\r\nContent-Type: text/plain\r\n\r\n"
    )
    .into_bytes();
    let tail = format!("\r\n--{BOUNDARY}--\r\n").into_bytes();
    (head, tail)
}

/// The file's content: printable text, so `text()` reads it whole.
fn content(len: usize) -> Vec<u8> {
    (0..len).map(|i| b'a' + (i % 26) as u8).collect()
}

/// Send `content` as an upload. With `cancel_after`, stop after that many
/// content bytes and close the connection, as a person giving up would.
fn upload(app: &ServedApp, path: &str, content: &[u8], cancel_after: Option<usize>) -> Option<(u16, Value)> {
    let (head, tail) = multipart_parts();
    let total = head.len() + content.len() + tail.len();
    let mut stream = TcpStream::connect(("127.0.0.1", app.port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok()?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: multipart/form-data; boundary={BOUNDARY}\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    stream.write_all(&head).ok()?;
    if let Some(sent) = cancel_after {
        stream.write_all(&content[..sent]).ok()?;
        stream.flush().ok()?;
        std::thread::sleep(Duration::from_millis(500));
        drop(stream);
        return None;
    }
    for piece in content.chunks(1 << 20) {
        stream.write_all(piece).ok()?;
    }
    stream.write_all(&tail).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let raw = String::from_utf8_lossy(&raw);
    let status = raw.lines().next()?.split_whitespace().nth(1)?.parse().ok()?;
    let reply = raw.split("\r\n\r\n").nth(1).unwrap_or("");
    Some((status, serde_json::from_str(reply).unwrap_or(Value::String(reply.to_string()))))
}

/// The server process's peak resident memory, in bytes.
fn peak_memory(app: &ServedApp) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{}/status", app.child.id())).unwrap();
    let line = status.lines().find(|line| line.starts_with("VmHWM:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse::<u64>().unwrap() * 1024
}

/// Every held file under the server's upload directory.
fn held_files(app: &ServedApp) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "upload") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&app.data.path().join("uploads"), &mut out);
    out
}

const MB: usize = 1024 * 1024;

#[test]
fn a_large_upload_is_held_on_disk_so_memory_stays_bounded_and_the_file_is_gone_after() {
    let app = serve();
    // Warm the server with a small upload, so its baseline includes the
    // request path's own allocations.
    assert_eq!(upload(&app, "/size", &content(1024), None).unwrap().0, 200);
    let before = peak_memory(&app);
    let file = content(64 * MB);
    let (status, reply) = upload(&app, "/size", &file, None).unwrap();
    assert_eq!((status, reply["size"].as_u64()), (200, Some(64 * MB as u64)), "{reply}");
    let grown = peak_memory(&app).saturating_sub(before);
    assert!(grown < 40 * MB as u64, "a 64 MB upload under a 16 MB threshold grew peak memory by {} MB", grown / MB as u64);
    assert!(held_files(&app).is_empty(), "the held file is deleted when the request ends: {:?}", held_files(&app));
}

#[test]
fn the_methods_read_the_same_values_from_disk_as_from_memory() {
    let app = serve();
    let small = content(MB);
    let large = content(20 * MB);
    for file in [&small, &large] {
        let (status, reply) = upload(&app, "/digest", file, None).unwrap();
        assert_eq!(status, 200, "{reply}");
        let expected = corvid_crypto::sha256(&corvid_crypto::Bytes::new(file)).to_hex();
        assert_eq!(reply["digest"], Value::String(expected), "the same bytes, held in memory or on disk");
        let (_, text) = upload(&app, "/text", file, None).unwrap();
        assert_eq!(text["size"].as_u64(), Some(file.len() as u64), "text() reads the whole file");
    }
}

#[test]
fn a_cancelled_or_refused_upload_leaves_no_file_and_the_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let app = serve();
    let uploads = app.data.path().join("uploads");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&uploads), 0o700, "the upload directory is the server's own");

    upload(&app, "/size", &content(40 * MB), Some(30 * MB));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !held_files(&app).is_empty() {
        assert!(Instant::now() < deadline, "a cancelled upload's file is deleted: {:?}", held_files(&app));
        std::thread::sleep(Duration::from_millis(100));
    }

    let (status, _) = upload(&app, "/size", &content(81 * MB), None).unwrap_or((413, Value::Null));
    assert_ne!(status, 200, "past the maximum is refused");
    assert!(held_files(&app).is_empty(), "a refused upload's file is deleted: {:?}", held_files(&app));
}
