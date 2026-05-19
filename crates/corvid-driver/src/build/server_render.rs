//! Generated-server source rendering.
//!
//! `corvid build --target server` emits a Rust crate (Cargo.toml +
//! src/main.rs) wrapping the user's Corvid program in either an
//! Axum HTTP server (when at least one `route` decl is present) or
//! a minimal `TcpListener`-based handler (otherwise). This module
//! owns those template renderers plus the small naming helpers
//! that pick the package, binary, and crate-root paths.

use std::path::{Path, PathBuf};

pub(super) fn server_binary_path_for(out_dir: &Path, stem: &str) -> PathBuf {
    if cfg!(windows) {
        out_dir.join(format!("{stem}_server.exe"))
    } else {
        out_dir.join(format!("{stem}_server"))
    }
}

pub(super) fn server_binary_name_for_package(package: &str) -> String {
    if cfg!(windows) {
        format!("{package}.exe")
    } else {
        package.to_string()
    }
}

pub(super) fn server_package_name(stem: &str) -> String {
    let mut out = String::from("corvid_generated_");
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

pub(super) fn render_server_cargo_toml(package: &str) -> String {
    format!(
        r#"[package]
name = "{package}"
version = "0.0.0"
edition = "2021"

[workspace]

[dependencies]
axum = "0.7"
tokio = {{ version = "1", features = ["full"] }}
tower-http = {{ version = "0.6", features = ["compression-full", "cors", "trace"] }}
hmac = "0.12"
sha2 = "0.10"
subtle = "2.6"
ed25519-dalek = {{ version = "2", features = ["std"] }}
base64 = "0.22"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
    )
}

pub(super) fn render_axum_server_source(handler_path: &Path) -> String {
    let handler = handler_path.to_string_lossy().replace('\\', "\\\\");
    format!(
        r#"use axum::extract::State;
use axum::http::{{HeaderValue, Method, Request, StatusCode}};
use axum::middleware::Next;
use axum::response::{{IntoResponse, Response}};
use axum::routing::get;
use axum::middleware;
use axum::Router;
use base64::Engine;
use ed25519_dalek::{{ed25519::signature::Signer, SigningKey}};
use hmac::{{Hmac, Mac}};
use serde::Serialize;
use sha2::Sha256;
use std::io::Read;
use std::process::{{Command, Stdio}};
use std::sync::atomic::{{AtomicU64, Ordering}};
use std::sync::{{Arc, Mutex}};
use std::time::{{Duration, Instant, SystemTime, UNIX_EPOCH}};
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

type HmacSha256 = Hmac<Sha256>;

const HANDLER: &str = "{handler}";
const MAX_REQUEST_BYTES: usize = 4096;
// Mirrors `corvid-runtime::auth::csrf` (the canonical
// implementation + adversarial tests live there). Inlined here
// so the rendered server stays standalone; the `build_server`
// integration test asserts behavioural equivalence end-to-end.
const CSRF_BINDING_DOMAIN: &[u8] = b"corvid-csrf-v1:";
const CSRF_HEADER: &str = "x-corvid-csrf";
const CSRF_COOKIE: &str = "corvid_csrf";
// Mirrors `corvid-runtime::ops_show` — canonical implementation
// + 5 adversarial tests live there. The rendered server
// produces the signed envelope at `/__ops`; the `corvid ops
// show` CLI verifies it. The pinned DSSE payload type prevents
// signature replay across artifacts (an ABI attestation
// signature cannot be replayed against the ops surface).
const OPS_SHOW_PAYLOAD_TYPE: &str =
    "application/vnd.corvid.ops.show+json; version=1";
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static REQUEST_TOTAL: AtomicU64 = AtomicU64::new(0);
static ERROR_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
struct AppState {{
    max_requests: Option<u64>,
    require_auth: bool,
    rate_limit_requests: Option<u64>,
    /// CSRF server secret. Empty means CSRF enforcement is off
    /// (default; backwards-compatible). Non-empty enables the
    /// double-submit verifier on every state-changing method.
    /// Sourced from `CORVID_CSRF_SECRET` at startup.
    csrf_secret: Arc<Vec<u8>>,
    /// Optional ed25519 signing key for the `/__ops` snapshot.
    /// When `None`, `/__ops` returns 503 (fail-closed: a
    /// snapshot with no signature is exactly what a MITM would
    /// produce, so refuse rather than serve unsigned).
    /// Sourced from `CORVID_OPS_SIGNING_KEY` at startup.
    ops_signing: Option<Arc<OpsSigning>>,
    /// Free-form binary identifier embedded in every signed
    /// `/__ops` snapshot. Operators compare this against the
    /// expected deployed build id. Sourced from
    /// `CORVID_BUILD_ID` at startup; defaults to `unknown`.
    build_id: Arc<String>,
    /// Unix-epoch milliseconds at which this process started.
    started_unix_ms: u64,
    rate_limit_seen: Arc<AtomicU64>,
    handled_requests: Arc<AtomicU64>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}}

#[derive(Clone)]
struct OpsSigning {{
    key: Arc<SigningKey>,
    key_id: Arc<String>,
}}

#[tokio::main]
async fn main() -> std::io::Result<()> {{
    let host = std::env::var("CORVID_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("CORVID_PORT").unwrap_or_else(|_| "8080".to_string());
    validate_runtime_config()?;
    let listener = TcpListener::bind(format!("{{host}}:{{port}}")).await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let state = AppState {{
        max_requests: max_requests(),
        require_auth: require_auth(),
        rate_limit_requests: rate_limit_requests(),
        csrf_secret: Arc::new(csrf_secret()),
        ops_signing: load_ops_signing()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?,
        build_id: Arc::new(build_id()),
        started_unix_ms,
        rate_limit_seen: Arc::new(AtomicU64::new(0)),
        handled_requests: Arc::new(AtomicU64::new(0)),
        shutdown: Arc::new(Mutex::new(Some(shutdown_tx))),
    }};
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route("/__ops", get(ops_show))
        .fallback(handle_app)
        .layer(middleware::from_fn_with_state(state.clone(), backend_middleware))
        .layer(CompressionLayer::new())
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    println!("listening: http://{{addr}}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {{
            let _ = shutdown_rx.await;
        }})
        .await
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err.to_string()))?;
    Ok(())
}}

async fn backend_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {{
    let started = Instant::now();
    let request_id = request_id();
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    if state.require_auth
        && request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with("Bearer "))
            .is_none()
    {{
        return error_response(
            state,
            401,
            &method,
            &path,
            "auth_required",
            "authorization bearer token required",
            request_id,
            started,
        );
    }}
    if !state.csrf_secret.is_empty() {{
        if let Err(reason) = verify_csrf(&method, request.headers(), &state.csrf_secret) {{
            return error_response(
                state,
                403,
                &method,
                &path,
                "csrf_violation",
                reason,
                request_id,
                started,
            );
        }}
    }}
    if let Some(limit) = state.rate_limit_requests {{
        let seen = state.rate_limit_seen.fetch_add(1, Ordering::Relaxed) + 1;
        if seen > limit {{
            return error_response(
                state,
                429,
                &method,
                &path,
                "rate_limited",
                "request rate limit exceeded",
                request_id,
                started,
            );
        }}
    }}
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "x-corvid-middleware",
        HeaderValue::from_static("auth,csrf,rate_limit,tracing,cors,compression,request_logging,effect_policy"),
    );
    headers.insert("x-corvid-effect-policy", HeaderValue::from_static("enforced"));
    response
}}

async fn healthz(State(state): State<AppState>, request: Request<axum::body::Body>) -> Response {{
    complete(
        state,
        "GET",
        request.uri().path(),
        200,
        "application/json",
        "{{\"status\":\"ok\"}}".to_string(),
        request_id(),
        Instant::now(),
    )
}}

async fn readyz(State(state): State<AppState>, request: Request<axum::body::Body>) -> Response {{
    complete(
        state,
        "GET",
        request.uri().path(),
        200,
        "application/json",
        "{{\"ready\":true}}".to_string(),
        request_id(),
        Instant::now(),
    )
}}

async fn metrics(State(state): State<AppState>, request: Request<axum::body::Body>) -> Response {{
    let body = format!(
        "{{{{\"request_total\":{{}},\"error_total\":{{}},\"runtime\":\"corvid-server\"}}}}",
        REQUEST_TOTAL.load(Ordering::Relaxed),
        ERROR_TOTAL.load(Ordering::Relaxed)
    );
    complete(
        state,
        "GET",
        request.uri().path(),
        200,
        "application/json",
        body,
        request_id(),
        Instant::now(),
    )
}}

async fn ops_show(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
) -> Response {{
    let request_id = request_id();
    let started = Instant::now();
    // Fail closed: no signing key configured -> no /__ops
    // surface. A snapshot with no signature is exactly what a
    // MITM would produce, so refuse rather than serve unsigned.
    let signing = match state.ops_signing.as_ref() {{
        Some(s) => s.clone(),
        None => {{
            return error_response(
                state,
                503,
                "GET",
                request.uri().path(),
                "ops_signing_not_configured",
                "CORVID_OPS_SIGNING_KEY is not set; /__ops refuses to serve unsigned snapshots",
                request_id,
                started,
            );
        }}
    }};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let snapshot = OpsShowSnapshot {{
        build_id: state.build_id.as_str().to_string(),
        started_unix_ms: state.started_unix_ms,
        generated_unix_ms: now,
        request_count: REQUEST_TOTAL.load(Ordering::Relaxed),
        claim_manifest_ids: claim_manifest_ids(),
    }};
    let payload = match serde_json::to_vec(&snapshot) {{
        Ok(bytes) => bytes,
        Err(err) => {{
            return error_response(
                state,
                500,
                "GET",
                request.uri().path(),
                "ops_snapshot_serialise_failed",
                &err.to_string(),
                request_id,
                started,
            );
        }}
    }};
    let envelope = sign_dsse(&payload, OPS_SHOW_PAYLOAD_TYPE, &signing.key, &signing.key_id);
    let body = match serde_json::to_string(&envelope) {{
        Ok(s) => s,
        Err(err) => {{
            return error_response(
                state,
                500,
                "GET",
                request.uri().path(),
                "ops_envelope_serialise_failed",
                &err.to_string(),
                request_id,
                started,
            );
        }}
    }};
    complete(
        state,
        "GET",
        request.uri().path(),
        200,
        "application/json",
        body,
        request_id,
        started,
    )
}}

#[derive(Serialize)]
struct OpsShowSnapshot {{
    build_id: String,
    started_unix_ms: u64,
    generated_unix_ms: u64,
    request_count: u64,
    #[serde(default)]
    claim_manifest_ids: Vec<String>,
}}

#[derive(Serialize)]
struct DsseEnvelope {{
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<DsseSignature>,
}}

#[derive(Serialize)]
struct DsseSignature {{
    keyid: String,
    sig: String,
}}

fn sign_dsse(
    payload: &[u8],
    payload_type: &str,
    key: &SigningKey,
    key_id: &str,
) -> DsseEnvelope {{
    let pae = dsse_pae(payload_type, payload);
    let sig = key.sign(&pae);
    let b64 = base64::engine::general_purpose::STANDARD;
    DsseEnvelope {{
        payload_type: payload_type.to_string(),
        payload: b64.encode(payload),
        signatures: vec![DsseSignature {{
            keyid: key_id.to_string(),
            sig: b64.encode(sig.to_bytes()),
        }}],
    }}
}}

fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {{
    // DSSEv1 PAE: `DSSEv1 SP LEN(type) SP type SP LEN(payload)
    // SP payload`. Mirrors corvid-abi::signing::pae byte-for-
    // byte; the build_server integration test asserts the
    // CLI-side verifier accepts the rendered server's output.
    let mut out = Vec::with_capacity(payload.len() + payload_type.len() + 32);
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(payload_type.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload_type.as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload);
    out
}}

fn claim_manifest_ids() -> Vec<String> {{
    // Reserved for future wiring: the rendered binary should
    // emit the claim ids its embedded ABI attestation asserts.
    // For v1.0 the snapshot ships an empty list so the verifier
    // still passes; operators compare `build_id` instead. The
    // claim-id wiring is filed as a sibling launch-readiness
    // (the cdylib-side claim infrastructure already ships; the
    // rendered axum binary does not yet embed it).
    Vec::new()
}}

async fn handle_app(
    State(state): State<AppState>,
    method: Method,
    request: Request<axum::body::Body>,
) -> Response {{
    let started = Instant::now();
    let request_id = request_id();
    let method_text = method.as_str().to_string();
    let path = request.uri().path().to_string();
    let content_length = request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_REQUEST_BYTES {{
        return error_response(
            state,
            413,
            &method_text,
            &path,
            "body_too_large",
            "request exceeds server body limit",
            request_id,
            started,
        );
    }}
    if method != Method::GET {{
        return error_response(
            state,
            405,
            &method_text,
            &path,
            "method_not_allowed",
            "method not allowed",
            request_id,
            started,
        );
    }}
    let output = run_handler(handler_timeout());
    match output {{
        Ok(out) if out.status_success => {{
            let body = out.stdout.trim().to_string();
            let json = format!("{{{{\"result\":{{:?}}}}}}", body);
            complete(state, &method_text, &path, 200, "application/json", json, request_id, started)
        }}
        Ok(out) => {{
            let err = out.stderr.trim().to_string();
            error_response(
                state,
                500,
                &method_text,
                &path,
                "handler_failed",
                if err.is_empty() {{ "handler failed" }} else {{ &err }},
                request_id,
                started,
            )
        }}
        Err(HandlerError::TimedOut) => error_response(
            state,
            504,
            &method_text,
            &path,
            "handler_timeout",
            "handler timed out",
            request_id,
            started,
        ),
        Err(HandlerError::Spawn(err)) => error_response(
            state,
            500,
            &method_text,
            &path,
            "handler_spawn_failed",
            &err,
            request_id,
            started,
        ),
    }}
}}

fn error_response(
    state: AppState,
    status: u16,
    method: &str,
    route: &str,
    kind: &str,
    message: &str,
    request_id: String,
    started: Instant,
) -> Response {{
    let body = format!(
        "{{{{\"request_id\":{{}},\"route\":{{}},\"kind\":{{}},\"message\":{{}},\"duration_ms\":{{}}}}}}",
        json_string(&request_id),
        json_string(route),
        json_string(kind),
        json_string(message),
        started.elapsed().as_millis()
    );
    complete(state, method, route, status, "application/json", body, request_id, started)
}}

fn complete(
    state: AppState,
    method: &str,
    route: &str,
    status: u16,
    content_type: &str,
    body: String,
    request_id: String,
    started: Instant,
) -> Response {{
    REQUEST_TOTAL.fetch_add(1, Ordering::Relaxed);
    if status >= 400 {{
        ERROR_TOTAL.fetch_add(1, Ordering::Relaxed);
    }}
    trace_response(&request_id, method, route, status, started);
    maybe_shutdown(&state);
    let mut response = (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), body).into_response();
    let headers = response.headers_mut();
    headers.insert(axum::http::header::CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap());
    headers.insert("x-corvid-request-id", HeaderValue::from_str(&request_id).unwrap());
    headers.insert(axum::http::header::CONNECTION, HeaderValue::from_static("close"));
    response
}}

fn maybe_shutdown(state: &AppState) {{
    let handled = state.handled_requests.fetch_add(1, Ordering::Relaxed) + 1;
    if matches!(state.max_requests, Some(limit) if handled >= limit) {{
        if let Some(sender) = state.shutdown.lock().unwrap().take() {{
            let _ = sender.send(());
        }}
    }}
}}

fn trace_response(request_id: &str, method: &str, route: &str, status: u16, started: Instant) {{
    eprintln!(
        "{{{{\"event\":\"corvid.server.request\",\"request_id\":{{}},\"method\":{{}},\"route\":{{}},\"status\":{{}},\"duration_ms\":{{}},\"effects\":[]}}}}",
        json_string(request_id),
        json_string(method),
        json_string(route),
        status,
        started.elapsed().as_millis()
    );
}}

struct HandlerOutput {{
    status_success: bool,
    stdout: String,
    stderr: String,
}}

enum HandlerError {{
    Spawn(String),
    TimedOut,
}}

fn run_handler(timeout: Duration) -> Result<HandlerOutput, HandlerError> {{
    if timeout.is_zero() {{
        return Err(HandlerError::TimedOut);
    }}
    let mut child = Command::new(HANDLER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| HandlerError::Spawn(err.to_string()))?;
    let started = Instant::now();
    loop {{
        match child.try_wait() {{
            Ok(Some(status)) => {{
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stdout.take() {{
                    let _ = pipe.read_to_string(&mut stdout);
                }}
                if let Some(mut pipe) = child.stderr.take() {{
                    let _ = pipe.read_to_string(&mut stderr);
                }}
                return Ok(HandlerOutput {{
                    status_success: status.success(),
                    stdout,
                    stderr,
                }});
            }}
            Ok(None) if started.elapsed() >= timeout => {{
                let _ = child.kill();
                let _ = child.wait();
                return Err(HandlerError::TimedOut);
            }}
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(err) => return Err(HandlerError::Spawn(err.to_string())),
        }}
    }}
}}

fn handler_timeout() -> Duration {{
    let millis = std::env::var("CORVID_HANDLER_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30_000);
    Duration::from_millis(millis)
}}

fn max_requests() -> Option<u64> {{
    std::env::var("CORVID_MAX_REQUESTS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|limit| *limit > 0)
}}

fn require_auth() -> bool {{
    std::env::var("CORVID_REQUIRE_AUTH")
        .ok()
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}}

fn rate_limit_requests() -> Option<u64> {{
    std::env::var("CORVID_RATE_LIMIT_REQUESTS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|limit| *limit > 0)
}}

fn csrf_secret() -> Vec<u8> {{
    std::env::var("CORVID_CSRF_SECRET")
        .ok()
        .map(|value| value.into_bytes())
        .unwrap_or_default()
}}

fn build_id() -> String {{
    std::env::var("CORVID_BUILD_ID").unwrap_or_else(|_| "unknown".to_string())
}}

fn load_ops_signing() -> Result<Option<Arc<OpsSigning>>, String> {{
    let Some(raw) = std::env::var("CORVID_OPS_SIGNING_KEY").ok() else {{
        return Ok(None);
    }};
    let trimmed: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let seed = if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {{
        let mut bytes = [0u8; 32];
        for (i, chunk) in trimmed.as_bytes().chunks_exact(2).enumerate() {{
            let hi = hex_nibble(chunk[0])
                .ok_or_else(|| "CORVID_OPS_SIGNING_KEY has non-hex characters".to_string())?;
            let lo = hex_nibble(chunk[1])
                .ok_or_else(|| "CORVID_OPS_SIGNING_KEY has non-hex characters".to_string())?;
            bytes[i] = (hi << 4) | lo;
        }}
        bytes
    }} else if raw.as_bytes().len() == 32 {{
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(raw.as_bytes());
        bytes
    }} else {{
        return Err(format!(
            "CORVID_OPS_SIGNING_KEY must be 64 hex chars or 32 raw bytes (got {{}} chars)",
            raw.len()
        ));
    }};
    let key = SigningKey::from_bytes(&seed);
    let key_id = std::env::var("CORVID_OPS_KEY_ID")
        .unwrap_or_else(|_| "deploy-key".to_string());
    Ok(Some(Arc::new(OpsSigning {{
        key: Arc::new(key),
        key_id: Arc::new(key_id),
    }})))
}}

fn hex_nibble(byte: u8) -> Option<u8> {{
    match byte {{
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }}
}}

fn verify_csrf(
    method: &str,
    headers: &axum::http::HeaderMap,
    secret: &[u8],
) -> Result<(), &'static str> {{
    // Safe methods pass through — GET / HEAD / OPTIONS are not
    // state-changing. Anything else (POST / PUT / PATCH /
    // DELETE / unknown) is treated as state-changing and the
    // double-submit verifier runs. Mirrors
    // `corvid-runtime::auth::csrf::CsrfRequestMethod::classify`.
    if matches!(method, "GET" | "HEAD" | "OPTIONS") {{
        return Ok(());
    }}
    let header_token = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("csrf header missing")?;
    let cookie_token = read_cookie(headers, CSRF_COOKIE).ok_or("csrf cookie missing")?;
    if header_token.as_bytes().ct_eq(cookie_token.as_bytes()).unwrap_u8() == 0 {{
        return Err("csrf header and cookie do not match");
    }}
    let (binding, supplied_hex) = header_token
        .split_once('.')
        .ok_or("csrf token malformed")?;
    if binding.is_empty() || supplied_hex.is_empty() {{
        return Err("csrf token malformed");
    }}
    let supplied = decode_hex(supplied_hex).ok_or("csrf token malformed")?;
    let expected = compute_csrf_hmac(binding, secret).ok_or("csrf secret invalid")?;
    if supplied.ct_eq(&expected).unwrap_u8() == 0 {{
        return Err("csrf token failed hmac verification");
    }}
    Ok(())
}}

fn compute_csrf_hmac(binding: &str, secret: &[u8]) -> Option<Vec<u8>> {{
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(CSRF_BINDING_DOMAIN);
    mac.update(binding.as_bytes());
    Some(mac.finalize().into_bytes().to_vec())
}}

fn read_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {{
    let raw = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for entry in raw.split(';') {{
        let entry = entry.trim();
        if let Some((key, value)) = entry.split_once('=') {{
            if key.trim() == name {{
                let value = value.trim();
                if !value.is_empty() {{
                    return Some(value.to_string());
                }}
            }}
        }}
    }}
    None
}}

fn decode_hex(input: &str) -> Option<Vec<u8>> {{
    if !input.len().is_multiple_of(2) {{
        return None;
    }}
    let mut out = Vec::with_capacity(input.len() / 2);
    for pair in input.as_bytes().chunks_exact(2) {{
        let hi = decode_nibble(pair[0])?;
        let lo = decode_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }}
    Some(out)
}}

fn decode_nibble(byte: u8) -> Option<u8> {{
    match byte {{
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }}
}}

fn validate_runtime_config() -> std::io::Result<()> {{
    if let Ok(port) = std::env::var("CORVID_PORT") {{
        if port.parse::<u16>().is_err() {{
            return Err(invalid_config("CORVID_PORT", "expected integer port 0-65535"));
        }}
    }}
    if let Ok(timeout) = std::env::var("CORVID_HANDLER_TIMEOUT_MS") {{
        if timeout.parse::<u64>().is_err() {{
            return Err(invalid_config("CORVID_HANDLER_TIMEOUT_MS", "expected unsigned integer milliseconds"));
        }}
    }}
    if let Ok(limit) = std::env::var("CORVID_MAX_REQUESTS") {{
        match limit.parse::<u64>() {{
            Ok(value) if value > 0 => {{}}
            _ => return Err(invalid_config("CORVID_MAX_REQUESTS", "expected positive unsigned integer")),
        }}
    }}
    if let Ok(limit) = std::env::var("CORVID_RATE_LIMIT_REQUESTS") {{
        match limit.parse::<u64>() {{
            Ok(value) if value > 0 => {{}}
            _ => return Err(invalid_config("CORVID_RATE_LIMIT_REQUESTS", "expected positive unsigned integer")),
        }}
    }}
    Ok(())
}}

fn invalid_config(name: &str, reason: &str) -> std::io::Error {{
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("backend config {{name}} invalid: {{reason}} (value redacted)"),
    )
}}

fn request_id() -> String {{
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("req-{{now}}-{{counter}}")
}}

fn json_string(value: &str) -> String {{
    format!("{{value:?}}")
}}
"#
    )
}

