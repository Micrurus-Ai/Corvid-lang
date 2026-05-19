use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SOURCE: &str = r#"
agent main() -> String:
    return "hello from corvid"
"#;

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

fn server_binary_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}_server.exe")
    } else {
        format!("{stem}_server")
    }
}

fn run_corvid(args: &[String], cwd: &Path) -> std::process::Output {
    Command::new(corvid_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run corvid")
}

fn http_request(addr: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect server");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .write_all(request.as_bytes())
        .expect("write request");
    let mut bytes = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
            Err(err) if !bytes.is_empty() => {
                eprintln!("server closed connection after partial response: {err}");
                break;
            }
            Err(err) => panic!("read response: {err}"),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn http_get(addr: &str, path: &str) -> String {
    http_request(
        addr,
        &format!("GET {path} HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n"),
    )
}

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn build_server_emits_runnable_local_http_binary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("src dir");
    let source_path = src_dir.join("hello.cor");
    std::fs::write(&source_path, SOURCE).expect("write source");

    let args = vec![
        "build".to_string(),
        source_path.to_string_lossy().into_owned(),
        "--target=server".to_string(),
    ];
    let out = run_corvid(&args, dir.path());
    assert!(
        out.status.success(),
        "server build failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let server = dir
        .path()
        .join("target")
        .join("server")
        .join(server_binary_name("hello"));
    let handler = dir
        .path()
        .join("target")
        .join("bin")
        .join(if cfg!(windows) { "hello.exe" } else { "hello" });
    assert!(server.exists(), "missing server binary at {}", server.display());
    assert!(handler.exists(), "missing handler binary at {}", handler.display());

    let child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");
    let mut child = ChildGuard(child);
    let stdout = child.0.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader.read_line(&mut line).expect("read listening line");
    }
    assert!(
        line.starts_with("listening: http://"),
        "unexpected server stdout line: {line:?}"
    );
    let addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("listening prefix");

    let health = http_get(addr, "/healthz");
    assert!(health.contains("HTTP/1.1 200 OK"), "{health}");
    assert!(health.contains(r#"{"status":"ok"}"#), "{health}");
    assert!(health.contains("x-corvid-request-id:"), "{health}");
    assert!(health.contains("x-corvid-middleware:"), "{health}");
    assert!(health.contains("csrf"), "{health}");
    assert!(health.contains("x-corvid-effect-policy: enforced"), "{health}");

    let ready = http_get(addr, "/readyz");
    assert!(ready.contains("HTTP/1.1 200 OK"), "{ready}");
    assert!(ready.contains(r#"{"ready":true}"#), "{ready}");

    let root = http_get(addr, "/");
    assert!(root.contains("HTTP/1.1 200 OK"), "{root}");
    assert!(root.contains(r#""result":"hello from corvid""#), "{root}");

    let rejected = http_request(
        addr,
        &format!("POST / HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n"),
    );
    assert!(
        rejected.contains("HTTP/1.1 405 Method Not Allowed"),
        "{rejected}"
    );
    assert!(rejected.contains("content-type: application/json"), "{rejected}");
    assert!(rejected.contains("x-corvid-request-id: req-"), "{rejected}");
    assert!(rejected.contains(r#""request_id":"req-"#), "{rejected}");
    assert!(rejected.contains(r#""route":"/""#), "{rejected}");
    assert!(
        rejected.contains(r#""kind":"method_not_allowed""#),
        "{rejected}"
    );
    assert!(
        rejected.contains(r#""message":"method not allowed""#),
        "{rejected}"
    );

    let query = http_get(addr, "/?source=parser");
    assert!(query.contains("HTTP/1.1 200 OK"), "{query}");
    assert!(query.contains(r#""result":"hello from corvid""#), "{query}");

    let oversized = http_request(
        addr,
        &format!(
            "POST / HTTP/1.1\r\nhost: {addr}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            4097
        ),
    );
    assert!(
        oversized.contains("HTTP/1.1 413 Payload Too Large"),
        "{oversized}"
    );
    assert!(
        oversized.contains(r#""kind":"body_too_large""#),
        "{oversized}"
    );

    let metrics = http_get(addr, "/metrics");
    assert!(metrics.contains("HTTP/1.1 200 OK"), "{metrics}");
    assert!(metrics.contains(r#""request_total":"#), "{metrics}");
    assert!(metrics.contains(r#""error_total":"#), "{metrics}");
    assert!(metrics.contains(r#""runtime":"corvid-server""#), "{metrics}");

    drop(child);

    let timeout_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_HANDLER_TIMEOUT_MS", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn timeout server");
    let mut timeout_child = ChildGuard(timeout_child);
    let stdout = timeout_child.0.stdout.take().expect("timeout server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader
            .read_line(&mut line)
            .expect("read timeout listening line");
    }
    let timeout_addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("timeout listening prefix");
    let timeout = http_get(timeout_addr, "/");
    assert!(
        timeout.contains("HTTP/1.1 504 Gateway Timeout"),
        "{timeout}"
    );
    assert!(timeout.contains(r#""kind":"handler_timeout""#), "{timeout}");

    drop(timeout_child);

    let auth_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_REQUIRE_AUTH", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn auth server");
    let mut auth_child = ChildGuard(auth_child);
    let stdout = auth_child.0.stdout.take().expect("auth server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader
            .read_line(&mut line)
            .expect("read auth listening line");
    }
    let auth_addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("auth listening prefix");
    let unauthorized = http_get(auth_addr, "/healthz");
    assert!(
        unauthorized.contains("HTTP/1.1 401 Unauthorized"),
        "{unauthorized}"
    );
    assert!(
        unauthorized.contains(r#""kind":"auth_required""#),
        "{unauthorized}"
    );
    drop(auth_child);

    let rate_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_RATE_LIMIT_REQUESTS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rate-limit server");
    let mut rate_child = ChildGuard(rate_child);
    let stdout = rate_child.0.stdout.take().expect("rate-limit server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader
            .read_line(&mut line)
            .expect("read rate-limit listening line");
    }
    let rate_addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("rate-limit listening prefix");
    let first = http_get(rate_addr, "/healthz");
    assert!(first.contains("HTTP/1.1 200 OK"), "{first}");
    let limited = http_get(rate_addr, "/healthz");
    assert!(
        limited.contains("HTTP/1.1 429 Too Many Requests"),
        "{limited}"
    );
    assert!(limited.contains(r#""kind":"rate_limited""#), "{limited}");
    drop(rate_child);

    let hidden_handler = handler.with_extension("missing");
    std::fs::rename(&handler, &hidden_handler).expect("hide handler binary");
    let isolated_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn handler-isolation server");
    let mut isolated_child = ChildGuard(isolated_child);
    let stdout = isolated_child
        .0
        .stdout
        .take()
        .expect("handler-isolation server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader
            .read_line(&mut line)
            .expect("read handler-isolation listening line");
    }
    let isolated_addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("handler-isolation listening prefix");
    let failed_handler = http_get(isolated_addr, "/");
    assert!(
        failed_handler.contains("HTTP/1.1 500 Internal Server Error"),
        "{failed_handler}"
    );
    assert!(
        failed_handler.contains(r#""kind":"handler_spawn_failed""#),
        "{failed_handler}"
    );
    drop(isolated_child);
    std::fs::rename(&hidden_handler, &handler).expect("restore handler binary");

    let invalid_config = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_HANDLER_TIMEOUT_MS", "super-secret-invalid")
        .output()
        .expect("run invalid config server");
    assert!(!invalid_config.status.success());
    let invalid_stderr = String::from_utf8_lossy(&invalid_config.stderr);
    assert!(
        invalid_stderr.contains("CORVID_HANDLER_TIMEOUT_MS invalid"),
        "{invalid_stderr}"
    );
    assert!(invalid_stderr.contains("value redacted"), "{invalid_stderr}");
    assert!(
        !invalid_stderr.contains("super-secret-invalid"),
        "{invalid_stderr}"
    );

    let doctor = Command::new(corvid_bin())
        .arg("doctor")
        .env("CORVID_HANDLER_TIMEOUT_MS", "super-secret-invalid")
        .output()
        .expect("run doctor");
    assert!(!doctor.status.success());
    let doctor_stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(
        doctor_stdout.contains("CORVID_HANDLER_TIMEOUT_MS invalid"),
        "{doctor_stdout}"
    );
    assert!(doctor_stdout.contains("value redacted"), "{doctor_stdout}");
    assert!(
        !doctor_stdout.contains("super-secret-invalid"),
        "{doctor_stdout}"
    );

    let trace_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_MAX_REQUESTS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn trace server");
    let mut trace_child = ChildGuard(trace_child);
    let stdout = trace_child.0.stdout.take().expect("trace server stdout");
    let mut stderr = trace_child.0.stderr.take().expect("trace server stderr");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader
            .read_line(&mut line)
            .expect("read trace listening line");
    }
    let trace_addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("trace listening prefix");
    let traced = http_get(trace_addr, "/healthz");
    assert!(traced.contains("HTTP/1.1 200 OK"), "{traced}");
    let _ = trace_child.0.wait();
    let mut traces = String::new();
    stderr.read_to_string(&mut traces).expect("read traces");
    assert!(
        traces.contains(r#""event":"corvid.server.request""#),
        "{traces}"
    );
    assert!(traces.contains(r#""method":"GET""#), "{traces}");
    assert!(traces.contains(r#""route":"/healthz""#), "{traces}");
    assert!(traces.contains(r#""status":200"#), "{traces}");
    assert!(traces.contains(r#""effects":[]"#), "{traces}");
}

#[test]
fn refund_api_backend_example_checks_and_builds() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let example = repo.join("examples").join("backend").join("refund_api");

    let contract = example.join("src").join("refund_api.cor");
    let check = run_corvid(
        &["check".to_string(), contract.to_string_lossy().into_owned()],
        &repo,
    );
    assert!(
        check.status.success(),
        "refund contract check failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );

    let entrypoint = example.join("src").join("main.cor");
    let build = run_corvid(
        &[
            "build".to_string(),
            entrypoint.to_string_lossy().into_owned(),
            "--target=server".to_string(),
        ],
        &repo,
    );
    assert!(
        build.status.success(),
        "refund server build failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
}

#[test]
fn shared_app_template_checks_and_builds() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entrypoint = repo
        .join("examples")
        .join("backend")
        .join("shared_app_template")
        .join("src")
        .join("main.cor");

    let check = run_corvid(
        &["check".to_string(), entrypoint.to_string_lossy().into_owned()],
        &repo,
    );
    assert!(
        check.status.success(),
        "shared app template check failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );

    let build = run_corvid(
        &[
            "build".to_string(),
            entrypoint.to_string_lossy().into_owned(),
            "--target=server".to_string(),
        ],
        &repo,
    );
    assert!(
        build.status.success(),
        "shared app template server build failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
}

/// Slice 35V2-P39-C-LR (end-to-end, named-threat):
/// `CSRF-bypass-on-PUT/PATCH/DELETE`.
///
/// Builds the rendered axum server, sets `CORVID_CSRF_SECRET` so
/// the middleware enforces CSRF, then asserts:
///
///   - GET passes without any token (safe method).
///   - POST without the `x-corvid-csrf` header is refused with
///     403 csrf_violation (the central threat — the rendered
///     server never reaches the handler).
///   - POST with both cookie + header carrying a valid
///     double-submit token passes the CSRF middleware (the
///     downstream handler still 405s for this GET-only fixture,
///     proving CSRF allowed the request past the gate).
///   - POST with a forged token (no knowledge of the server
///     secret) is refused with 403 csrf_violation.
#[test]
fn rendered_server_csrf_middleware_refuses_state_change_without_double_submit_token() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("src dir");
    let source_path = src_dir.join("hello.cor");
    std::fs::write(&source_path, SOURCE).expect("write source");

    let args = vec![
        "build".to_string(),
        source_path.to_string_lossy().into_owned(),
        "--target=server".to_string(),
    ];
    let out = run_corvid(&args, dir.path());
    assert!(
        out.status.success(),
        "server build failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let server = dir
        .path()
        .join("target")
        .join("server")
        .join(server_binary_name("hello"));

    const SECRET: &str = "csrf-test-secret-32-bytes-long-ok!";
    let child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_CSRF_SECRET", SECRET)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");
    let mut child = ChildGuard(child);
    let stdout = child.0.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader.read_line(&mut line).expect("read listening line");
    }
    let addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("listening prefix");

    // Safe method passes — no CSRF check on GET.
    let safe = http_get(addr, "/healthz");
    assert!(safe.contains("HTTP/1.1 200 OK"), "{safe}");

    // POST without the CSRF header is refused with 403 — the
    // named CSRF-bypass-on-PUT/PATCH/DELETE threat.
    let bypass = http_request(
        addr,
        &format!("POST / HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\n\r\n"),
    );
    assert!(
        bypass.contains("HTTP/1.1 403 Forbidden"),
        "expected 403 on POST without CSRF, got: {bypass}"
    );
    assert!(
        bypass.contains(r#""kind":"csrf_violation""#),
        "expected csrf_violation kind: {bypass}"
    );

    // Mint a valid double-submit token. Mirror the rendered
    // server's HMAC scheme: hex(HMAC-SHA256(secret,
    // "corvid-csrf-v1:" || binding)).
    let binding = "sess-test";
    let valid_token = mint_csrf_for_test(binding, SECRET.as_bytes());

    // POST with valid cookie + header passes the CSRF gate.
    // The handler still 405s (the fixture's main() is
    // GET-only), but the response shape proves the middleware
    // let the request through to the handler.
    let allowed = http_request(
        addr,
        &format!(
            "POST / HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\nx-corvid-csrf: {valid_token}\r\ncookie: corvid_csrf={valid_token}\r\n\r\n",
        ),
    );
    assert!(
        allowed.contains("HTTP/1.1 405 Method Not Allowed"),
        "expected 405 on POST with valid CSRF (fixture is GET-only), got: {allowed}"
    );
    assert!(
        !allowed.contains("csrf_violation"),
        "valid CSRF token should not produce csrf_violation: {allowed}"
    );

    // POST with a forged token (no knowledge of the secret)
    // is refused on HMAC verification.
    let forged_token = format!("{binding}.deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
    let forged = http_request(
        addr,
        &format!(
            "POST / HTTP/1.1\r\nhost: {addr}\r\nconnection: close\r\nx-corvid-csrf: {forged_token}\r\ncookie: corvid_csrf={forged_token}\r\n\r\n",
        ),
    );
    assert!(
        forged.contains("HTTP/1.1 403 Forbidden"),
        "expected 403 on POST with forged CSRF, got: {forged}"
    );
    assert!(
        forged.contains(r#""kind":"csrf_violation""#),
        "expected csrf_violation kind on forged token: {forged}"
    );

    drop(child);
}

fn mint_csrf_for_test(binding: &str, secret: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(b"corvid-csrf-v1:");
    mac.update(binding.as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(digest.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("{binding}.{hex}")
}

/// Slice 43P-LR (end-to-end): the rendered axum server signs
/// the `/__ops` snapshot with the operator-supplied ed25519
/// key when `CORVID_OPS_SIGNING_KEY` is set, and the
/// `corvid ops show` CLI verifies it against the matching
/// public key.
///
/// Asserts the full producer-consumer loop:
///
///   - GET /__ops without `CORVID_OPS_SIGNING_KEY` returns
///     503 (fail-closed; an unsigned snapshot is exactly what
///     a MITM would produce).
///   - With the key set, GET /__ops returns a DSSE envelope
///     whose payloadType is `corvid.ops.show.v1`.
///   - `corvid ops show --envelope-file <body> --pubkey <pub>`
///     verifies the envelope and prints the snapshot.
///   - Verifying with the WRONG public key (man-in-the-middle
///     simulation) returns a non-zero exit.
#[test]
fn rendered_server_ops_show_signs_snapshot_and_cli_verifies_it() {
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;

    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("src dir");
    let source_path = src_dir.join("hello.cor");
    std::fs::write(&source_path, SOURCE).expect("write source");

    let args = vec![
        "build".to_string(),
        source_path.to_string_lossy().into_owned(),
        "--target=server".to_string(),
    ];
    let out = run_corvid(&args, dir.path());
    assert!(
        out.status.success(),
        "server build failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let server = dir
        .path()
        .join("target")
        .join("server")
        .join(server_binary_name("hello"));

    let signing_key = SigningKey::generate(&mut OsRng);
    let signing_hex = hex::encode(signing_key.to_bytes());
    let pubkey_hex = hex::encode(signing_key.verifying_key().as_bytes());

    // 1. Without CORVID_OPS_SIGNING_KEY → /__ops fails closed.
    let unsigned_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn unsigned server");
    let mut unsigned_child = ChildGuard(unsigned_child);
    let stdout = unsigned_child.0.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader.read_line(&mut line).expect("read listening line");
    }
    let addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("listening prefix");
    let unsigned = http_get(addr, "/__ops");
    assert!(
        unsigned.contains("HTTP/1.1 503"),
        "expected 503 fail-closed on /__ops without signing key, got: {unsigned}"
    );
    assert!(
        unsigned.contains("ops_signing_not_configured"),
        "expected ops_signing_not_configured kind: {unsigned}"
    );
    drop(unsigned_child);

    // 2. With the key + build_id set → /__ops returns a DSSE
    // envelope the CLI verifies under the matching pubkey.
    let signed_child = Command::new(&server)
        .env("CORVID_PORT", "0")
        .env("CORVID_OPS_SIGNING_KEY", &signing_hex)
        .env("CORVID_OPS_KEY_ID", "deploy-key-1")
        .env("CORVID_BUILD_ID", "git:test-build-1234")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn signed server");
    let mut signed_child = ChildGuard(signed_child);
    let stdout = signed_child.0.stdout.take().expect("server stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let start = Instant::now();
    while line.is_empty() && start.elapsed() < Duration::from_secs(10) {
        reader.read_line(&mut line).expect("read listening line");
    }
    let addr = line
        .trim()
        .strip_prefix("listening: http://")
        .expect("listening prefix");

    let signed = http_get(addr, "/__ops");
    assert!(signed.contains("HTTP/1.1 200 OK"), "{signed}");
    assert!(
        signed.contains("application/vnd.corvid.ops.show+json"),
        "expected ops payloadType in envelope JSON: {signed}"
    );

    // Extract the envelope body (everything after the blank
    // header-body separator).
    let body_start = signed
        .find("\r\n\r\n")
        .expect("HTTP body separator")
        + 4;
    let envelope_json = &signed[body_start..];
    let envelope_path = dir.path().join("ops.json");
    std::fs::write(&envelope_path, envelope_json).unwrap();

    let pubkey_path = dir.path().join("deploy.pub");
    std::fs::write(&pubkey_path, &pubkey_hex).unwrap();

    let verify = Command::new(corvid_bin())
        .args([
            "ops",
            "show",
            "--envelope-file",
            envelope_path.to_string_lossy().as_ref(),
            "--pubkey",
            pubkey_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run corvid ops show");
    assert!(
        verify.status.success(),
        "corvid ops show failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
    let verify_stdout = String::from_utf8_lossy(&verify.stdout);
    assert!(
        verify_stdout.contains("git:test-build-1234"),
        "expected build_id in CLI output: {verify_stdout}"
    );
    assert!(
        verify_stdout.contains("signature-verified"),
        "expected signature-verified marker: {verify_stdout}"
    );

    // 3. Verifying with the WRONG pubkey (MITM simulation) is
    // refused with a non-zero exit.
    let attacker_key = SigningKey::generate(&mut OsRng);
    let wrong_pubkey_path = dir.path().join("attacker.pub");
    std::fs::write(
        &wrong_pubkey_path,
        hex::encode(attacker_key.verifying_key().as_bytes()),
    )
    .unwrap();
    let mitm_verify = Command::new(corvid_bin())
        .args([
            "ops",
            "show",
            "--envelope-file",
            envelope_path.to_string_lossy().as_ref(),
            "--pubkey",
            wrong_pubkey_path.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("run corvid ops show with wrong pubkey");
    assert!(
        !mitm_verify.status.success(),
        "corvid ops show should have rejected wrong pubkey but exited 0"
    );

    drop(signed_child);
}
