use crate::errors::RuntimeError;
pub use corvid_ast::RedirectPolicy;
use sha2::{Digest, Sha256};
use serde::Serialize;
use std::time::{Duration, Instant};

/// The most redirects one request follows, under any policy. A fixed
/// structural bound like the SSRF block, not a per-program setting: the
/// declared policy decides *where* a redirect may go, and no legitimate
/// API chains more hops than this.
pub const MAX_REDIRECT_HOPS: u32 = 10;

/// Headers that carry a credential under every scheme. A request never
/// sends them to an origin other than the one it started at, and a
/// request's own credential headers (`HttpRequest::credential_headers`)
/// are dropped with them.
const ALWAYS_CREDENTIAL_HEADERS: [&str; 3] = ["authorization", "cookie", "proxy-authorization"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRetryPolicy {
    pub max_retries: u32,
    pub retry_on_5xx: bool,
}

impl Default for HttpRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_on_5xx: true,
        }
    }
}

/// One outbound HTTP request. Its timeout and redirect policy are
/// constructor arguments, so no request can be built without both: the
/// runtime holds no default for either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<HttpHeader>,
    pub body: Option<String>,
    /// The longest one attempt may take, from connecting to the last
    /// byte of the response, redirects included.
    pub timeout_ms: u64,
    pub redirects: RedirectPolicy,
    /// Lower-cased names of the headers in `headers` that carry this
    /// request's credential (a connector's `header("X-Api-Key", ...)`
    /// auth). They are never sent to another origin, and never enter a
    /// fingerprint or a record.
    pub credential_headers: Vec<String>,
    pub retry: HttpRetryPolicy,
    pub effect_tag: Option<String>,
}

impl HttpRequest {
    pub fn new(
        method: impl Into<String>,
        url: impl Into<String>,
        timeout: Duration,
        redirects: RedirectPolicy,
    ) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
            redirects,
            credential_headers: Vec::new(),
            retry: HttpRetryPolicy::default(),
            effect_tag: None,
        }
    }

    /// Send `body` as JSON.
    pub fn json_body(self, body: impl Into<String>) -> Self {
        self.body("application/json", body)
    }

    /// Send `body` with the given content type.
    pub fn body(mut self, content_type: &str, body: impl Into<String>) -> Self {
        self.headers.push(HttpHeader {
            name: "content-type".to_string(),
            value: content_type.to_string(),
        });
        self.body = Some(body.into());
        self
    }

    pub fn retry(mut self, retry: HttpRetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(HttpHeader {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    /// Attach a header that carries the request's credential. It is sent
    /// only to the origin the request starts at.
    pub fn credential_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        self.credential_headers.push(name.to_ascii_lowercase());
        self.header(name, value)
    }

    pub fn effect_tag(mut self, effect_tag: impl Into<String>) -> Self {
        self.effect_tag = Some(effect_tag.into());
        self
    }

    fn is_credential_header(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        ALWAYS_CREDENTIAL_HEADERS.contains(&name.as_str())
            || self.credential_headers.iter().any(|credential| credential == &name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<HttpHeader>,
    pub body: String,
    pub attempts: u32,
    pub elapsed_ms: u64,
}

/// A response opened for streaming: its status and headers, and the
/// body still to read. Dropping it closes the connection.
pub struct OpenedStream {
    pub status: u16,
    pub headers: Vec<HttpHeader>,
    pub response: reqwest::Response,
    /// The request's declared timeout, which bounds the wait for each
    /// event once the stream is open.
    pub timeout: Duration,
}

/// How one hop of one attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpExchangeOutcome {
    /// A response arrived and is the answer.
    Response,
    /// A redirect arrived and was followed.
    Redirected,
    /// The declared timeout expired with this hop in flight.
    Timeout,
    /// A response arrived, but the declared policy refused where it led.
    Refused,
    /// The connection or the body failed.
    TransportError,
}

impl HttpExchangeOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::Redirected => "redirected",
            Self::Timeout => "timeout",
            Self::Refused => "refused",
            Self::TransportError => "transport_error",
        }
    }
}

/// What one hop of one attempt did, reported to the caller of
/// [`HttpClient::send_observed`] so it can be recorded. It holds nothing
/// credential-shaped: headers by name only, the request body by its
/// SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpExchangeObservation {
    /// 1 for the first attempt, 2 for the first retry, and so on.
    pub attempt: u32,
    /// 0 for the request as sent, 1 for the first redirect followed.
    pub hop: u32,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<String>,
    pub request_body_sha256: Option<String>,
    pub request_body_bytes: u64,
    pub status: Option<u16>,
    pub response_headers: Vec<String>,
    pub response_body_bytes: u64,
    pub elapsed_ms: u64,
    pub outcome: HttpExchangeOutcome,
    pub detail: Option<String>,
}

/// The receiver of every exchange a send makes.
pub type HttpExchangeObserver<'a> = &'a (dyn Fn(HttpExchangeObservation) + Send + Sync);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedHttpExchange {
    pub request_fingerprint: String,
    pub method: String,
    pub url: String,
    pub status: u16,
    pub attempts: u32,
    pub effect_tag: Option<String>,
    pub response_body: String,
}

/// The HTTP transport every outbound request goes through.
///
/// Redirects are followed here, not by reqwest: the underlying client
/// never follows one itself, so the request's declared
/// [`RedirectPolicy`], the egress policy and the rule that a credential
/// never crosses to another origin apply to every hop.
#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    /// The project's `[http] allow` list and SSRF block, applied to
    /// every redirect under `redirects: allowed_hosts`. The first hop is
    /// checked by the caller, which reports the refusal as its own.
    egress: HttpEgressPolicy,
    /// Slice `35V2-P38-C-5`: set to `true` by
    /// `RuntimeBuilder::build` when entering Substitute-mode replay.
    /// `send` short-circuits with `RuntimeError::QuarantineViolation`
    /// so a replayed run cannot reach the network even if some caller
    /// bypasses the runtime's trace-substitution path.
    quarantined: bool,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::with_reqwest_builder(reqwest::Client::builder())
    }
}

/// Why one attempt failed, and whether trying again could help. A
/// refusal (a redirect the policy forbids) is final; a transport error
/// or a timeout may be retried.
struct AttemptFailure {
    message: String,
    retryable: bool,
}

impl AttemptFailure {
    fn transient(message: String) -> Self {
        Self { message, retryable: true }
    }

    fn refused(message: String) -> Self {
        Self { message, retryable: false }
    }

    fn into_error(self) -> RuntimeError {
        RuntimeError::ToolFailed {
            tool: "std.http".to_string(),
            message: self.message,
        }
    }
}

impl HttpClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the transport from a caller-configured reqwest builder.
    /// Production code uses [`HttpClient::new`]; tests pass a builder
    /// with `.resolve(host, addr)` DNS overrides so a publicly named URL
    /// (`http://api.example.com/`) reaches a loopback wiremock server
    /// while the egress policy still sees the public name. Automatic
    /// redirects are always turned off here, whatever the builder says,
    /// because this client follows redirects itself.
    pub fn with_reqwest_builder(builder: reqwest::ClientBuilder) -> Self {
        Self {
            client: builder
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client builds"),
            egress: HttpEgressPolicy::unset(),
            quarantined: false,
        }
    }

    /// Install the project's egress policy, used for redirects under
    /// `redirects: allowed_hosts`. Called by `RuntimeBuilder::build`.
    pub fn set_egress_policy(&mut self, policy: HttpEgressPolicy) {
        self.egress = policy;
    }

    /// Flip into replay-quarantine mode. Subsequent `send` calls fail
    /// closed with `RuntimeError::QuarantineViolation { surface:
    /// "http", .. }` instead of reaching the network. Called by
    /// `RuntimeBuilder::build` when entering Substitute-mode replay.
    pub fn quarantine(&mut self) {
        self.quarantined = true;
    }

    /// True when this client refuses live HTTP calls. Test helper.
    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }

    pub async fn send(&self, request: &HttpRequest) -> Result<HttpResponse, RuntimeError> {
        self.send_observed(request, &|_| {}).await
    }

    /// [`Self::send`], reporting every hop of every attempt to `observe`.
    pub async fn send_observed(
        &self,
        request: &HttpRequest,
        observe: HttpExchangeObserver<'_>,
    ) -> Result<HttpResponse, RuntimeError> {
        if self.quarantined {
            return Err(RuntimeError::QuarantineViolation {
                surface: "http".to_string(),
                detail: format!(
                    "blocked an unrecorded {} call to `{}` during replay-mode quarantine. \
                     A replayed run must not reach the network; if the connector layer needs \
                     this request, the trace did not record an equivalent exchange.",
                    request.method, request.url
                ),
            });
        }
        let started = Instant::now();
        let attempts_allowed = request.retry.max_retries.saturating_add(1);
        let mut attempt = 0;
        let mut last_error = None;
        while attempt < attempts_allowed {
            attempt += 1;
            match self.attempt(request, attempt, observe).await {
                Ok((status, headers, body)) => {
                    let should_retry =
                        request.retry.retry_on_5xx && status >= 500 && attempt < attempts_allowed;
                    if should_retry {
                        last_error = Some(format!("HTTP {status}"));
                        continue;
                    }
                    return Ok(HttpResponse {
                        status,
                        headers,
                        body,
                        attempts: attempt,
                        elapsed_ms: elapsed_ms(started),
                    });
                }
                Err(failure) if failure.retryable && attempt < attempts_allowed => {
                    last_error = Some(failure.message);
                }
                Err(failure) => return Err(failure.into_error()),
            }
        }
        Err(RuntimeError::ToolFailed {
            tool: "std.http".to_string(),
            message: last_error.unwrap_or_else(|| "HTTP request failed".to_string()),
        })
    }

    pub async fn send_recorded(
        &self,
        request: &HttpRequest,
    ) -> Result<(HttpResponse, RecordedHttpExchange), RuntimeError> {
        let response = self.send(request).await?;
        let record = record_exchange(request, &response);
        Ok((response, record))
    }

    /// Open a response to read as a stream: send the request and follow
    /// redirects as [`Self::send_observed`] does, under the request's
    /// timeout for opening, and return it unread. Retries happen only
    /// before the first byte, so no event is ever delivered twice. The
    /// exchange is recorded when the response opens, with the body
    /// size unknown (0); reading it is the caller's.
    pub async fn open_stream(
        &self,
        request: &HttpRequest,
        observe: HttpExchangeObserver<'_>,
    ) -> Result<OpenedStream, RuntimeError> {
        if self.quarantined {
            return Err(RuntimeError::QuarantineViolation {
                surface: "http".to_string(),
                detail: format!(
                    "blocked an unrecorded streamed {} call to `{}` during replay-mode quarantine",
                    request.method, request.url
                ),
            });
        }
        let attempts_allowed = request.retry.max_retries.saturating_add(1);
        let mut last_error = None;
        for attempt in 1..=attempts_allowed {
            let in_flight = std::sync::Mutex::new(HopInFlight {
                hop: 0,
                url: request.url.clone(),
                headers: request.headers.clone(),
                body: request.body.clone(),
                started: Instant::now(),
            });
            let exchange = ExchangeRecorder { request, attempt, observe, in_flight: &in_flight };
            let timeout = Duration::from_millis(request.timeout_ms);
            match tokio::time::timeout(timeout, self.follow_redirects(request, &exchange)).await {
                Ok(Ok((response, headers))) => {
                    let status = response.status().as_u16();
                    if request.retry.retry_on_5xx && status >= 500 && attempt < attempts_allowed {
                        exchange.finish(Some(status), &headers, 0, HttpExchangeOutcome::Response, None);
                        last_error = Some(format!("HTTP {status}"));
                        continue;
                    }
                    exchange.finish(Some(status), &headers, 0, HttpExchangeOutcome::Response, None);
                    return Ok(OpenedStream { status, headers, response, timeout });
                }
                Ok(Err(failure)) if failure.retryable && attempt < attempts_allowed => {
                    last_error = Some(failure.message);
                }
                Ok(Err(failure)) => return Err(failure.into_error()),
                Err(_) => {
                    let message = format!(
                        "{} `{}` did not open within its declared timeout of {}",
                        request.method,
                        request.url,
                        describe_timeout(request.timeout_ms)
                    );
                    exchange.timed_out(&message);
                    if attempt < attempts_allowed {
                        last_error = Some(message);
                    } else {
                        return Err(AttemptFailure::transient(message).into_error());
                    }
                }
            }
        }
        Err(RuntimeError::ToolFailed {
            tool: "std.http".to_string(),
            message: last_error.unwrap_or_else(|| "HTTP request failed".to_string()),
        })
    }

    /// One attempt, bounded as a whole by the request's timeout:
    /// connecting, every redirect hop, and reading the full body.
    async fn attempt(
        &self,
        request: &HttpRequest,
        attempt: u32,
        observe: HttpExchangeObserver<'_>,
    ) -> Result<(u16, Vec<HttpHeader>, String), AttemptFailure> {
        let timeout = Duration::from_millis(request.timeout_ms);
        // The hop in flight, so a timeout that cancels the redirect loop
        // can still be recorded against the request it interrupted.
        let in_flight = std::sync::Mutex::new(HopInFlight {
            hop: 0,
            url: request.url.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            started: Instant::now(),
        });
        let exchange = ExchangeRecorder { request, attempt, observe, in_flight: &in_flight };
        let read = async {
            let (response, received_headers) = self.follow_redirects(request, &exchange).await?;
            let status = response.status().as_u16();
            match response.text().await {
                Ok(text) => {
                    exchange.finish(Some(status), &received_headers, text.len() as u64, HttpExchangeOutcome::Response, None);
                    Ok((status, received_headers, text))
                }
                Err(err) => {
                    let failure = AttemptFailure::transient(format!("failed to read HTTP response body: {err}"));
                    exchange.finish(Some(status), &received_headers, 0, HttpExchangeOutcome::TransportError, Some(&failure.message));
                    Err(failure)
                }
            }
        };
        match tokio::time::timeout(timeout, read).await {
            Ok(result) => result,
            Err(_) => {
                let message = format!(
                    "{} `{}` did not finish within its declared timeout of {}",
                    request.method,
                    request.url,
                    describe_timeout(request.timeout_ms)
                );
                exchange.timed_out(&message);
                Err(AttemptFailure::transient(message))
            }
        }
    }

    /// Send the request and follow redirects under its policy, returning
    /// the final response still unread, with its headers. Every hop but
    /// the last is recorded here; the caller records the last once it
    /// knows how much body it read.
    async fn follow_redirects(
        &self,
        request: &HttpRequest,
        exchange: &ExchangeRecorder<'_>,
    ) -> Result<(reqwest::Response, Vec<HttpHeader>), AttemptFailure> {
        let method = request.method.parse::<reqwest::Method>().map_err(|err| {
            AttemptFailure::refused(format!("invalid HTTP method `{}`: {err}", request.method))
        })?;
        let mut url = reqwest::Url::parse(&request.url).map_err(|err| {
            AttemptFailure::refused(format!("invalid request URL `{}`: {err}", request.url))
        })?;
        let mut headers = request.headers.clone();
        let mut body = request.body.clone();
        let mut hops = 0;
        loop {
            exchange.start_hop(hops, &url, &headers, &body);
            let mut builder = self.client.request(method.clone(), url.clone());
            for header in &headers {
                builder = builder.header(&header.name, &header.value);
            }
            if let Some(body) = &body {
                builder = builder.body(body.clone());
            }
            let response = match builder.send().await {
                Ok(response) => response,
                Err(err) => {
                    let failure = AttemptFailure::transient(err.to_string());
                    exchange.finish(None, &[], 0, HttpExchangeOutcome::TransportError, Some(&failure.message));
                    return Err(failure);
                }
            };
            let status = response.status().as_u16();
            let received_headers = response_headers(response.headers());
            let location = redirect_location(&response);
            let Some(location) = location.filter(|_| request.redirects != RedirectPolicy::Never)
            else {
                return Ok((response, received_headers));
            };
            // Every refusal below is recorded against the hop whose
            // response led to it.
            let refuse = |message: String| {
                exchange.finish(Some(status), &received_headers, 0, HttpExchangeOutcome::Refused, Some(&message));
                AttemptFailure::refused(message)
            };
            let next = url.join(&location).map_err(|err| {
                refuse(format!(
                    "`{url}` answered {status} with a Location `{location}` that is not a URL: {err}"
                ))
            })?;
            if hops == MAX_REDIRECT_HOPS {
                return Err(refuse(format!(
                    "{} `{}` was redirected more than {MAX_REDIRECT_HOPS} times; the last hop pointed at `{next}`",
                    request.method, request.url
                )));
            }
            hops += 1;
            let changes_method = matches!(status, 301 | 302 | 303)
                && method != reqwest::Method::GET
                && method != reqwest::Method::HEAD;
            if changes_method {
                return Err(refuse(format!(
                    "{method} `{url}` was answered {status}, which would re-send it to `{next}` as a GET \
                     without its body; Corvid refuses rather than change the request's method. \
                     A provider that moves a {method} endpoint answers 307 or 308"
                )));
            }
            let leaves_origin = origin(&next) != origin(&url);
            match request.redirects {
                RedirectPolicy::Never => unreachable!("a `never` policy returns the 3xx above"),
                RedirectPolicy::SameHost if leaves_origin => {
                    return Err(refuse(format!(
                        "`{url}` redirected to `{next}`, another host, and the connector declares \
                         `redirects: same_host`"
                    )));
                }
                RedirectPolicy::SameHost => {}
                RedirectPolicy::AllowedHosts => {
                    if let Err(err) = self.egress.check(next.as_str()) {
                        return Err(refuse(format!(
                            "`{url}` redirected to `{next}`, which the project's HTTP policy refuses: {}",
                            policy_message(err)
                        )));
                    }
                }
            }
            exchange.finish(Some(status), &received_headers, 0, HttpExchangeOutcome::Redirected, None);
            if leaves_origin {
                headers.retain(|header| !request.is_credential_header(&header.name));
            }
            if status == 303 {
                // A 303 after a GET or HEAD: fetch the other resource
                // with the same method and no body.
                body = None;
            }
            url = next;
        }
    }
}

/// The hop an attempt is sending, kept so a timeout can name it.
struct HopInFlight {
    hop: u32,
    url: String,
    headers: Vec<HttpHeader>,
    body: Option<String>,
    started: Instant,
}

/// Turns each hop of one attempt into an [`HttpExchangeObservation`].
struct ExchangeRecorder<'a> {
    request: &'a HttpRequest,
    attempt: u32,
    observe: HttpExchangeObserver<'a>,
    in_flight: &'a std::sync::Mutex<HopInFlight>,
}

impl ExchangeRecorder<'_> {
    fn start_hop(&self, hop: u32, url: &reqwest::Url, headers: &[HttpHeader], body: &Option<String>) {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *in_flight = HopInFlight {
            hop,
            url: url.to_string(),
            headers: headers.to_vec(),
            body: body.clone(),
            started: Instant::now(),
        };
    }

    fn finish(
        &self,
        status: Option<u16>,
        response_headers: &[HttpHeader],
        response_body_bytes: u64,
        outcome: HttpExchangeOutcome,
        detail: Option<&str>,
    ) {
        let in_flight = self.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        (self.observe)(HttpExchangeObservation {
            attempt: self.attempt,
            hop: in_flight.hop,
            method: self.request.method.clone(),
            url: in_flight.url.clone(),
            request_headers: in_flight.headers.iter().map(|h| h.name.to_ascii_lowercase()).collect(),
            request_body_sha256: in_flight.body.as_ref().map(|body| {
                encode_hex(&Sha256::digest(body.as_bytes()))
            }),
            request_body_bytes: in_flight.body.as_ref().map_or(0, |body| body.len() as u64),
            status,
            response_headers: response_headers.iter().map(|h| h.name.to_ascii_lowercase()).collect(),
            response_body_bytes,
            elapsed_ms: elapsed_ms(in_flight.started),
            outcome,
            detail: detail.map(str::to_string),
        });
    }

    fn timed_out(&self, message: &str) {
        self.finish(None, &[], 0, HttpExchangeOutcome::Timeout, Some(message));
    }
}

/// The `Location` of a response that asks to be followed, if any. Only
/// 301, 302, 303, 307 and 308 redirect; a 300 or 304 is an answer.
fn redirect_location(response: &reqwest::Response) -> Option<String> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn origin(url: &reqwest::Url) -> (String, Option<String>, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().map(str::to_ascii_lowercase),
        url.port_or_known_default(),
    )
}

fn policy_message(error: RuntimeError) -> String {
    match error {
        RuntimeError::ToolFailed { message, .. } => message,
        other => other.to_string(),
    }
}

fn describe_timeout(timeout_ms: u64) -> String {
    if timeout_ms % 1000 == 0 {
        format!("{}s", timeout_ms / 1000)
    } else {
        format!("{timeout_ms}ms")
    }
}

fn response_headers(headers: &reqwest::header::HeaderMap) -> Vec<HttpHeader> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value.to_str().ok().map(|value| HttpHeader {
                name: name.as_str().to_string(),
                value: value.to_string(),
            })
        })
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub fn record_exchange(request: &HttpRequest, response: &HttpResponse) -> RecordedHttpExchange {
    RecordedHttpExchange {
        request_fingerprint: request_fingerprint(request),
        method: request.method.clone(),
        url: request.url.clone(),
        status: response.status,
        attempts: response.attempts,
        effect_tag: request.effect_tag.clone(),
        response_body: response.body.clone(),
    }
}

/// A SHA-256 of everything that defines the request. A credential header
/// contributes its name only, so the fingerprint is the same for every
/// key and reveals nothing about one.
pub fn request_fingerprint(request: &HttpRequest) -> String {
    let mut headers: Vec<HttpHeader> = request
        .headers
        .iter()
        .map(|header| HttpHeader {
            name: header.name.clone(),
            value: if request.is_credential_header(&header.name) {
                crate::REDACTED_SECRET.to_string()
            } else {
                header.value.clone()
            },
        })
        .collect();
    headers.sort_by(|left, right| left.name.cmp(&right.name).then(left.value.cmp(&right.value)));
    let canonical = serde_json::json!({
        "method": request.method,
        "url": request.url,
        "headers": headers,
        "body": request.body,
        "timeout_ms": request.timeout_ms,
        "redirects": request.redirects.as_str(),
        "retry": {
            "max_retries": request.retry.max_retries,
            "retry_on_5xx": request.retry.retry_on_5xx,
        },
        "effect_tag": request.effect_tag,
    });
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string().as_bytes());
    encode_hex(&hasher.finalize())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

// ============================================================================
// Phase 33S2a — HttpEgressPolicy.
//
// Every outbound request a connector sends carries its security
// boundary in this policy struct. The `Runtime` holds one
// `HttpEgressPolicy`; the connector dispatcher runs each request URL
// through `check` before `HttpClient::send` runs, the redirect loop
// runs every `allowed_hosts` hop through it, and startup runs each
// `base_url` through it in `real` mode. (Until B05 slice 4 the
// `http_get` / `http_post_json` tools were checked here too.)
//
// The policy enforces TWO independent properties:
//
//   1. SSRF block — **always on, never configurable**. Any URL
//      whose host resolves to a private RFC1918 range
//      (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16), loopback
//      (127.0.0.0/8 + ::1), or link-local (169.254.0.0/16 +
//      fe80::/10) is refused regardless of allowlist contents.
//      This is a STRUCTURAL property — there is no way for a
//      program to opt out without modifying this code.
//
//   2. Allowlist — `[http] allow = [...]` in corvid.toml (or
//      the `CORVID_HTTP_ALLOW` env override). REQUIRED: when
//      the list is empty, every request fails closed with the
//      missing-config diagnostic.
//
// Composition: a URL passes only when BOTH the SSRF check
// passes AND its host appears verbatim (case-insensitive) in
// the allowlist. Either failure mode emits a structured
// diagnostic that names the URL, the failing check, and the
// configured allowlist (when relevant) so the operator can
// see why the call was refused.
// ============================================================================

/// Phase 33S2c — anchor for the structural SSRF-block guarantee.
/// `HttpEgressPolicy::check` is the enforcement site: every
/// executing HTTP call parses the URL's host and refuses any
/// private RFC1918 / loopback / link-local / unspecified / ULA /
/// `localhost` literal — regardless of allowlist contents. The
/// `corvid-guarantees` inverse-coverage sentinel uses this anchor
/// to confirm the registry row is wired to the enforcement code.
pub const GUARANTEE_ID_IO_SOURCE_HTTP_SSRF_STRUCTURAL_BLOCK: &str =
    "io_source.http_ssrf_structural_block";

/// Phase 33S2c — anchor for the allowlist-enforcement guarantee.
/// `HttpEgressPolicy::check` requires the URL's host to appear
/// (case-insensitive) in the configured `[http] allow` list.
/// When no allowlist is configured (missing section, empty list,
/// or unset env), every call fails closed with a structured
/// diagnostic naming the URL, the missing config, and the
/// `CORVID_HTTP_ALLOW` env override pathway.
pub const GUARANTEE_ID_IO_SOURCE_HTTP_ALLOWLIST_ENFORCEMENT: &str =
    "io_source.http_allowlist_enforcement";

/// Phase 33S2c — anchor for the replay-quarantine guarantee.
/// `HttpClient::send` returns QuarantineViolation when the
/// `quarantined` flag is set (the floor); a connector operation's
/// call goes through the replay-substitution path FIRST, so it
/// either substitutes from the recorded trace OR diverges — it never
/// reaches the live network.
pub const GUARANTEE_ID_IO_SOURCE_HTTP_QUARANTINE_ON_REPLAY: &str =
    "io_source.http_quarantine_on_replay";

/// Policy carrying the configured `[http] allow` list for the
/// executing HTTP-client surface plus the always-on SSRF block.
/// Construct via `HttpEgressPolicy::new` once per `Runtime` and
/// store on the runtime; the `http_*` tool dispatch interception
/// calls `check` per request.
#[derive(Debug, Clone, Default)]
pub struct HttpEgressPolicy {
    /// Lower-cased host strings allowed for egress. `None` and
    /// `Some(empty)` both mean unconfigured — every `check`
    /// call fails closed. `Some(non-empty)` is the active
    /// allowlist; case-insensitive host comparison.
    allow: Option<Vec<String>>,
}

impl HttpEgressPolicy {
    /// Build a policy from the parsed `[http] allow` value. An
    /// empty input list is treated the same as `None` (the
    /// fail-closed default). Each entry is lower-cased so host
    /// comparison in `check` is case-insensitive.
    pub fn new(allow_list: Option<&[String]>) -> Self {
        let allow = allow_list.and_then(|list| {
            if list.is_empty() {
                None
            } else {
                Some(list.iter().map(|s| s.to_ascii_lowercase()).collect())
            }
        });
        Self { allow }
    }

    /// Empty policy — no `[http] allow` configured. Every
    /// `check` call returns the missing-config error.
    pub fn unset() -> Self {
        Self { allow: None }
    }

    /// True when the policy has a non-empty allowlist.
    pub fn is_configured(&self) -> bool {
        self.allow.as_ref().is_some_and(|v| !v.is_empty())
    }

    /// Return the configured allowlist for inspection (used by
    /// `corvid doctor`-style introspection). Empty when
    /// unconfigured.
    pub fn allow_list(&self) -> &[String] {
        self.allow.as_deref().unwrap_or(&[])
    }

    /// Check a request URL against (1) the always-on SSRF
    /// block and (2) the configured allowlist. Returns the host
    /// string on success (for tracing) or a structured
    /// `RuntimeError` naming the violation.
    pub fn check(&self, url: &str) -> Result<String, RuntimeError> {
        let host = parse_host_lowercase(url).ok_or_else(|| RuntimeError::ToolFailed {
            tool: "http".to_string(),
            message: format!(
                "URL `{url}` could not be parsed for SSRF + allowlist check; \
                 the executing HTTP surface requires an absolute http(s):// URL"
            ),
        })?;

        // SSRF check first — always on, regardless of
        // allowlist contents. Blocks loopback + RFC1918 +
        // link-local. Names that ALSO appear in the allowlist
        // still get refused (the SSRF block is the floor).
        if is_ssrf_blocked_host(&host) {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` is refused by Corvid's structural SSRF block: \
                     host `{host}` resolves to a private / loopback / link-local \
                     address range, which is never reachable through the \
                     executing HTTP surface regardless of `[http] allow` \
                     contents. SSRF protection is a structural property of \
                     the language, not a configurable setting."
                ),
            });
        }

        // Allowlist check second. Empty allowlist = fail closed
        // with the missing-config diagnostic.
        let Some(allow) = &self.allow else {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` cannot be reached: no `[http] allow` list is \
                     configured in this project's corvid.toml. Add the host to \
                     `[http]\\nallow = [\"...\"]` (or set CORVID_HTTP_ALLOW) to \
                     scope executing HTTP egress. The executing HTTP surface \
                     fails closed without an explicit allowlist — this is the \
                     33S0 security model."
                ),
            });
        };
        if !allow.iter().any(|h| h == &host) {
            return Err(RuntimeError::ToolFailed {
                tool: "http".to_string(),
                message: format!(
                    "URL `{url}` is refused: host `{host}` is not in this \
                     project's `[http] allow` allowlist ({}). Add it to the \
                     list or change the request URL.",
                    if allow.is_empty() {
                        "<empty>".to_string()
                    } else {
                        format!("[{}]", allow.join(", "))
                    }
                ),
            });
        }
        Ok(host)
    }
}

/// Extract the host part of a URL and lower-case it. Returns
/// `None` for inputs that don't carry an http(s) scheme or
/// have no host segment. Used by `HttpEgressPolicy::check` to
/// drive both the SSRF check and the allowlist comparison.
fn parse_host_lowercase(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // Use the transport's normalized host for both IPv4 and IPv6 policy checks.
    Some(url.host_str()?.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Classify a host string as one Corvid will refuse on SSRF
/// grounds. Covers:
///   * Literal IPv4 in private / loopback / link-local ranges
///     (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8,
///     169.254.0.0/16).
///   * Literal IPv6 loopback `::1` and link-local `fe80::/10`.
///   * Hostname `localhost` (DNS-shaped loopback alias).
///
/// Hosts that don't fit any of these patterns pass through and
/// land in the allowlist check next.
fn is_ssrf_blocked_host(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    // IPv4 literal?
    if let Ok(ipv4) = host.parse::<std::net::Ipv4Addr>() {
        let octets = ipv4.octets();
        // 10.0.0.0/8
        if octets[0] == 10 {
            return true;
        }
        // 172.16.0.0/12
        if octets[0] == 172 && (16..=31).contains(&octets[1]) {
            return true;
        }
        // 192.168.0.0/16
        if octets[0] == 192 && octets[1] == 168 {
            return true;
        }
        // 127.0.0.0/8 — loopback
        if octets[0] == 127 {
            return true;
        }
        // 169.254.0.0/16 — link-local
        if octets[0] == 169 && octets[1] == 254 {
            return true;
        }
        // 0.0.0.0/8 — "any" / unspecified (treated as loopback
        // for egress purposes; binding to 0.0.0.0 means
        // every-local-interface, which includes loopback).
        if octets[0] == 0 {
            return true;
        }
        return false;
    }
    // IPv6 literal? Handle the bracketed form already stripped
    // by parse_host_lowercase — host arrives without `[]`.
    if let Ok(ipv6) = host.parse::<std::net::Ipv6Addr>() {
        if ipv6.is_loopback() {
            return true;
        }
        // Link-local fe80::/10.
        if ipv6.segments()[0] & 0xffc0 == 0xfe80 {
            return true;
        }
        // Unique-local fc00::/7.
        if ipv6.segments()[0] & 0xfe00 == 0xfc00 {
            return true;
        }
        // Unspecified ::.
        if ipv6.is_unspecified() {
            return true;
        }
        return false;
    }
    // Plain hostname (not a literal IP) — let it through to
    // the allowlist check. Production-grade SSRF would also
    // resolve the hostname via DNS and re-check the resolved
    // IPs; that adds a DNS round-trip on every call AND is
    // racy (the DNS answer can change between check and
    // connect). v1.0 ships with literal-IP coverage; richer
    // DNS-resolution-time SSRF is a follow-up slice.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn get(url: impl Into<String>) -> HttpRequest {
        HttpRequest::new("GET", url, Duration::from_secs(5), RedirectPolicy::Never)
    }

    /// Slice 35V2-P38-C-5: a quarantined `HttpClient` short-circuits
    /// `send` with `RuntimeError::QuarantineViolation { surface:
    /// "http", .. }` before reaching the network. No mock server is
    /// needed — the test asserts the request never gets out.
    #[tokio::test]
    async fn quarantined_http_client_refuses_send_with_typed_violation() {
        let mut client = HttpClient::new();
        client.quarantine();
        assert!(client.is_quarantined());
        let req = get("https://example.invalid/should-not-reach");
        let err = client.send(&req).await.expect_err("quarantine must error");
        match err {
            RuntimeError::QuarantineViolation { surface, detail } => {
                assert_eq!(surface, "http");
                assert!(
                    detail.contains("example.invalid/should-not-reach"),
                    "detail should name the URL: {detail}"
                );
                assert!(
                    detail.contains("GET"),
                    "detail should name the method: {detail}"
                );
            }
            other => panic!("expected http QuarantineViolation, got {other:?}"),
        }
    }

    /// Slice 35V2-P38-C-5: a default (non-quarantined) `HttpClient`
    /// continues to function — the flag defaults to false and the
    /// existing tests below all rely on it.
    #[tokio::test]
    async fn default_http_client_is_not_quarantined() {
        let client = HttpClient::new();
        assert!(!client.is_quarantined());
    }

    #[tokio::test]
    async fn http_client_gets_text_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hello"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello"))
            .mount(&server)
            .await;

        let response = HttpClient::new()
            .send(&get(format!("{}/hello", server.uri())))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "hello");
        assert_eq!(response.attempts, 1);
    }

    #[tokio::test]
    async fn http_client_retries_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flaky"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let response = HttpClient::new()
            .send(
                &get(format!("{}/flaky", server.uri())).retry(HttpRetryPolicy {
                    max_retries: 1,
                    retry_on_5xx: true,
                }),
            )
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "ok");
        assert_eq!(response.attempts, 2);
    }

    #[tokio::test]
    async fn http_client_records_exchange_with_effect_tag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/recorded"))
            .respond_with(ResponseTemplate::new(200).set_body_string("captured"))
            .mount(&server)
            .await;

        let request = get(format!("{}/recorded", server.uri())).effect_tag("network:http");
        let (response, record) = HttpClient::new().send_recorded(&request).await.unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(record.status, 200);
        assert_eq!(record.effect_tag.as_deref(), Some("network:http"));
        assert_eq!(record.response_body, "captured");
        assert_eq!(record.request_fingerprint.len(), 64);
    }

    // ------------ Declared redirect policy and timeouts ------------

    /// A client that reaches each named host's wiremock server, so the
    /// egress policy sees public names while the bytes stay on loopback.
    fn resolving_client(hosts: &[(&str, &MockServer)], allow: &[&str]) -> HttpClient {
        let mut builder = reqwest::Client::builder();
        for (host, server) in hosts {
            builder = builder.resolve(host, *server.address());
        }
        let mut client = HttpClient::with_reqwest_builder(builder);
        let allow: Vec<String> = allow.iter().map(|h| h.to_string()).collect();
        client.set_egress_policy(HttpEgressPolicy::new(Some(&allow)));
        client
    }

    fn url_on(host: &str, server: &MockServer, path: &str) -> String {
        format!("http://{host}:{}{path}", server.address().port())
    }

    fn request(method: &str, url: String, redirects: RedirectPolicy) -> HttpRequest {
        HttpRequest::new(method, url, Duration::from_secs(5), redirects)
    }

    async fn redirect_to(server: &MockServer, from: &str, status: u16, location: String) {
        Mock::given(path(from))
            .respond_with(ResponseTemplate::new(status).insert_header("location", location.as_str()))
            .mount(server)
            .await;
    }

    async fn answer(server: &MockServer, at: &str, body: &str) {
        Mock::given(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn never_returns_the_redirect_as_the_answer() {
        let server = MockServer::start().await;
        let target = url_on("api.example.com", &server, "/moved");
        redirect_to(&server, "/old", 302, target).await;
        answer(&server, "/moved", "followed").await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);

        let response = client
            .send(&request("GET", url_on("api.example.com", &server, "/old"), RedirectPolicy::Never))
            .await
            .unwrap();

        assert_eq!(response.status, 302);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "`never` must not follow the redirect");
    }

    #[tokio::test]
    async fn same_host_follows_a_redirect_within_the_origin() {
        let server = MockServer::start().await;
        redirect_to(&server, "/old", 301, "/new".to_string()).await;
        answer(&server, "/new", "here").await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);

        let response = client
            .send(&request("GET", url_on("api.example.com", &server, "/old"), RedirectPolicy::SameHost))
            .await
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, "here");
    }

    #[tokio::test]
    async fn same_host_refuses_a_redirect_to_another_host() {
        let origin = MockServer::start().await;
        let other = MockServer::start().await;
        redirect_to(&origin, "/old", 302, url_on("cdn.example.com", &other, "/file")).await;
        answer(&other, "/file", "elsewhere").await;
        let client = resolving_client(
            &[("api.example.com", &origin), ("cdn.example.com", &other)],
            &["api.example.com", "cdn.example.com"],
        );

        let err = client
            .send(&request("GET", url_on("api.example.com", &origin, "/old"), RedirectPolicy::SameHost))
            .await
            .expect_err("a cross-host redirect must be refused under same_host");

        let message = err.to_string();
        assert!(message.contains("redirects: same_host"), "{message}");
        assert!(message.contains("cdn.example.com"), "{message}");
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    /// The credential stays with the origin it was meant for: a redirect
    /// to another allowed host drops `Authorization` and the connector's
    /// own credential header, and keeps the rest.
    #[tokio::test]
    async fn allowed_hosts_follows_to_an_allowed_host_without_the_credential() {
        let origin = MockServer::start().await;
        let other = MockServer::start().await;
        redirect_to(&origin, "/old", 307, url_on("cdn.example.com", &other, "/file")).await;
        answer(&other, "/file", "from the cdn").await;
        let client = resolving_client(
            &[("api.example.com", &origin), ("cdn.example.com", &other)],
            &["api.example.com", "cdn.example.com"],
        );
        let req = request("GET", url_on("api.example.com", &origin, "/old"), RedirectPolicy::AllowedHosts)
            .credential_header("authorization", "Bearer sk_live_secret")
            .credential_header("X-Api-Key", "key_secret")
            .header("x-request-id", "req-1");

        let response = client.send(&req).await.unwrap();

        assert_eq!(response.body, "from the cdn");
        let first = &origin.received_requests().await.unwrap()[0];
        assert!(first.headers.get("x-api-key").is_some(), "the origin receives the key");
        let forwarded = &other.received_requests().await.unwrap()[0];
        assert!(forwarded.headers.get("authorization").is_none());
        assert!(forwarded.headers.get("x-api-key").is_none());
        assert_eq!(forwarded.headers.get("x-request-id").unwrap(), "req-1");
    }

    #[tokio::test]
    async fn allowed_hosts_refuses_a_host_outside_the_allow_list() {
        let origin = MockServer::start().await;
        let other = MockServer::start().await;
        redirect_to(&origin, "/old", 302, url_on("evil.example.org", &other, "/steal")).await;
        let client = resolving_client(
            &[("api.example.com", &origin), ("evil.example.org", &other)],
            &["api.example.com"],
        );

        let err = client
            .send(&request("GET", url_on("api.example.com", &origin, "/old"), RedirectPolicy::AllowedHosts))
            .await
            .expect_err("an unlisted redirect host must be refused");

        assert!(err.to_string().contains("evil.example.org"), "{err}");
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    /// A 302 to a POST would make most clients re-send it as a body-less
    /// GET. Corvid refuses instead; a 307 keeps the method and the body.
    #[tokio::test]
    async fn a_post_is_never_silently_turned_into_a_get() {
        let server = MockServer::start().await;
        redirect_to(&server, "/found", 302, "/elsewhere".to_string()).await;
        redirect_to(&server, "/temporary", 307, "/kept".to_string()).await;
        Mock::given(method("POST"))
            .and(path("/kept"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .mount(&server)
            .await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);

        let err = client
            .send(&request("POST", url_on("api.example.com", &server, "/found"), RedirectPolicy::SameHost).json_body("{}"))
            .await
            .expect_err("a 302 to a POST must be refused");
        assert!(err.to_string().contains("as a GET"), "{err}");

        let response = client
            .send(&request("POST", url_on("api.example.com", &server, "/temporary"), RedirectPolicy::SameHost).json_body("{\"a\":1}"))
            .await
            .unwrap();
        assert_eq!(response.status, 201);
        let kept = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path() == "/kept")
            .unwrap();
        assert_eq!(kept.method.as_str(), "POST");
        assert_eq!(kept.body, b"{\"a\":1}");
    }

    #[tokio::test]
    async fn a_redirect_loop_stops_at_the_hop_limit() {
        let server = MockServer::start().await;
        redirect_to(&server, "/loop", 302, "/loop".to_string()).await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);

        let err = client
            .send(&request("GET", url_on("api.example.com", &server, "/loop"), RedirectPolicy::SameHost))
            .await
            .expect_err("a redirect loop must stop");

        assert!(err.to_string().contains(&format!("more than {MAX_REDIRECT_HOPS} times")), "{err}");
        assert_eq!(server.received_requests().await.unwrap().len(), MAX_REDIRECT_HOPS as usize + 1);
    }

    /// The timeout bounds the whole exchange, body included, and names
    /// the declared value.
    #[tokio::test]
    async fn the_declared_timeout_bounds_a_slow_response() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);
        let req = HttpRequest::new(
            "GET",
            url_on("api.example.com", &server, "/slow"),
            Duration::from_millis(300),
            RedirectPolicy::Never,
        );

        let started = Instant::now();
        let err = client.send(&req).await.expect_err("the slow response must time out");

        assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
        assert!(err.to_string().contains("declared timeout of 300ms"), "{err}");
    }

    /// Collects what an observed send reports.
    fn observe_into(
        seen: &std::sync::Mutex<Vec<HttpExchangeObservation>>,
    ) -> impl Fn(HttpExchangeObservation) + Send + Sync + '_ {
        move |observation| seen.lock().unwrap().push(observation)
    }

    /// A followed redirect and its answer are two exchanges, numbered by
    /// hop; a retried 5xx is a second attempt. Headers are reported by
    /// name, and no credential value reaches an observation.
    #[tokio::test]
    async fn every_hop_of_every_attempt_is_observed_without_credentials() {
        let server = MockServer::start().await;
        Mock::given(path("/a"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        redirect_to(&server, "/a", 302, "/b".to_string()).await;
        answer(&server, "/b", "done").await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);
        let req = request("GET", url_on("api.example.com", &server, "/a"), RedirectPolicy::SameHost)
            .retry(HttpRetryPolicy { max_retries: 1, retry_on_5xx: true })
            .credential_header("authorization", "Bearer sk_observed_secret");

        let seen = std::sync::Mutex::new(Vec::new());
        let response = client.send_observed(&req, &observe_into(&seen)).await.unwrap();

        assert_eq!(response.body, "done");
        let seen = seen.into_inner().unwrap();
        let summary: Vec<_> = seen.iter().map(|o| (o.attempt, o.hop, o.status, o.outcome)).collect();
        assert_eq!(
            summary,
            vec![
                (1, 0, Some(503), HttpExchangeOutcome::Response),
                (2, 0, Some(302), HttpExchangeOutcome::Redirected),
                (2, 1, Some(200), HttpExchangeOutcome::Response),
            ]
        );
        assert!(seen[2].url.ends_with("/b"));
        assert_eq!(seen[2].response_body_bytes, 4);
        assert!(seen[0].request_headers.contains(&"authorization".to_string()));
        assert!(!format!("{seen:?}").contains("sk_observed_secret"));
    }

    /// A timeout and a refused redirect are each recorded against the hop
    /// they ended, with the reason.
    #[tokio::test]
    async fn a_timeout_and_a_refusal_are_observed_with_their_reason() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(3)))
            .mount(&server)
            .await;
        redirect_to(&server, "/away", 302, "https://elsewhere.example.org/x".to_string()).await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);

        let seen = std::sync::Mutex::new(Vec::new());
        let slow = HttpRequest::new(
            "GET",
            url_on("api.example.com", &server, "/slow"),
            Duration::from_millis(200),
            RedirectPolicy::Never,
        );
        client.send_observed(&slow, &observe_into(&seen)).await.expect_err("times out");
        let away = request("GET", url_on("api.example.com", &server, "/away"), RedirectPolicy::SameHost);
        client.send_observed(&away, &observe_into(&seen)).await.expect_err("refused");

        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].outcome, HttpExchangeOutcome::Timeout);
        assert_eq!(seen[0].status, None);
        assert!(seen[0].detail.as_deref().unwrap().contains("declared timeout of 200ms"));
        assert_eq!(seen[1].outcome, HttpExchangeOutcome::Refused);
        assert_eq!(seen[1].status, Some(302));
        assert!(seen[1].detail.as_deref().unwrap().contains("redirects: same_host"));
    }

    /// The request body is recorded by its SHA-256 and size, never as text.
    #[tokio::test]
    async fn the_request_body_is_observed_by_digest_only() {
        let server = MockServer::start().await;
        answer(&server, "/submit", "ok").await;
        let client = resolving_client(&[("api.example.com", &server)], &["api.example.com"]);
        let body = r#"{"card":"4242424242424242"}"#;
        let req = request("PUT", url_on("api.example.com", &server, "/submit"), RedirectPolicy::Never)
            .json_body(body);

        let seen = std::sync::Mutex::new(Vec::new());
        client.send_observed(&req, &observe_into(&seen)).await.unwrap();

        let seen = seen.into_inner().unwrap();
        assert_eq!(seen[0].request_body_bytes, body.len() as u64);
        assert_eq!(
            seen[0].request_body_sha256.as_deref(),
            Some(encode_hex(&Sha256::digest(body.as_bytes())).as_str())
        );
        assert!(!format!("{seen:?}").contains("4242424242424242"));
    }

    #[test]
    fn the_fingerprint_never_depends_on_a_credential() {
        let with_key = |key: &str| {
            request("GET", "https://api.example.com/x".to_string(), RedirectPolicy::Never)
                .credential_header("X-Api-Key", key)
        };
        assert_eq!(request_fingerprint(&with_key("first")), request_fingerprint(&with_key("second")));
        let plain = |id: &str| {
            request("GET", "https://api.example.com/x".to_string(), RedirectPolicy::Never)
                .header("x-request-id", id)
        };
        assert_ne!(request_fingerprint(&plain("a")), request_fingerprint(&plain("b")));
    }

    // ------------ Slice 33S2a: HttpEgressPolicy plumbing tests ------------

    /// 33S2a — Unconfigured policy fails closed: every `check`
    /// returns the missing-config diagnostic naming `[http] allow`
    /// and the 33S0 security model.
    #[test]
    fn http_egress_policy_unconfigured_fails_closed_on_check() {
        let policy = HttpEgressPolicy::unset();
        assert!(!policy.is_configured());
        assert!(policy.allow_list().is_empty());
        let err = policy
            .check("https://api.example.com/v1/health")
            .expect_err("unconfigured policy must fail closed");
        match err {
            RuntimeError::ToolFailed { tool, message } => {
                assert_eq!(tool, "http");
                assert!(
                    message.contains("[http] allow"),
                    "diagnostic must name [http] allow; got {message}"
                );
                assert!(
                    message.contains("33S0"),
                    "diagnostic must reference the 33S0 security model; got {message}"
                );
            }
            other => panic!("expected ToolFailed, got {other:?}"),
        }
    }

    /// 33S2a — Empty allowlist is the same as None: still fails
    /// closed. Operators who declare `[http] allow = []`
    /// explicitly get the same diagnostic.
    #[test]
    fn http_egress_policy_empty_allow_list_is_unconfigured() {
        let policy = HttpEgressPolicy::new(Some(&[]));
        assert!(!policy.is_configured());
        let err = policy
            .check("https://api.example.com/v1/health")
            .expect_err("empty allowlist must fail closed");
        assert!(matches!(err, RuntimeError::ToolFailed { .. }));
    }

    /// 33S2a — Configured allowlist permits matching hosts.
    /// Host comparison is case-insensitive (the policy lower-
    /// cases all entries at construction).
    #[test]
    fn http_egress_policy_allowlist_permits_matching_host() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        assert!(policy.is_configured());
        let host = policy
            .check("https://api.example.com/v1/health")
            .expect("allowed host must pass");
        assert_eq!(host, "api.example.com");

        // Case-insensitive: caller URL has uppercase host.
        let host = policy
            .check("https://API.Example.COM/v1/health")
            .expect("case-insensitive match");
        assert_eq!(host, "api.example.com");
    }

    /// 33S2a — Configured allowlist refuses non-matching hosts
    /// with a diagnostic that names the offending host AND the
    /// full allowlist so the operator can see both.
    #[test]
    fn http_egress_policy_allowlist_refuses_unlisted_host_with_clear_diagnostic() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        let err = policy
            .check("https://untrusted.example.org/data")
            .expect_err("unlisted host must be refused");
        let msg = format!("{err}");
        assert!(
            msg.contains("untrusted.example.org"),
            "diagnostic must name the refused host; got {msg}"
        );
        assert!(
            msg.contains("api.example.com"),
            "diagnostic must list the allowlist; got {msg}"
        );
    }

    /// 33S2a — SSRF block fires for all IPv4 private + loopback
    /// + link-local ranges, regardless of allowlist contents.
    /// The diagnostic names the violation as "structural" and
    /// "not a configurable setting" — load-bearing language for
    /// the security model.
    #[test]
    fn http_egress_policy_ssrf_block_refuses_all_private_loopback_ipv4_ranges() {
        let allow = vec![
            "10.0.0.1".to_string(),
            "127.0.0.1".to_string(),
            "192.168.1.1".to_string(),
            "169.254.169.254".to_string(),
            "0.0.0.0".to_string(),
        ];
        let policy = HttpEgressPolicy::new(Some(&allow));

        for url in [
            "http://10.0.0.1/admin",
            "http://10.255.255.255/admin",
            "http://172.16.0.1/admin",
            "http://172.31.255.255/admin",
            "http://192.168.1.1/admin",
            "http://127.0.0.1:8080/secret",
            "http://169.254.169.254/aws/instance-meta",
            "http://0.0.0.0/admin",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("SSRF block must refuse {url}"));
            let msg = format!("{err}");
            assert!(
                msg.contains("structural SSRF block"),
                "URL {url}: diagnostic must name structural SSRF block; got {msg}"
            );
            assert!(
                msg.contains("not a configurable setting"),
                "URL {url}: diagnostic must say SSRF is not configurable; got {msg}"
            );
        }
    }

    /// 33S2a — SSRF block also fires for IPv6 loopback (`::1`),
    /// link-local (`fe80::/10`), unique-local (`fc00::/7`), and
    /// unspecified (`::`).
    #[test]
    fn http_egress_policy_ssrf_block_refuses_ipv6_loopback_and_link_local() {
        let policy = HttpEgressPolicy::new(Some(&["[::1]".to_string()]));
        // The parser strips the `[]` brackets — both forms hit
        // the SSRF check.
        for url in [
            "http://[::1]/admin",
            "http://[fe80::1]/admin",
            "http://[fc00::1]/admin",
            "http://[::]/admin",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("IPv6 SSRF must refuse {url}"));
            assert!(
                format!("{err}").contains("structural SSRF block"),
                "URL {url}: expected SSRF block, got {err}"
            );
        }
    }

    /// 33S2a — `localhost` (DNS-shaped loopback alias) is also
    /// SSRF-blocked, NOT just literal IPs. A reviewer who tries
    /// `http://localhost:8080` gets refused without an explicit
    /// IP literal.
    #[test]
    fn http_egress_policy_ssrf_block_refuses_localhost_dns_alias() {
        let allow = vec!["localhost".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        let err = policy
            .check("http://localhost:8080/data")
            .expect_err("localhost must be refused even if explicitly allowed");
        assert!(format!("{err}").contains("structural SSRF block"));
    }

    /// 33S2a — Non-http(s) URLs fail with a structured error
    /// that says the request URL must be `http(s)://...`. Guards
    /// against `file://`, `ftp://`, `gopher://`, etc.
    #[test]
    fn http_egress_policy_rejects_non_http_schemes() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        for url in [
            "file:///etc/passwd",
            "ftp://api.example.com/",
            "gopher://api.example.com/0/",
            "javascript:alert(1)",
            "",
        ] {
            let err = policy
                .check(url)
                .expect_err(&format!("non-http URL {url:?} must be rejected"));
            assert!(format!("{err}").contains("absolute http(s):// URL"));
        }
    }

    /// 33S2a — URL with `user:pass@host` userinfo strips
    /// correctly: the policy compares the bare host, not the
    /// userinfo-prefixed authority. Prevents authority-confusion
    /// SSRF where a request like
    /// `http://api.example.com@evil.com/x` would otherwise be
    /// mis-categorized.
    #[test]
    fn http_egress_policy_strips_userinfo_and_port_before_host_comparison() {
        let allow = vec!["api.example.com".to_string()];
        let policy = HttpEgressPolicy::new(Some(&allow));
        // Authority is `evil.com` — the userinfo-prefixed form
        // must NOT be treated as `api.example.com`.
        let err = policy
            .check("http://api.example.com@evil.com/")
            .expect_err("authority-confusion URL must be refused");
        assert!(
            format!("{err}").contains("evil.com"),
            "diagnostic must name the actual authority host"
        );

        // Port suffix is stripped: `api.example.com:8443` still
        // matches the allowlist entry `api.example.com`.
        let host = policy
            .check("https://api.example.com:8443/data")
            .expect("port suffix must not block allowlist match");
        assert_eq!(host, "api.example.com");
    }
}
