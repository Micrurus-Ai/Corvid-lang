use super::{Runtime, RuntimeMode};
use crate::approvals::{Approver, StdinApprover};
use crate::calibration::CalibrationStore;
use crate::errors::RuntimeError;
use crate::db::DbHandleRegistry;
use crate::http::HttpClient;
use crate::human::{HumanInteractor, StdinHumanInteractor};
use crate::http::HttpEgressPolicy;
use crate::io::{IoRuntime, IoToolPolicy};
use crate::llm::{LlmAdapter, LlmRegistry};
use crate::models::{ModelCatalog, RegisteredModel};
use crate::prompt_cache::PromptCache;
use crate::queue::QueueRuntime;
use crate::record::Recorder;
use crate::replay::ReplaySource;
use crate::secrets::SecretRuntime;
use crate::store::StoreManager;
use crate::tools::ToolRegistry;
use crate::tracing::{fresh_run_id, Tracer};
use crate::usage::LlmUsageLedger;
use corvid_trace_schema::WRITER_INTERPRETER;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
pub struct RuntimeBuilder {
    tools: ToolRegistry,
    llms: LlmRegistry,
    approver: Option<Arc<dyn Approver>>,
    human: Option<Arc<dyn HumanInteractor>>,
    tracer: Option<Tracer>,
    pub(super) trace_schema_writer: &'static str,
    default_model: String,
    model_catalog: ModelCatalog,
    model_catalog_root: Option<PathBuf>,
    rollout_seed: Option<u64>,
    pub(super) replay_trace: Option<PathBuf>,
    pub(super) replay_model_swap: Option<String>,
    replay_mutation: Option<(usize, serde_json::Value)>,
    stores: StoreManager,
    /// Slice 33S1a: policy carrying the configured `[io] root`
    /// from `corvid.toml`. Threaded into `Runtime::io_policy`
    /// at build time. Defaults to an unconfigured policy that
    /// makes every executing file-I/O tool fail closed with the
    /// missing-config diagnostic.
    io_policy: IoToolPolicy,
    rag_embedder: Option<std::sync::Arc<dyn crate::rag::RagEmbedder>>,
    mcp: crate::mcp::McpRuntime,
    /// Slice 33S2a: policy carrying the configured `[http] allow`
    /// allowlist from `corvid.toml`. Threaded into
    /// `Runtime::http_policy` at build time. Defaults to an
    /// unconfigured policy that makes every executing HTTP tool
    /// fail closed with the missing-config diagnostic; SSRF
    /// block is always on regardless of allowlist contents.
    http_policy: HttpEgressPolicy,
    /// Slice 33S2b: optional caller-supplied `HttpClient`. `None`
    /// (the default) means `build()` constructs a fresh
    /// `HttpClient::new()` with a standard reqwest backend. End-to-
    /// end tests inject a client built with `reqwest::Client`'s
    /// `.resolve(...)` DNS override so a publicly-named URL routes
    /// at the TCP layer to a loopback wiremock server — see the
    /// docstring on `HttpClient::with_reqwest_client` for why this
    /// is the no-shortcut alternative to a test-only SSRF carve-out.
    http_client_override: Option<HttpClient>,
    /// Slice 52g-3c: the deployment-selected connector execution mode.
    /// `None` means no mode was selected — a program that declares
    /// connectors then refuses to start (the selection is a consequential
    /// choice with no default). Set once at build time and immutable for
    /// the process (`Runtime::connector_mode`).
    connector_mode: Option<corvid_ast::ConnectorMode>,
    /// Slice 52g-3c-4: real-mode connector HTTP dispatch specs, keyed by
    /// the operation's tool name. Built from the IR at startup by the
    /// driver. Empty for programs with no connectors.
    connector_calls: std::collections::HashMap<String, crate::connectors::ConnectorHttpSpec>,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self {
            tools: ToolRegistry::default(),
            llms: LlmRegistry::default(),
            approver: None,
            human: None,
            tracer: None,
            trace_schema_writer: WRITER_INTERPRETER,
            default_model: String::new(),
            model_catalog: ModelCatalog::default(),
            model_catalog_root: None,
            rollout_seed: None,
            replay_trace: None,
            replay_model_swap: None,
            replay_mutation: None,
            stores: StoreManager::default(),
            io_policy: IoToolPolicy::default(),
            rag_embedder: None,
            mcp: crate::mcp::McpRuntime::default(),
            http_policy: HttpEgressPolicy::default(),
            http_client_override: None,
            connector_mode: None,
            connector_calls: std::collections::HashMap::new(),
        }
    }
}

impl RuntimeBuilder {
    pub fn tool<F, Fut>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(Vec<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<serde_json::Value, RuntimeError>> + Send + 'static,
    {
        self.tools.register(name, handler);
        self
    }

    /// Replace the builder's `ToolRegistry` wholesale.
    ///
    /// Use this to share a registry across multiple runtime instances
    /// constructed from the same host configuration — for example, the
    /// `corvid serve` slice 33Q1a wiring builds the main interpreter
    /// runtime AND the `bypass_runtime` that re-executes after `/approve`
    /// from the same set of cdylib-host tool handlers.
    /// `ToolRegistry` is `Clone` (handlers are `Arc<dyn Fn ...>`), so
    /// callers typically clone once per builder.
    pub fn tool_registry(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    /// Register deterministic mock tool handlers from
    /// `CORVID_TEST_MOCK_TOOLS`.
    ///
    /// The env var is a JSON object whose keys are tool names. Each value may
    /// be either a single JSON response or an array of responses consumed in
    /// FIFO order by that tool.
    pub fn env_mock_tools_from_env(mut self) -> Self {
        let Some(map) = std::env::var("CORVID_TEST_MOCK_TOOLS")
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&raw).ok())
        else {
            return self;
        };

        self.register_mock_tool_responses(map);
        self
    }

    fn register_mock_tool_responses(&mut self, map: serde_json::Map<String, serde_json::Value>) {
        for (name, value) in map {
            let queue = match value {
                serde_json::Value::Array(values) => values.into_iter().collect(),
                other => VecDeque::from([other]),
            };
            let responses = Arc::new(Mutex::new(queue));
            let tool_name = name.clone();
            self.tools.register(name, move |_| {
                let responses = Arc::clone(&responses);
                let tool_name = tool_name.clone();
                async move {
                    responses
                        .lock()
                        .unwrap()
                        .pop_front()
                        .ok_or_else(|| RuntimeError::ToolFailed {
                            tool: tool_name,
                            message: "CORVID_TEST_MOCK_TOOLS response queue exhausted".into(),
                        })
                }
            });
        }
    }

    pub fn llm(mut self, adapter: Arc<dyn LlmAdapter>) -> Self {
        self.llms.register(adapter);
        self
    }

    /// Slice 33S1a: install the parsed `[io] root` policy on the
    /// runtime. The executing file-I/O tools (`io_read_text` /
    /// `io_write_text` / `io_list_dir`) resolve every caller-
    /// supplied path through this policy. Default (when this
    /// setter isn't called) is an unconfigured policy that makes
    /// every executing file-I/O call fail closed with the
    /// missing-config diagnostic — the 33S0 security model.
    pub fn io_policy(mut self, policy: IoToolPolicy) -> Self {
        self.io_policy = policy;
        self
    }

    /// Slice 46f: install the configured MCP servers for the
    /// executing `mcp_call` tool. Servers are UNTRUSTED by default
    /// (calls go through the runtime approver); `trusted: true`
    /// mirrors `trust = "autonomous"` in corvid.toml.
    pub fn mcp_servers(
        mut self,
        servers: std::collections::HashMap<String, crate::mcp::McpServerConfig>,
    ) -> Self {
        self.mcp = crate::mcp::McpRuntime::new(servers);
        self
    }

    /// Slice 46g: install the embedding provider for the executing
    /// `rag_ingest` / `rag_search` stdlib tools. Without one,
    /// retrieval degrades honestly to lexical search.
    pub fn rag_embedder(
        mut self,
        embedder: std::sync::Arc<dyn crate::rag::RagEmbedder>,
    ) -> Self {
        self.rag_embedder = Some(embedder);
        self
    }

    /// Slice 33S2a: install the parsed `[http] allow` policy on
    /// the runtime. The executing HTTP tools (`http_get` /
    /// `http_post_json`) check every URL against the policy
    /// (always-on SSRF block + allowlist) before any
    /// `HttpClient::send` runs. Default (when this setter isn't
    /// called) is an unconfigured policy that makes every
    /// executing HTTP call fail closed with the missing-config
    /// diagnostic — the 33S0 security model.
    pub fn http_policy(mut self, policy: HttpEgressPolicy) -> Self {
        self.http_policy = policy;
        self
    }

    /// Slice 52g-3c: select the connector execution mode for this
    /// process. `None` (the default, when this setter isn't called)
    /// means no mode was selected — a program that declares connectors
    /// then refuses to start rather than pick a mode silently. The
    /// selection is immutable once the runtime is built.
    pub fn connector_mode(mut self, mode: Option<corvid_ast::ConnectorMode>) -> Self {
        self.connector_mode = mode;
        self
    }

    /// Slice 52g-3c-4: install the real-mode connector HTTP dispatch
    /// specs (keyed by operation tool name). The driver derives these
    /// from the IR at startup. In mock/replay mode they are unused (mock
    /// evaluates the compiled payload in the VM; replay serves the
    /// recorded interaction), but installing them unconditionally keeps
    /// the wiring uniform.
    pub fn connector_calls(
        mut self,
        calls: std::collections::HashMap<String, crate::connectors::ConnectorHttpSpec>,
    ) -> Self {
        self.connector_calls = calls;
        self
    }

    /// Slice 33S2b: install a caller-supplied `HttpClient` instead
    /// of letting `build()` construct a default one. End-to-end
    /// tests use this to inject a reqwest client with
    /// `.resolve(host, addr)` DNS overrides pointing publicly-named
    /// URLs at a loopback wiremock server. Production callers
    /// generally do NOT call this — the default `HttpClient::new()`
    /// is correct for shipping binaries.
    pub fn http_client(mut self, client: HttpClient) -> Self {
        self.http_client_override = Some(client);
        self
    }

    pub fn approver(mut self, approver: Arc<dyn Approver>) -> Self {
        self.approver = Some(approver);
        self
    }

    pub fn human_interactor(mut self, human: Arc<dyn HumanInteractor>) -> Self {
        self.human = Some(human);
        self
    }

    pub fn tracer(mut self, tracer: Tracer) -> Self {
        self.tracer = Some(tracer);
        self
    }

    pub fn trace_schema_writer(mut self, writer: &'static str) -> Self {
        self.trace_schema_writer = writer;
        self
    }

    /// Open a JSONL trace file under `dir` with a fresh run id.
    pub fn trace_to(self, dir: &Path) -> Self {
        let tracer = Tracer::open(dir, fresh_run_id());
        self.tracer(tracer)
    }

    pub fn default_model(mut self, model: impl Into<String>) -> Self {
        self.default_model = model.into();
        self
    }

    pub fn model(mut self, model: RegisteredModel) -> Self {
        self.model_catalog.register(model);
        self
    }

    pub fn model_catalog(mut self, catalog: ModelCatalog) -> Self {
        self.model_catalog = catalog;
        self
    }

    pub fn model_catalog_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.model_catalog_root = Some(root.into());
        self
    }

    pub fn stores(mut self, stores: StoreManager) -> Self {
        self.stores = stores;
        self
    }

    pub fn sqlite_store(mut self, path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        self.stores = StoreManager::sqlite(path)?;
        Ok(self)
    }

    pub fn rollout_seed(mut self, seed: u64) -> Self {
        self.rollout_seed = Some(seed);
        self
    }

    pub fn replay_from(mut self, path: impl Into<PathBuf>) -> Self {
        self.replay_trace = Some(path.into());
        self
    }

    pub fn replay_model_swap(mut self, model: impl Into<String>) -> Self {
        self.replay_model_swap = Some(model.into());
        self
    }

    pub fn differential_replay_from(
        mut self,
        path: impl Into<PathBuf>,
        model: impl Into<String>,
    ) -> Self {
        self.replay_trace = Some(path.into());
        self.replay_model_swap = Some(model.into());
        self
    }

    pub fn mutation_replay_from(
        mut self,
        path: impl Into<PathBuf>,
        step_1based: usize,
        replacement: serde_json::Value,
    ) -> Self {
        self.replay_trace = Some(path.into());
        self.replay_mutation = Some((step_1based, replacement));
        self
    }

    pub fn build(self) -> Runtime {
        let mut model_catalog = self.model_catalog;
        let model_catalog_error = if model_catalog.is_empty() {
            let start = self
                .model_catalog_root
                .or_else(|| std::env::current_dir().ok());
            match start {
                Some(start) => match ModelCatalog::load_walking(&start) {
                    Ok(Some(loaded)) => {
                        model_catalog.extend(loaded);
                        None
                    }
                    Ok(None) => None,
                    Err(err) => Some(err),
                },
                None => None,
            }
        } else {
            None
        };
        let tracer = self.tracer.unwrap_or_else(Tracer::null);
        let recorder = Recorder::for_tracer(&tracer, self.trace_schema_writer).map(Arc::new);
        let (mode, replay_error, rollout_seed) = if let Some(path) = self.replay_trace {
            let replay_load = if let Some((step_1based, replacement)) = self.replay_mutation {
                ReplaySource::from_path_for_writer_with_mutation(
                    path,
                    self.trace_schema_writer,
                    step_1based,
                    replacement,
                )
            } else if let Some(model) = self.replay_model_swap {
                ReplaySource::from_path_for_writer_with_model(path, self.trace_schema_writer, model)
            } else {
                ReplaySource::from_path_for_writer(path, self.trace_schema_writer)
            };
            match replay_load {
                Ok(source) => (
                    RuntimeMode::Replay(source.clone()),
                    None,
                    source.initial_rollout_seed(),
                ),
                Err(err) => (
                    RuntimeMode::Live,
                    Some(err),
                    self.rollout_seed.unwrap_or_else(crate::tracing::now_ms),
                ),
            }
        } else {
            (
                RuntimeMode::Live,
                None,
                self.rollout_seed.unwrap_or_else(crate::tracing::now_ms),
            )
        };
        if let Some(recorder) = &recorder {
            recorder.emit_schema_header();
            recorder.emit_seed_read("rollout_default_seed", rollout_seed);
        }
        // Slices 35V2-P38-C-4 / C-5: quarantine every side-effect
        // surface when entering a Substitute-mode replay (the default
        // for `corvid replay` and `corvid jobs replay`). Differential
        // mode keeps live adapters / clients — its whole purpose is
        // to compare recorded output against live calls. Mutation
        // mode also keeps live access because the mutation produces
        // a counterfactual that may legitimately reach the registry.
        //
        // Surfaces:
        // - C-4: `LlmRegistry::quarantine_all` wraps every adapter
        //   so direct registry calls refuse with `QuarantineViolation`.
        // - C-5 HTTP: `HttpClient::quarantine` flag short-circuits
        //   `send`. Connector / tool HTTP calls during replay are
        //   blocked.
        // - C-5 Store: `StoreManager::quarantine_writes` short-
        //   circuits `put` / `put_record` / `put_record_if_revision`
        //   / `delete` / `delete_with_policy`. Reads pass through.
        //   The durable job queue uses raw `rusqlite` and is
        //   unaffected.
        // - C-5 IO: `IoRuntime::quarantine_writes` short-circuits
        //   `write_text*`. Reads pass through. Trace emission uses
        //   `JsonlTraceWriter` directly and is unaffected.
        let mut llms = self.llms;
        let mut http = self.http_client_override.unwrap_or_else(HttpClient::new);
        let mut stores = self.stores;
        let mut io = IoRuntime::new();
        let db_registry = DbHandleRegistry::new();
        if let RuntimeMode::Replay(source) = &mode {
            if !source.uses_live_llm() {
                llms.quarantine_all();
                http.quarantine();
                stores.quarantine_writes();
                io.quarantine_writes();
                // Slice 33S3b — flip the SQLite write-quarantine.
                // `Runtime::db_execute_tool` then refuses with
                // `QuarantineViolation { surface: "db", .. }`.
                // Reads (`db_query_tool`) pass through; the
                // dispatch-level substitution path is the upper
                // gate (33S3c integrates the trace-substitution
                // layer alongside the io / http precedents).
                db_registry.quarantine_writes();
            }
        }
        Runtime {
            tools: self.tools,
            llms,
            approver: self
                .approver
                .unwrap_or_else(|| Arc::new(StdinApprover::new())),
            human: self
                .human
                .unwrap_or_else(|| Arc::new(StdinHumanInteractor::new())),
            tracer,
            recorder,
            mode,
            replay_error,
            default_model: self.default_model,
            model_catalog,
            model_catalog_error,
            rollout_state: Arc::new(AtomicU64::new(rollout_seed)),
            calibration: CalibrationStore::default(),
            prompt_cache: PromptCache::default(),
            stores,
            usage_ledger: LlmUsageLedger::new(),
            http,
            http_policy: self.http_policy,
            io,
            io_policy: self.io_policy,
            rag_embedder: self.rag_embedder,
            mcp: self.mcp,
            db_registry,
            secrets: SecretRuntime::new(),
            cache: std::sync::Arc::new(std::sync::Mutex::new(
                crate::cache::CacheRuntime::new(),
            )),
            queue: QueueRuntime::new(),
            connector_mode: self.connector_mode,
            connector_calls: std::sync::Arc::new(self.connector_calls),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_tool_responses_are_registered_and_consumed_fifo() {
        let mut map = serde_json::Map::new();
        map.insert("lookup".to_string(), json!(["first", "second"]));

        let mut builder = Runtime::builder();
        builder.register_mock_tool_responses(map);
        let runtime = builder.build();

        let first = runtime.tools().call("lookup", vec![]).await.unwrap();
        let second = runtime.tools().call("lookup", vec![]).await.unwrap();
        assert_eq!(first, json!("first"));
        assert_eq!(second, json!("second"));
    }

    #[tokio::test]
    async fn mock_tool_response_queue_exhaustion_is_explicit() {
        let mut map = serde_json::Map::new();
        map.insert("lookup".to_string(), json!("only"));

        let mut builder = Runtime::builder();
        builder.register_mock_tool_responses(map);
        let runtime = builder.build();

        runtime.tools().call("lookup", vec![]).await.unwrap();
        let err = runtime.tools().call("lookup", vec![]).await.unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::ToolFailed { ref tool, ref message }
                if tool == "lookup" && message.contains("response queue exhausted")
        ));
    }
}
