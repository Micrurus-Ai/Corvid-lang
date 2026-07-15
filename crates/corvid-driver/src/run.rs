//! Run execution — from source path to exit code.
//!
//! `corvid run <file>` picks a tier (auto / native / interpreter),
//! orchestrates the build, and invokes the runtime. Errors from any
//! stage are surfaced as `RunError`. The native tier also feeds a
//! per-binary `CachedNativeBinary` so repeated runs skip recompile.
//!
//! Extracted from `lib.rs` as part of Phase 20i responsibility
//! decomposition (20i-audit-driver-e).

use super::native_cache;
use super::{
    compile_to_ir_with_config_at_path, load_corvid_config_for, load_corvid_config_with_path_for,
    native_ability, render_all_pretty, run_ir_with_runtime, Diagnostic, NotNativeReason,
};
use corvid_ir::IrFile;
use corvid_runtime::{
    load_dotenv_walking, AnthropicAdapter, EnvVarMockAdapter, HttpEgressPolicy, IoToolPolicy,
    OllamaAdapter, OpenAiAdapter, RedactionSet, Runtime, StdinApprover, Tracer,
};
use corvid_vm::{InterpError, Value};
use std::fmt;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------
// Native run: compile + interpret with a Runtime.
// ------------------------------------------------------------

/// Errors `run_with_runtime` and friends can produce.
#[derive(Debug)]
pub enum RunError {
    /// IO failed reading the source file.
    Io { path: PathBuf, error: std::io::Error },
    /// Frontend produced diagnostics; nothing to run.
    Compile(Vec<Diagnostic>),
    /// The IR contains no agents.
    NoAgents,
    /// The caller didn't pick an agent and there are several to choose from.
    AmbiguousAgent { available: Vec<String> },
    /// The named agent doesn't exist in the IR.
    UnknownAgent { name: String, available: Vec<String> },
    /// The caller asked for default args but the agent expects parameters.
    NeedsArgs { agent: String, expected: usize },
    /// The interpreter aborted.
    Interp(InterpError),
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "cannot read `{}`: {}", path.display(), error),
            Self::Compile(d) => write!(f, "{} compile error(s)", d.len()),
            Self::NoAgents => write!(f, "no agents declared in this file"),
            Self::AmbiguousAgent { available } => write!(
                f,
                "multiple agents declared; pick one with --agent. available: {}",
                available.join(", ")
            ),
            Self::UnknownAgent { name, available } => write!(
                f,
                "no agent named `{name}`. available: {}",
                available.join(", ")
            ),
            Self::NeedsArgs { agent, expected } => write!(
                f,
                "agent `{agent}` expects {expected} argument(s); `corvid run` cannot supply them yet — use a runner binary that calls `run_with_runtime` with arguments"
            ),
            Self::Interp(e) => write!(f, "{e}"),
        }
    }
}

impl RunError {
    /// User-facing detail string, suitable for HTTP 500 response
    /// bodies and other operator-facing surfaces where internal
    /// compiler artifacts (IR byte-spans, etc.) leak the
    /// implementation. The default `Display` impl prepends
    /// `[start..end]` to interpreter errors because that anchor is
    /// useful in tracing + dev-time stderr; the HTTP layer should
    /// strip it because clients can't act on a byte-span in source
    /// they don't have. Slice 33Q10 (maintainer-as-reviewer-2026-06-05
    /// P2.2): without this method, a 500 from
    /// `corvid serve` carried bodies like
    /// `{"detail":"[1227..1269] no handler registered for tool ..."}` —
    /// the bracketed range is meaningless to the client.
    pub fn user_facing_detail(&self) -> String {
        match self {
            Self::Interp(e) => e.kind.to_string(),
            other => other.to_string(),
        }
    }
}

impl std::error::Error for RunError {}

/// Which execution tier `corvid run` should use.
///
/// - `Auto` (default): try the native AOT tier; fall back to the
///   interpreter when the program uses features native doesn't support
///   yet (tool calls, prompts, `approve`, Python imports). A one-line
///   stderr message announces the fallback so the user can reason about
///   which tier actually ran.
/// - `Native`: require the native tier. Programs that need the
///   interpreter fail with a clean error naming the missing feature.
/// - `Interpreter`: force the interpreter, even when native would work.
///   Useful for debugging, trace capture, and comparing tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTarget {
    Auto,
    Native,
    Interpreter,
}

/// `corvid run <file>` with auto-dispatch — native tier where possible,
/// interpreter fallback with an announced-on-stderr reason otherwise.
/// Equivalent to `run_with_target(path, RunTarget::Auto, None, &[])`.
pub fn run_native(path: &Path) -> Result<u8, anyhow::Error> {
    run_with_target(path, RunTarget::Auto, None, &[])
}

/// `corvid run <file> [--target=...] [--with-tools-lib <path>]`
/// entry point. Dispatches by tier per `target`; when `tools_lib`
/// is `Some`, tool-using programs gain access to the native tier
/// (their tool implementations live in that staticlib). Without a tools_lib, tool calls still route
/// to the interpreter fallback (auto) or hard-fail (native).
///
/// Common setup (env, tracer config) lives in the per-tier helpers
/// since only the interpreter needs the async runtime.
pub fn run_with_target(
    path: &Path,
    target: RunTarget,
    tools_lib: Option<&Path>,
    args: &[String],
) -> Result<u8, anyhow::Error> {
    // Env is loaded for both tiers: the native binary may read it via
    // libc `getenv` (the entry shim's leak-counter toggle does), and the
    // interpreter needs API keys from it.
    if let Some(parent) = path.parent() {
        let _ = load_dotenv_walking(parent);
    }
    let _ = load_dotenv_walking(&std::env::current_dir().unwrap_or_else(|_| Path::new(".").into()));

    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read `{}`: {e}", path.display());
            return Ok(1);
        }
    };
    let config = load_corvid_config_for(path);
    let ir = match compile_to_ir_with_config_at_path(&source, path, config.as_ref()) {
        Ok(ir) => ir,
        Err(diags) => {
            eprint!("{}", render_all_pretty(&diags, path, &source));
            return Ok(1);
        }
    };

    // Tool calls are native-able only when the caller supplied a
    // tools staticlib. The `native_ability` scan reports ToolCall
    // unconditionally (it doesn't know about the lib); the dispatcher
    // here decides whether to treat that reason as a blocker. Other
    // reasons (python imports, prompt calls) still block until their
    // respective feature gaps.
    let scan = native_ability(&ir);
    let tools_satisfy = |r: &NotNativeReason| -> bool {
        matches!(r, NotNativeReason::ToolCall { .. }) && tools_lib.is_some()
    };

    match target {
        RunTarget::Native => match &scan {
            Ok(()) => run_via_native_tier(path, &source, &ir, tools_lib, args),
            Err(reason) if tools_satisfy(reason) => {
                run_via_native_tier(path, &source, &ir, tools_lib, args)
            }
            Err(reason) => {
                eprintln!(
                    "error: `--target=native` refused: {reason}. Run without `--target` to fall back to the interpreter."
                );
                Ok(1)
            }
        },
        RunTarget::Interpreter => run_via_interpreter_tier(path, &ir, args),
        RunTarget::Auto => match &scan {
            Ok(()) => try_native_then_interpret(path, &source, &ir, tools_lib, args),
            Err(reason) if tools_satisfy(reason) => {
                try_native_then_interpret(path, &source, &ir, tools_lib, args)
            }
            Err(reason) => {
                eprintln!("↻ running via interpreter: {reason}");
                run_via_interpreter_tier(path, &ir, args)
            }
        },
    }
}

/// Run the native tier; if it fails because the corvid-runtime
/// staticlib isn't available on this host, fall back to the
/// interpreter using the same `↻ running via interpreter:` UX prefix
/// the eligibility-scan path emits. Any other native-build failure
/// (linker errors unrelated to the staticlib, codegen rejections)
/// keeps propagating — those are real bugs the user should see.
///
/// Used by the `RunTarget::Auto` arm only. `RunTarget::Native`
/// continues to surface the actionable diagnostic from
/// `missing_staticlib_diagnostic` so users who explicitly opted
/// into native still see the recovery instructions instead of a
/// silent fall-back they didn't ask for.
fn try_native_then_interpret(
    path: &Path,
    source: &str,
    ir: &IrFile,
    tools_lib: Option<&Path>,
    args: &[String],
) -> Result<u8, anyhow::Error> {
    match run_via_native_tier(path, source, ir, tools_lib, args) {
        Ok(code) => Ok(code),
        Err(err) if is_missing_staticlib_error(&err) => {
            eprintln!("↻ running via interpreter: native staticlib unavailable");
            run_via_interpreter_tier(path, ir, args)
        }
        Err(err) => Err(err),
    }
}

/// Detect the two error shapes that mean "the corvid-runtime
/// staticlib couldn't be found on this host" — the missing-fallback
/// path from `corvid-codegen-cl::link::missing_staticlib_diagnostic`
/// and the `CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent
/// path` override-branch error from the same module. Both indicate
/// the runtime staticlib is unavailable for linking on this host;
/// neither indicates a real codegen bug.
///
/// String-matching the diagnostic phrase is acceptable here because
/// the phrases are stable (link.rs has unit tests pinning both) and
/// owned by the same workspace. If the upstream wording ever changes,
/// the link.rs unit test fails first and forces this matcher to
/// follow.
fn is_missing_staticlib_error(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}");
    msg.contains("corvid-runtime staticlib missing")
        || msg.contains("CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent path")
}

/// Interpreter tier: build a `Runtime` with stdin approver + env-driven
/// LLM adapters + JSONL tracer, run the entry agent under the async
/// interpreter, print its return value. Matches prior `run_native`
/// semantics exactly — this is the only path that existed before 12j.
/// Slice 33Q17a: pick the entry agent the same way `run_ir_with_runtime`
/// will (single-agent OR `main` OR ambiguous error), then parse each
/// CLI string against the agent's declared scalar parameter types. We
/// do the parse HERE (not inside the VM call) so the user sees a
/// crisp `cannot parse "abc" as Int for parameter `n`` error rather
/// than a generic type mismatch deep in the VM.
fn parse_args_for_entry_agent(
    ir: &IrFile,
    args: &[String],
) -> Result<Vec<Value>, anyhow::Error> {
    if args.is_empty() {
        return Ok(Vec::new());
    }
    let chosen = if ir.agents.len() == 1 {
        &ir.agents[0]
    } else if let Some(main) = ir.agents.iter().find(|a| a.name == "main") {
        main
    } else {
        anyhow::bail!(
            "cannot pick an entry agent to receive {} CLI argument(s): \
             this file declares multiple agents and none of them is named \
             `main`. Use a runner that calls `run_with_runtime` with an \
             explicit agent name.",
            args.len()
        );
    };
    if args.len() != chosen.params.len() {
        anyhow::bail!(
            "agent `{}` expects {} argument(s), got {} from the CLI",
            chosen.name,
            chosen.params.len(),
            args.len()
        );
    }
    let mut parsed = Vec::with_capacity(args.len());
    for (param, raw) in chosen.params.iter().zip(args.iter()) {
        let value = match &param.ty {
            corvid_types::Type::Int => Value::Int(raw.parse::<i64>().map_err(|_| {
                anyhow::anyhow!(
                    "cannot parse `{raw}` as Int for parameter `{}` of agent `{}`",
                    param.name,
                    chosen.name
                )
            })?),
            corvid_types::Type::Float => Value::Float(raw.parse::<f64>().map_err(|_| {
                anyhow::anyhow!(
                    "cannot parse `{raw}` as Float for parameter `{}` of agent `{}`",
                    param.name,
                    chosen.name
                )
            })?),
            corvid_types::Type::Bool => match raw.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => anyhow::bail!(
                    "cannot parse `{raw}` as Bool for parameter `{}` of agent `{}` \
                     (expected `true` or `false`)",
                    param.name,
                    chosen.name
                ),
            },
            corvid_types::Type::String => {
                Value::String(std::sync::Arc::<str>::from(raw.as_str()))
            }
            other => anyhow::bail!(
                "parameter `{}` of agent `{}` has type `{}` which is not \
                 supported as a CLI argument; only `Int` / `Float` / `Bool` / \
                 `String` parameters can receive `corvid run` positional args",
                param.name,
                chosen.name,
                other.display_name()
            ),
        };
        parsed.push(value);
    }
    Ok(parsed)
}

fn run_via_interpreter_tier(
    path: &Path,
    ir: &IrFile,
    args: &[String],
) -> Result<u8, anyhow::Error> {
    let trace_dir = trace_dir_for(path);
    let tracer = Tracer::open(&trace_dir, corvid_runtime::fresh_run_id())
        .with_redaction(RedactionSet::from_env());

    let mut builder = Runtime::builder()
        .approver(std::sync::Arc::new(StdinApprover::new()))
        .tracer(tracer);

    // Slice 33S1b: install the [io] root policy parsed from
    // corvid.toml (33S0) onto the runtime so the executing
    // io.read_text / io.write_text / io.list_dir tools resolve
    // every caller path through the configured root. CORVID_IO_ROOT
    // env override takes precedence over the corvid.toml value —
    // matches the existing env-override pattern for CORVID_MODEL.
    builder = builder.io_policy(load_io_tool_policy(path));

    // Slice 33S2b: install the [http] allow policy parsed from
    // corvid.toml (33S0) onto the runtime so the executing
    // http_get / http_post_json tools gate every URL through the
    // allowlist + always-on SSRF block. CORVID_HTTP_ALLOW env
    // override (comma-separated host list) takes precedence over
    // the corvid.toml value — same env-override pattern as
    // CORVID_IO_ROOT / CORVID_MODEL.
    builder = builder.http_policy(load_http_egress_policy(path));

    // Slice 46g: install the [rag] embedder when configured; the
    // executing rag tools degrade to lexical search without one.
    if let Some(embedder) = load_rag_embedder(path) {
        builder = builder.rag_embedder(embedder);
    }

    // Slice 46f: configured MCP servers (untrusted by default —
    // approval-gated through the StdinApprover installed above).
    let mcp_servers = load_mcp_servers(path);
    if !mcp_servers.is_empty() {
        builder = builder.mcp_servers(mcp_servers);
    }

    builder = apply_env_llm_wiring(builder);
    let rt = builder.build();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let parsed_args = match parse_args_for_entry_agent(ir, args) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(1);
        }
    };
    // Stream draining must happen INSIDE the same async context as
    // the run: the chunk producer is a task spawned during the run,
    // and it must be driven while stdout drains — a Stream-returning
    // agent streams each chunk to stdout the moment it arrives
    // (printing the stream HANDLE's debug form was the pre-50d
    // behavior).
    let result = tokio_rt.block_on(async {
        match run_ir_with_runtime(ir, None, parsed_args, &rt).await {
            Ok(corvid_vm::Value::Stream(stream)) => {
                use std::io::Write as _;
                loop {
                    match stream.next().await {
                        Some(Ok(chunk)) => {
                            match &chunk {
                                corvid_vm::Value::String(s) => print!("{s}"),
                                other => print!("{other}"),
                            }
                            let _ = std::io::stdout().flush();
                        }
                        Some(Err(err)) => {
                            eprintln!();
                            return Ok(corvid_vm::Value::String(
                                format!("stream error: {err:?}").into(),
                            ));
                        }
                        None => break,
                    }
                }
                println!();
                Ok(corvid_vm::Value::Nothing)
            }
            other => other,
        }
    });

    match result {
        Ok(corvid_vm::Value::Nothing) => Ok(0),
        Ok(value) => {
            println!("{value}");
            Ok(0)
        }
        Err(RunError::Compile(diags)) => {
            let source = std::fs::read_to_string(path).unwrap_or_default();
            eprint!("{}", render_all_pretty(&diags, path, &source));
            Ok(1)
        }
        Err(other) => {
            eprintln!("error: {other}");
            Ok(1)
        }
    }
}

/// Native tier: produce a binary (via cache when possible) and exec it.
/// The codegen-emitted `main` handles argv decoding and result printing,
/// so we inherit stdin/stdout/stderr and let the binary
/// own the user interaction directly.
fn run_via_native_tier(
    path: &Path,
    source: &str,
    ir: &IrFile,
    tools_lib: Option<&Path>,
    args: &[String],
) -> Result<u8, anyhow::Error> {
    let binary = build_or_get_cached_native(path, source, ir, tools_lib)?.path;
    // Slice 33Q17a: forward CLI positional args verbatim. The
    // codegen-emitted `main` decodes argv per parameter type, so the
    // shape is the same as a normal C-style `prog arg1 arg2 ...` —
    // pass the strings through.
    let status = std::process::Command::new(&binary)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("spawn native binary `{}`: {e}", binary.display()))?;
    Ok(status.code().unwrap_or(1) as u8)
}

/// Result of asking the cache for a compiled binary — used by tests to
/// verify cache hits without re-timing the whole pipeline.
#[derive(Debug, Clone)]
pub struct CachedNativeBinary {
    pub path: PathBuf,
    /// `true` if the binary already existed in the cache (no recompile
    /// happened this call); `false` if we compiled it now.
    pub from_cache: bool,
}

/// Core compile-or-reuse path. Hashes the inputs to pick a cache slot,
/// uses the existing binary if it's there, otherwise invokes codegen
/// + link and stores the result keyed by that hash.
///
/// Does NOT run the binary — that's the caller's job. Exposed as `pub`
/// so tests + future `corvid build --cache` tooling can observe the
/// cache state without executing.
pub fn build_or_get_cached_native(
    path: &Path,
    source: &str,
    ir: &IrFile,
    tools_lib: Option<&Path>,
) -> anyhow::Result<CachedNativeBinary> {
    let cache_dir = native_cache::cache_dir_for(path);
    // Tools-lib path participates in the cache key: if the user
    // swaps between `--with-tools-lib A` and `--with-tools-lib B`,
    // they get distinct cached binaries. Re-linking against the same
    // lib re-uses. Users who modify A in place and keep the same
    // path get stale cache — a `cargo clean` fixes it; a future
    // future polish work could hash the lib contents.
    let tools_lib_str = tools_lib
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let key = native_cache::cache_key_with_tools(source, &tools_lib_str);
    let cached = native_cache::cached_binary_path(&cache_dir, &key);
    if cached.exists() {
        return Ok(CachedNativeBinary {
            path: cached,
            from_cache: true,
        });
    }
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| anyhow::anyhow!("create cache dir `{}`: {e}", cache_dir.display()))?;
    // `build_native_to_disk` takes the final bin_path and derives parent
    // + stem from it — passing `<cache_dir>/<key>` produces
    // `<cache_dir>/<key>[.exe]` which is exactly where we want it.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("program")
        .to_string();
    let module_name = format!("corvid_native_{key}");
    let target_bin = cache_dir.join(&key);
    // Forward the tools lib (if any) to the linker so
    // `__corvid_tool_<name>` symbols resolve against the user's
    // compiled `#[tool]` implementations.
    let extra_libs_owned: Vec<&Path> = tools_lib.iter().copied().collect();
    let produced = corvid_codegen_cl::build_native_to_disk(
        ir,
        &module_name,
        &target_bin,
        &extra_libs_owned,
    )
    .map_err(|e| anyhow::anyhow!("native codegen failed for `{stem}`: {e}"))?;
    Ok(CachedNativeBinary {
        path: produced,
        from_cache: false,
    })
}

/// Pick a trace directory next to the source file's project root.
fn trace_dir_for(source_path: &Path) -> PathBuf {
    let mut ancestor: Option<&Path> = source_path.parent();
    while let Some(dir) = ancestor {
        if dir.file_name().map(|n| n == "src").unwrap_or(false) {
            if let Some(project_root) = dir.parent() {
                return project_root.join("target").join("trace");
            }
        }
        ancestor = dir.parent();
    }
    let parent = source_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join("target").join("trace")
}

#[cfg(test)]
mod tests {
    use super::is_missing_staticlib_error;

    #[test]
    fn detects_missing_staticlib_diagnostic_phrase() {
        // 20m-B regression: the auto-fallback branch must recognise
        // the canonical missing-fallback message produced by
        // `corvid_codegen_cl::link::missing_staticlib_diagnostic`,
        // wrapped by anyhow context as it bubbles up from
        // `run_via_native_tier`.
        let err = anyhow::anyhow!(
            "build native binary: link failed: corvid-runtime staticlib missing at `/dev/null/corvid_runtime.lib`. To fix this, ..."
        );
        assert!(
            is_missing_staticlib_error(&err),
            "should match canonical missing-fallback phrase"
        );
    }

    #[test]
    fn detects_override_branch_phrase() {
        // The `CORVID_RUNTIME_STATICLIB_OVERRIDE` override branch in
        // `link.rs` returns a different error string when the override
        // path doesn't exist. Same audience hits both paths
        // (staticlib unavailable on this host), so the auto-fallback
        // matcher must catch both.
        let err = anyhow::anyhow!(
            "CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent path `/tmp/missing.lib`"
        );
        assert!(
            is_missing_staticlib_error(&err),
            "should match override-branch phrase"
        );
    }

    #[test]
    fn rejects_unrelated_codegen_errors() {
        // Real codegen / link bugs (anything not staticlib-discovery)
        // must NOT trigger silent fall-back — those are bugs the user
        // should see.
        let err = anyhow::anyhow!("link failed: undefined symbol __corvid_unknown_helper");
        assert!(
            !is_missing_staticlib_error(&err),
            "non-staticlib link errors must propagate"
        );
        let err = anyhow::anyhow!("codegen failed: cranelift rejected basic block");
        assert!(
            !is_missing_staticlib_error(&err),
            "codegen errors must propagate"
        );
    }
}

/// Slice 33S1b — build the `IoToolPolicy` for a `corvid run` /
/// `corvid serve` invocation. Resolution order:
///
///   1. `CORVID_IO_ROOT` env var (overrides config; matches the
///      existing env-override pattern for CORVID_MODEL etc.).
///   2. `[io] root` from the loaded `corvid.toml`. Relative paths
///      anchor against the corvid.toml directory; absolute paths
///      are taken as-is.
///   3. None (unconfigured). Every executing file-I/O call then
///      fails closed with the missing-config diagnostic — the
///      33S0 security model.
///
/// Public so the embed paths (`corvid serve`, custom embedders)
/// can share this loading logic instead of re-implementing the
/// precedence.
/// Slice 46g: build the embedding provider from `[rag]` in
/// corvid.toml. `embedder = "openai"` needs OPENAI_API_KEY in the
/// environment; `embedder = "ollama"` takes an optional
/// `endpoint`. No `[rag]` table (or a missing key) means NO
/// embedder — retrieval degrades honestly to lexical search.
pub fn load_rag_embedder(
    source_path: &Path,
) -> Option<std::sync::Arc<dyn corvid_runtime::rag::RagEmbedder>> {
    let (_, config) = load_corvid_config_with_path_for(source_path)?;
    let provider = config.rag.embedder.as_deref()?;
    let model = config.rag.model.clone().unwrap_or_default();
    match provider {
        "openai" => {
            let key = std::env::var("OPENAI_API_KEY").ok()?;
            let model = if model.is_empty() {
                "text-embedding-3-small".to_string()
            } else {
                model
            };
            Some(std::sync::Arc::new(
                corvid_runtime::rag::OpenAiEmbedder::new(key, model),
            ))
        }
        "ollama" => {
            let model = if model.is_empty() {
                "nomic-embed-text".to_string()
            } else {
                model
            };
            let mut embedder = corvid_runtime::rag::OllamaEmbedder::new(model);
            if let Some(endpoint) = config.rag.endpoint.as_deref() {
                embedder = embedder.with_endpoint(endpoint);
            }
            Some(std::sync::Arc::new(embedder))
        }
        _ => None,
    }
}

/// Slice 46f: build the MCP server map from `[mcp.servers]` in
/// corvid.toml. Servers default to UNTRUSTED (approval-gated).
pub fn load_mcp_servers(
    source_path: &Path,
) -> std::collections::HashMap<String, corvid_runtime::mcp::McpServerConfig> {
    let Some((_, config)) = load_corvid_config_with_path_for(source_path) else {
        return Default::default();
    };
    config
        .mcp
        .servers
        .into_iter()
        .map(|(name, entry)| {
            (
                name,
                corvid_runtime::mcp::McpServerConfig {
                    command: entry.command,
                    url: entry.url,
                    trusted: entry.trust.as_deref() == Some("autonomous"),
                },
            )
        })
        .collect()
}

pub fn load_io_tool_policy(source_path: &Path) -> IoToolPolicy {
    // 1. Env override.
    if let Ok(env_root) = std::env::var("CORVID_IO_ROOT") {
        let env_root_trimmed = env_root.trim();
        if !env_root_trimmed.is_empty() {
            // The env override is treated relative to the
            // current working directory if not absolute — no
            // corvid.toml anchor is in play here.
            return IoToolPolicy::new(Some(env_root_trimmed), None);
        }
    }

    // 2. corvid.toml.
    if let Some((toml_path, config)) = load_corvid_config_with_path_for(source_path) {
        let anchor = toml_path.parent();
        return IoToolPolicy::new(config.io.root.as_deref(), anchor);
    }

    // 3. Unconfigured — fail-closed default.
    IoToolPolicy::unset()
}

/// Slice 33S2b — build the `HttpEgressPolicy` for a `corvid run` /
/// `corvid serve` invocation. Resolution order mirrors
/// `load_io_tool_policy` exactly:
///
///   1. `CORVID_HTTP_ALLOW` env var (overrides config; matches the
///      existing env-override pattern for CORVID_MODEL /
///      CORVID_IO_ROOT). Value is a comma-separated host list:
///      `CORVID_HTTP_ALLOW=api.example.com,api.anthropic.com`.
///      Empty entries are stripped; whitespace around hosts is
///      trimmed.
///   2. `[http] allow = [...]` from the loaded `corvid.toml`. Each
///      entry is taken verbatim (lowercase comparison happens
///      inside `HttpEgressPolicy::check`).
///   3. None (unconfigured). Every executing HTTP call then fails
///      closed with the missing-config diagnostic — the 33S0
///      security model.
///
/// The SSRF block is a structural property of
/// `HttpEgressPolicy::check`; it runs regardless of which path
/// supplied the allowlist, and there is no env override that can
/// disable it.
///
/// Public so the embed paths (`corvid serve`, custom embedders)
/// can share this loading logic instead of re-implementing the
/// precedence.
pub fn load_http_egress_policy(source_path: &Path) -> HttpEgressPolicy {
    // 1. Env override.
    if let Ok(env_allow) = std::env::var("CORVID_HTTP_ALLOW") {
        let env_allow_trimmed = env_allow.trim();
        if !env_allow_trimmed.is_empty() {
            let hosts: Vec<String> = env_allow_trimmed
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !hosts.is_empty() {
                return HttpEgressPolicy::new(Some(&hosts));
            }
        }
    }

    // 2. corvid.toml.
    if let Some((_toml_path, config)) = load_corvid_config_with_path_for(source_path) {
        // An empty allow list is intentional: it signals "the
        // project has been configured, but no hosts are
        // approved yet" — fail-closed. The configured-vs-
        // unset distinction is preserved here by passing the
        // (possibly empty) list through `HttpEgressPolicy::new`.
        return HttpEgressPolicy::new(Some(&config.http.allow));
    }

    // 3. Unconfigured — fail-closed default.
    HttpEgressPolicy::unset()
}

#[cfg(test)]
mod io_policy_loader_tests {
    use super::*;

    /// Slice 33S1b — corvid.toml with `[io] root = "."` produces
    /// a configured policy with the corvid.toml's parent dir as
    /// the resolved root.
    #[test]
    fn corvid_toml_with_relative_root_dot_anchors_against_toml_dir() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[io]\nroot = \".\"\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_IO_ROOT").ok();
        // SAFETY: tests serialize on env via dedicated locks; for
        // a single-test path we accept the rare race against
        // unrelated tests.
        unsafe { std::env::remove_var("CORVID_IO_ROOT") };

        let policy = load_io_tool_policy(&source);

        if let Some(v) = prior_env {
            unsafe { std::env::set_var("CORVID_IO_ROOT", v) };
        }

        assert!(
            policy.is_configured(),
            "configured corvid.toml should produce a configured policy"
        );
        let root = policy.root_path().expect("configured policy has root");
        // Project dir may be a symlinked /tmp path on some hosts —
        // normalize both sides via the policy's own resolver,
        // which uses the same normalize_path helper.
        assert!(
            root.ends_with(project.file_name().unwrap()),
            "configured root should be the corvid.toml dir; got {root:?}, expected to end with {project:?}"
        );
    }

    /// Slice 33S1b — missing `[io] root` (corvid.toml exists but
    /// no `[io]` table) produces an unconfigured policy. Every
    /// executing file-I/O call then fails closed.
    #[test]
    fn corvid_toml_without_io_section_produces_unconfigured_policy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[run]\ntarget = \"interpreter\"\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_IO_ROOT").ok();
        unsafe { std::env::remove_var("CORVID_IO_ROOT") };

        let policy = load_io_tool_policy(&source);

        if let Some(v) = prior_env {
            unsafe { std::env::set_var("CORVID_IO_ROOT", v) };
        }

        assert!(
            !policy.is_configured(),
            "no [io] section should produce an unconfigured policy"
        );
    }

    /// Slice 33S1b — `CORVID_IO_ROOT` env var takes precedence
    /// over `[io] root` in corvid.toml. Matches the existing
    /// env-override pattern for CORVID_MODEL.
    #[test]
    fn env_var_overrides_corvid_toml_io_root() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        let env_root = tmp.path().join("override_root");
        std::fs::create_dir_all(&env_root).unwrap();
        std::fs::write(
            project.join("corvid.toml"),
            "[io]\nroot = \"./from_toml\"\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_IO_ROOT").ok();
        unsafe { std::env::set_var("CORVID_IO_ROOT", env_root.to_str().unwrap()) };

        let policy = load_io_tool_policy(&source);

        match prior_env {
            Some(v) => unsafe { std::env::set_var("CORVID_IO_ROOT", v) },
            None => unsafe { std::env::remove_var("CORVID_IO_ROOT") },
        }

        assert!(policy.is_configured());
        let root = policy.root_path().expect("configured");
        assert!(
            root.ends_with("override_root"),
            "CORVID_IO_ROOT env override should win over corvid.toml; got {root:?}"
        );
    }
}

#[cfg(test)]
mod http_policy_loader_tests {
    //! Slice 33S2b — loader unit tests mirroring the
    //! `io_policy_loader_tests` mod. Resolution precedence:
    //! `CORVID_HTTP_ALLOW` env override > `[http] allow` from
    //! corvid.toml > unconfigured (fail-closed default).
    //!
    //! These tests serialise on env state — they always snapshot
    //! the current `CORVID_HTTP_ALLOW` value, mutate inside the
    //! test, then restore it. Two tests touching the same env
    //! var must NOT run in parallel; the `tokio::test` flavor
    //! `multi_thread` is fine here because cargo test runs each
    //! function on its own thread by default, but cross-test
    //! parallelism is the actual concern. For the loader tests
    //! here that's `cargo test`'s job to schedule; we don't add
    //! a serial lock because the existing 33S1b `CORVID_IO_ROOT`
    //! tests pass the same way.

    use super::*;

    /// 33S2b — corvid.toml with `[http] allow = ["api.example.com"]`
    /// produces a configured policy whose `allow_list` includes
    /// the host.
    #[test]
    fn corvid_toml_with_http_allow_produces_configured_policy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[http]\nallow = [\"api.example.com\", \"api.anthropic.com\"]\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_HTTP_ALLOW").ok();
        unsafe { std::env::remove_var("CORVID_HTTP_ALLOW") };

        let policy = load_http_egress_policy(&source);

        if let Some(v) = prior_env {
            unsafe { std::env::set_var("CORVID_HTTP_ALLOW", v) };
        }

        assert!(
            policy.is_configured(),
            "non-empty [http] allow should produce a configured policy"
        );
        let allow = policy.allow_list();
        assert!(
            allow.contains(&"api.example.com".to_string()),
            "configured allowlist should contain api.example.com; got {allow:?}"
        );
        assert!(
            allow.contains(&"api.anthropic.com".to_string()),
            "configured allowlist should contain api.anthropic.com; got {allow:?}"
        );
    }

    /// 33S2b — corvid.toml with an empty `[http] allow = []`
    /// produces an unconfigured policy. Same fail-closed shape
    /// as missing-section: the empty-list-and-missing-section
    /// distinction is intentional UX (the scaffolded
    /// corvid.toml uses an empty allow list to make the
    /// security boundary visible without granting any default
    /// host access).
    #[test]
    fn corvid_toml_with_empty_http_allow_produces_unconfigured_policy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[http]\nallow = []\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_HTTP_ALLOW").ok();
        unsafe { std::env::remove_var("CORVID_HTTP_ALLOW") };

        let policy = load_http_egress_policy(&source);

        if let Some(v) = prior_env {
            unsafe { std::env::set_var("CORVID_HTTP_ALLOW", v) };
        }

        assert!(
            !policy.is_configured(),
            "empty [http] allow should fail closed like missing section"
        );
    }

    /// 33S2b — corvid.toml WITHOUT a `[http]` section produces
    /// an unconfigured policy. Every executing HTTP call then
    /// fails closed.
    #[test]
    fn corvid_toml_without_http_section_produces_unconfigured_policy() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[run]\ntarget = \"interpreter\"\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_HTTP_ALLOW").ok();
        unsafe { std::env::remove_var("CORVID_HTTP_ALLOW") };

        let policy = load_http_egress_policy(&source);

        if let Some(v) = prior_env {
            unsafe { std::env::set_var("CORVID_HTTP_ALLOW", v) };
        }

        assert!(
            !policy.is_configured(),
            "no [http] section should produce an unconfigured policy"
        );
    }

    /// 33S2b — `CORVID_HTTP_ALLOW` env var takes precedence
    /// over `[http] allow` in corvid.toml. Mirrors the existing
    /// `CORVID_IO_ROOT` env-override pattern from 33S1b.
    /// Comma-separated entries are parsed; whitespace trimmed.
    #[test]
    fn env_var_overrides_corvid_toml_http_allow() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[http]\nallow = [\"from-toml.example\"]\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_HTTP_ALLOW").ok();
        unsafe {
            std::env::set_var(
                "CORVID_HTTP_ALLOW",
                "env-host-one.example, env-host-two.example",
            )
        };

        let policy = load_http_egress_policy(&source);

        match prior_env {
            Some(v) => unsafe { std::env::set_var("CORVID_HTTP_ALLOW", v) },
            None => unsafe { std::env::remove_var("CORVID_HTTP_ALLOW") },
        }

        assert!(policy.is_configured());
        let allow = policy.allow_list();
        assert!(
            allow.contains(&"env-host-one.example".to_string()),
            "env override should populate first host; got {allow:?}"
        );
        assert!(
            allow.contains(&"env-host-two.example".to_string()),
            "env override should populate second host; got {allow:?}"
        );
        assert!(
            !allow.iter().any(|h| h.contains("from-toml")),
            "env override should fully replace corvid.toml allowlist; got {allow:?}"
        );
    }

    /// 33S2b — `CORVID_HTTP_ALLOW` set to whitespace or comma-
    /// only garbage falls back to the corvid.toml value (or
    /// unconfigured if no toml). The env override should
    /// require at least one non-empty host to take effect.
    #[test]
    fn env_var_with_only_whitespace_falls_back_to_corvid_toml() {
        let tmp = tempfile::tempdir().expect("tmp");
        let project = tmp.path();
        std::fs::write(
            project.join("corvid.toml"),
            "[http]\nallow = [\"fallback.example\"]\n",
        )
        .expect("write corvid.toml");
        let source = project.join("src").join("main.cor");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "agent main() -> Int:\n    return 0\n").unwrap();

        let prior_env = std::env::var("CORVID_HTTP_ALLOW").ok();
        unsafe { std::env::set_var("CORVID_HTTP_ALLOW", "   ,, ,  ") };

        let policy = load_http_egress_policy(&source);

        match prior_env {
            Some(v) => unsafe { std::env::set_var("CORVID_HTTP_ALLOW", v) },
            None => unsafe { std::env::remove_var("CORVID_HTTP_ALLOW") },
        }

        assert!(
            policy.is_configured(),
            "whitespace-only env override should fall back to corvid.toml"
        );
        let allow = policy.allow_list();
        assert!(
            allow.contains(&"fallback.example".to_string()),
            "corvid.toml fallback should win when env is garbage; got {allow:?}"
        );
    }
}

/// The env-driven LLM wiring every Corvid entry point shares:
/// `CORVID_MODEL` default, the test mock adapter, provider adapters
/// from API-key env vars, and the local Ollama fallback. `corvid
/// run` always had this; `corvid serve` shipped WITHOUT it, so
/// served apps could not call models at all — every entry point
/// must wire models identically.
pub fn apply_env_llm_wiring(
    mut builder: corvid_runtime::RuntimeBuilder,
) -> corvid_runtime::RuntimeBuilder {
    if let Ok(model) = std::env::var("CORVID_MODEL") {
        builder = builder.default_model(&model);
    }
    if std::env::var("CORVID_TEST_MOCK_LLM").ok().as_deref() == Some("1") {
        builder = builder.llm(std::sync::Arc::new(EnvVarMockAdapter::from_env()));
    }
    builder = builder.env_mock_tools_from_env();
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        builder = builder.llm(std::sync::Arc::new(AnthropicAdapter::new(key)));
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        builder = builder.llm(std::sync::Arc::new(OpenAiAdapter::new(key)));
    }
    builder.llm(std::sync::Arc::new(OllamaAdapter::new()))
}
