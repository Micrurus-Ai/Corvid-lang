//! Tokio runtime construction + handle borrow, plus the
//! `Runtime` builders both `corvid_runtime_init` variants drive.
//!
//! `build_tokio_runtime` produces the multi-thread tokio runtime
//! the bridge stores behind `BridgeState`. `build_corvid_runtime`
//! is the live-adapter init that picks providers from
//! `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GEMINI_API_KEY` /
//! `OLLAMA_*` env vars; `build_embedded_corvid_runtime` is the
//! deny-everything cdylib variant. `tokio_handle()` is the `pub
//! fn` `#[tool]` wrappers and the Cranelift codegen call to
//! reach into the active tokio runtime.

use std::path::PathBuf;
use std::sync::Arc;

use crate::approvals::{ProgrammaticApprover, StdinApprover};
use crate::llm::anthropic::AnthropicAdapter;
use crate::llm::gemini::GeminiAdapter;
use crate::llm::mock::EnvVarMockAdapter;
use crate::llm::ollama::OllamaAdapter;
use crate::llm::openai::OpenAiAdapter;
use crate::llm::openai_compat::OpenAiCompatibleAdapter;
use crate::redact::RedactionSet;
use crate::runtime::{Runtime, RuntimeBuilder};
use crate::tracing::{fresh_run_id, Tracer};
use corvid_trace_schema::WRITER_NATIVE;

use super::state::bridge;
use super::trace_path_from_env;

/// Tokio handle the `#[tool]` wrappers block_on. Panics if
/// `corvid_runtime_init` hasn't run — matches the eager-init contract.
pub fn tokio_handle() -> tokio::runtime::Handle {
    bridge().tokio_handle()
}
pub(super) fn build_tokio_runtime() -> tokio::runtime::Runtime {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if let Ok(n) = std::env::var("CORVID_TOKIO_WORKERS") {
        if let Ok(parsed) = n.parse::<usize>() {
            if parsed > 0 {
                builder.worker_threads(parsed);
            }
        }
    }
    builder
        .build()
        .expect("construct multi-thread tokio runtime")
}

pub(super) fn build_corvid_runtime() -> Runtime {
    let mut b: RuntimeBuilder = Runtime::builder()
        .tracer(build_tracer_from_env_or_default())
        .trace_schema_writer(WRITER_NATIVE);

    // Approver: interactive stdin by default; programmatic-yes if the
    // user has opted into auto-approve (useful for batch / CI runs).
    if std::env::var("CORVID_APPROVE_AUTO").ok().as_deref() == Some("1") {
        b = b.approver(Arc::new(ProgrammaticApprover::always_yes()));
    } else {
        b = b.approver(Arc::new(StdinApprover::new()));
    }

    if let Ok(model) = std::env::var("CORVID_MODEL") {
        b = b.default_model(&model);
    }
    if let Ok(seed) = std::env::var("CORVID_ROLLOUT_SEED") {
        if let Ok(parsed) = seed.parse::<u64>() {
            b = b.rollout_seed(parsed);
        }
    }
    if let Some(path) = std::env::var_os("CORVID_REPLAY_TRACE_PATH") {
        if let (Some(step), Some(replacement)) = (
            std::env::var_os("CORVID_REPLAY_MUTATE_STEP"),
            std::env::var_os("CORVID_REPLAY_MUTATE_JSON"),
        ) {
            let step = step
                .to_string_lossy()
                .parse::<usize>()
                .expect("CORVID_REPLAY_MUTATE_STEP must be a positive integer");
            let replacement: serde_json::Value = serde_json::from_str(&replacement.to_string_lossy())
                .expect("CORVID_REPLAY_MUTATE_JSON must contain valid JSON");
            b = b.mutation_replay_from(PathBuf::from(path), step, replacement);
        } else if let Some(model) = std::env::var_os("CORVID_REPLAY_MODEL") {
            b = b.differential_replay_from(PathBuf::from(path), model.to_string_lossy().into_owned());
        } else {
            b = b.replay_from(PathBuf::from(path));
        }
    }

    // Register every supported LLM adapter unconditionally
    // so the model-prefix dispatch in `LlmRegistry::call` can route
    // any `CORVID_MODEL` to its provider. Adapters that need an API
    // key fall back to an empty string when the env var is missing —
    // calls then surface as `HTTP 401` from the provider, which is
    // a clearer failure than silently routing nowhere.
    //
    // Test-mode env-var mock takes PRECEDENCE: when
    // `CORVID_TEST_MOCK_LLM=1`, the mock handles every model spec
    // (its `handles` returns true unconditionally), avoiding real
    // API calls in CI even when keys leak into the env.
    if std::env::var("CORVID_TEST_MOCK_LLM").ok().as_deref() == Some("1") {
        b = b.llm(Arc::new(EnvVarMockAdapter::from_env()));
    }
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(AnthropicAdapter::new(anthropic_key)));
    let openai_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(OpenAiAdapter::new(openai_key)));
    let gemini_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_KEY"))
        .unwrap_or_default();
    b = b.llm(Arc::new(GeminiAdapter::new(gemini_key)));
    // Ollama is local, no key. OpenAI-compat key is optional.
    b = b.llm(Arc::new(OllamaAdapter::new()));
    let compat_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(OpenAiCompatibleAdapter::new(compat_key)));

    // Test-only mock-tool registration. Format:
    //   CORVID_TEST_MOCK_INT_TOOLS="name1:value1;name2:value2"
    //
    // Each name becomes a tool that ignores its args and returns the
    // given Int. Used by the parity harness to exercise the compiled
    // tool-call path before the user-facing proc-macro
    // registry. Not a production feature — users never set this env
    // var, and nothing in the driver surfaces it.
    if let Ok(spec) = std::env::var("CORVID_TEST_MOCK_INT_TOOLS") {
        for pair in spec.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let Some((name, value_str)) = pair.split_once(':') else {
                eprintln!(
                    "corvid: malformed CORVID_TEST_MOCK_INT_TOOLS entry `{pair}` (expected `name:value`); skipping"
                );
                continue;
            };
            let Ok(value) = value_str.trim().parse::<i64>() else {
                eprintln!(
                    "corvid: CORVID_TEST_MOCK_INT_TOOLS value `{value_str}` for `{name}` isn't a valid i64; skipping"
                );
                continue;
            };
            let name_owned = name.trim().to_string();
            b = b.tool(name_owned, move |_args| async move {
                Ok(serde_json::json!(value))
            });
        }
    }

    b = install_connectors(b);
    b.build()
}

/// The connector specs the compiled program embeds, installed by its
/// `main` through [`corvid_register_connectors`] before the runtime is
/// built.
static REGISTERED_CONNECTORS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Install the connector specs a native program embeds (native dispatch
/// N1). Called by the generated `main` before `corvid_runtime_init`.
///
/// # Safety
/// `specs` must be a live Corvid string holding the JSON the compiler
/// wrote with `connectors::connector_calls_json`.
#[no_mangle]
pub unsafe extern "C" fn corvid_register_connectors(specs: crate::abi::CorvidString) {
    let text = unsafe { specs.as_str() }.to_string();
    let _ = REGISTERED_CONNECTORS.set(text);
}

/// Install the embedded connectors on `b`, with the mode and HTTP policy
/// the environment names. A program with connectors refuses to start
/// without a mode, and in `real` mode without an `[http] allow` list,
/// as the interpreter does: neither is ever picked for the program.
fn install_connectors(mut b: crate::runtime::RuntimeBuilder) -> crate::runtime::RuntimeBuilder {
    let Some(text) = REGISTERED_CONNECTORS.get() else { return b };
    let specs = crate::connectors::connector_calls_from_json(text).unwrap_or_else(|reason| {
        panic!("internal compiler error: {reason}")
    });
    let refuse = |message: String| -> ! {
        eprintln!("{message}");
        std::process::exit(1);
    };
    let mode = match std::env::var("CORVID_CONNECTOR_MODE") {
        Ok(name) => corvid_ast::ConnectorMode::from_name(name.trim()).unwrap_or_else(|| {
            refuse(format!("corvid: CORVID_CONNECTOR_MODE is `{name}`; it is one of `mock`, `replay` or `real`"))
        }),
        Err(_) => refuse(
            "corvid: this program calls connectors, so it needs a mode: set CORVID_CONNECTOR_MODE to `real` or `replay` (there is no default)"
                .to_string(),
        ),
    };
    if mode == corvid_ast::ConnectorMode::Mock {
        refuse("corvid: mock mode evaluates Corvid mock payloads; run this program with `corvid run --mode mock` on the interpreter".to_string());
    }
    let policy = std::env::var("CORVID_HTTP_ALLOW")
        .ok()
        .and_then(|text| crate::http::HttpEgressPolicy::from_allow_list(&text));
    match policy {
        Some(policy) => b = b.http_policy(policy),
        None if mode == corvid_ast::ConnectorMode::Real => refuse(
            "corvid: this program calls connectors in `real` mode, so it needs the hosts it may reach: set CORVID_HTTP_ALLOW to a comma-separated host list"
                .to_string(),
        ),
        None => {}
    }
    b.connector_mode(Some(mode)).connector_calls(specs)
}

pub(super) fn build_embedded_corvid_runtime() -> Runtime {
    let mut b = Runtime::builder()
        .tracer(build_tracer_from_env_or_default())
        .trace_schema_writer(WRITER_NATIVE)
        .approver(Arc::new(ProgrammaticApprover::new(|req| {
            crate::catalog_c_api::decide_registered_approval(req)
        })));

    if let Ok(model) = std::env::var("CORVID_MODEL") {
        b = b.default_model(&model);
    }
    if let Ok(seed) = std::env::var("CORVID_ROLLOUT_SEED") {
        if let Ok(parsed) = seed.parse::<u64>() {
            b = b.rollout_seed(parsed);
        }
    }
    if let Some(path) = std::env::var_os("CORVID_REPLAY_TRACE_PATH") {
        if let (Some(step), Some(replacement)) = (
            std::env::var_os("CORVID_REPLAY_MUTATE_STEP"),
            std::env::var_os("CORVID_REPLAY_MUTATE_JSON"),
        ) {
            let step = step
                .to_string_lossy()
                .parse::<usize>()
                .expect("CORVID_REPLAY_MUTATE_STEP must be a positive integer");
            let replacement: serde_json::Value = serde_json::from_str(&replacement.to_string_lossy())
                .expect("CORVID_REPLAY_MUTATE_JSON must contain valid JSON");
            b = b.mutation_replay_from(PathBuf::from(path), step, replacement);
        } else if let Some(model) = std::env::var_os("CORVID_REPLAY_MODEL") {
            b = b.differential_replay_from(PathBuf::from(path), model.to_string_lossy().into_owned());
        } else {
            b = b.replay_from(PathBuf::from(path));
        }
    }
    if std::env::var("CORVID_TEST_MOCK_LLM").ok().as_deref() == Some("1") {
        b = b.llm(Arc::new(EnvVarMockAdapter::from_env()));
    }
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(AnthropicAdapter::new(anthropic_key)));
    let openai_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(OpenAiAdapter::new(openai_key)));
    let gemini_key = std::env::var("GOOGLE_API_KEY")
        .or_else(|_| std::env::var("GEMINI_API_KEY"))
        .unwrap_or_default();
    b = b.llm(Arc::new(GeminiAdapter::new(gemini_key)));
    b = b.llm(Arc::new(OllamaAdapter::new()));
    let compat_key = std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default();
    b = b.llm(Arc::new(OpenAiCompatibleAdapter::new(compat_key)));

    if let Ok(spec) = std::env::var("CORVID_TEST_MOCK_INT_TOOLS") {
        for pair in spec.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let Some((name, value_str)) = pair.split_once(':') else {
                eprintln!(
                    "corvid: malformed CORVID_TEST_MOCK_INT_TOOLS entry `{pair}` (expected `name:value`); skipping"
                );
                continue;
            };
            let Ok(value) = value_str.trim().parse::<i64>() else {
                eprintln!(
                    "corvid: CORVID_TEST_MOCK_INT_TOOLS value `{value_str}` for `{name}` isn't a valid i64; skipping"
                );
                continue;
            };
            let name_owned = name.trim().to_string();
            b = b.tool(name_owned, move |_args| async move {
                Ok(serde_json::json!(value))
            });
        }
    }

    b = install_connectors(b);
    b.build()
}

pub(super) fn build_tracer_from_env_or_default() -> Tracer {
    if std::env::var("CORVID_TRACE_DISABLE").ok().as_deref() == Some("1") {
        Tracer::null()
    } else if let Some(trace_path) = trace_path_from_env() {
        Tracer::open_path(trace_path, fresh_run_id()).with_redaction(RedactionSet::from_env())
    } else {
        let trace_dir = trace_dir_for_current_process();
        Tracer::open(&trace_dir, fresh_run_id()).with_redaction(RedactionSet::from_env())
    }
}

/// `target/trace/` under the current process's working directory. Same
/// convention as the interpreter tier uses — a compiled binary run
/// from `<project>/` writes traces next to the project's other
/// build artifacts.
pub(super) fn trace_dir_for_current_process() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target")
        .join("trace")
}

// ------------------------------------------------------------
// Tests — internal only, exercise the safe Rust surface. The C-ABI
// path is covered by the corvid-codegen-cl parity harness once
// the early link flow lands.
// ------------------------------------------------------------
