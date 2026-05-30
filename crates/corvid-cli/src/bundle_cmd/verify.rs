use anyhow::{bail, Context, Result};
use corvid_bind::{generate_bindings_from_descriptor_path, BindLanguage};
use corvid_driver::{build_catalog_descriptor_for_source, build_target_to_disk, BuildTarget};
use corvid_trace_schema::{read_events_from_path, validate_supported_schema, TraceEvent};
use std::ffi::{c_char, CString};
use std::fs;
use std::path::Path;

use super::attest::verify_dsse_envelope;
use super::manifest::{
    compare_dirs, current_target_triple, sha256_dir, sha256_file, LoadedManifest,
};

pub fn run_verify(bundle: &Path, rebuild: bool) -> Result<u8> {
    let loaded = LoadedManifest::load(bundle)?;
    verify_committed(&loaded)?;
    if rebuild {
        verify_rebuild(&loaded)?;
    }
    println!(
        "bundle OK: {} ({})",
        loaded.manifest.name, loaded.manifest.target_triple
    );
    Ok(0)
}

fn verify_committed(loaded: &LoadedManifest) -> Result<()> {
    verify_hash(
        "library",
        &loaded.library_path(),
        &loaded.manifest.hashes.library,
        false,
    )?;
    verify_hash(
        "descriptor",
        &loaded.descriptor_path(),
        &loaded.manifest.hashes.descriptor,
        false,
    )?;
    if let (Some(path), Some(expected)) = (loaded.header_path(), loaded.manifest.hashes.header.as_deref()) {
        verify_hash("header", &path, expected, false)?;
    }
    if let (Some(path), Some(expected)) = (
        loaded.tools_staticlib_path(),
        loaded.manifest.hashes.tools_staticlib.as_deref(),
    ) {
        verify_hash("tools_staticlib", &path, expected, false)?;
    }
    verify_hash(
        "bindings_rust",
        &loaded.bindings_rust_dir(),
        &loaded.manifest.hashes.bindings_rust,
        true,
    )?;
    verify_hash(
        "bindings_python",
        &loaded.bindings_python_dir(),
        &loaded.manifest.hashes.bindings_python,
        true,
    )?;
    if let (Some(path), Some(expected)) = (
        loaded.capsule_path(),
        loaded.manifest.hashes.capsule.as_deref(),
    ) {
        verify_hash("capsule", &path, expected, false)?;
    }
    if let (Some(path), Some(expected)) = (
        loaded.receipt_envelope_path(),
        loaded.manifest.hashes.receipt_envelope.as_deref(),
    ) {
        verify_hash("receipt_envelope", &path, expected, false)?;
    }
    if let (Some(path), Some(expected)) = (
        loaded.receipt_verify_key_path(),
        loaded.manifest.hashes.receipt_verify_key.as_deref(),
    ) {
        verify_hash("receipt_verify_key", &path, expected, false)?;
    }
    for trace in &loaded.manifest.traces {
        verify_hash(
            &format!("trace `{}`", trace.name),
            &loaded.resolve(&trace.path),
            &trace.sha256,
            false,
        )?;
        let events = read_events_from_path(&loaded.resolve(&trace.path))
            .with_context(|| format!("read trace `{}`", loaded.resolve(&trace.path).display()))?;
        validate_supported_schema(&events)
            .with_context(|| format!("validate trace `{}`", loaded.resolve(&trace.path).display()))?;
        let (agent, _args) = last_run_started(&events)?;
        if agent != trace.expected_agent {
            bail!(
                "BundleTraceAgentMismatch: trace `{}` recorded `{}` but manifest expected `{}`",
                trace.name,
                agent,
                trace.expected_agent
            );
        }
    }

    if let (Some(envelope_path), Some(key_path)) = (
        loaded.receipt_envelope_path(),
        loaded.receipt_verify_key_path(),
    ) {
        verify_dsse_envelope(&envelope_path, &key_path)?;
    }

    Ok(())
}

fn verify_rebuild(loaded: &LoadedManifest) -> Result<()> {
    if loaded.manifest.target_triple != current_target_triple() {
        bail!(
            "BundlePlatformUnsupported: bundle target `{}` cannot be rebuilt on host `{}`",
            loaded.manifest.target_triple,
            current_target_triple()
        );
    }

    let abi_output = build_catalog_descriptor_for_source(&loaded.primary_source_path())
        .with_context(|| format!("rebuild descriptor from `{}`", loaded.primary_source_path().display()))?;
    if !abi_output.diagnostics.is_empty() {
        let first = &abi_output.diagnostics[0];
        bail!(
            "BundleRebuildFailed: descriptor rebuild surfaced {} diagnostic(s); first: {}",
            abi_output.diagnostics.len(),
            first
        );
    }
    let rebuilt_descriptor = abi_output
        .descriptor_json
        .ok_or_else(|| anyhow::anyhow!("BundleRebuildFailed: descriptor rebuild produced no JSON"))?;
    let expected_descriptor = fs::read(loaded.descriptor_path())
        .with_context(|| format!("read descriptor `{}`", loaded.descriptor_path().display()))?;
    super::manifest::compare_bytes(
        "descriptor",
        &expected_descriptor,
        rebuilt_descriptor.as_bytes(),
    )?;

    let library_restore = RestoredFile::capture(loaded.library_path())?;
    let header_restore = match loaded.header_path() {
        Some(path) => Some(RestoredFile::capture(path)?),
        None => None,
    };

    let tools_staticlib = loaded.tools_staticlib_path();
    let tool_refs: Vec<&Path> = tools_staticlib.iter().map(|path| path.as_path()).collect();
    let build_output = build_target_to_disk(
        &loaded.primary_source_path(),
        BuildTarget::Cdylib,
        loaded.header_path().is_some(),
        true,
        &tool_refs,
        None,
    )
    .with_context(|| format!("rebuild cdylib from `{}`", loaded.primary_source_path().display()))?;
    if !build_output.diagnostics.is_empty() {
        let first = &build_output.diagnostics[0];
        bail!(
            "BundleRebuildFailed: library rebuild surfaced {} diagnostic(s); first: {}",
            build_output.diagnostics.len(),
            first
        );
    }
    let rebuilt_library = build_output
        .output_path
        .ok_or_else(|| anyhow::anyhow!("BundleRebuildFailed: library rebuild produced no output"))?;
    let rebuilt_library_bytes = fs::read(&rebuilt_library)
        .with_context(|| format!("read rebuilt library `{}`", rebuilt_library.display()))?;
    super::manifest::compare_bytes("library", library_restore.original_bytes(), &rebuilt_library_bytes)?;
    if let (Some(expected_header), Some(rebuilt_header)) = (&header_restore, build_output.header_path) {
        let rebuilt_header_bytes = fs::read(&rebuilt_header)
            .with_context(|| format!("read rebuilt header `{}`", rebuilt_header.display()))?;
        super::manifest::compare_bytes(
            "header",
            expected_header.original_bytes(),
            &rebuilt_header_bytes,
        )?;
    }

    let temp = tempfile::tempdir().context("create bundle rebuild tempdir")?;
    let rebuilt_descriptor_path = temp.path().join(
        loaded
            .descriptor_path()
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("descriptor path had no filename"))?,
    );
    fs::write(&rebuilt_descriptor_path, rebuilt_descriptor).context("write rebuilt descriptor")?;

    let rebuilt_rust_dir = temp.path().join("bindings_rust");
    generate_bindings_from_descriptor_path(BindLanguage::Rust, &rebuilt_descriptor_path, &rebuilt_rust_dir)
        .context("rebuild Rust bindings")?;
    compare_dirs("bindings_rust", &loaded.bindings_rust_dir(), &rebuilt_rust_dir)?;

    let rebuilt_python_dir = temp.path().join("bindings_python");
    generate_bindings_from_descriptor_path(
        BindLanguage::Python,
        &rebuilt_descriptor_path,
        &rebuilt_python_dir,
    )
    .context("rebuild Python bindings")?;
    compare_dirs("bindings_python", &loaded.bindings_python_dir(), &rebuilt_python_dir)?;

    for trace in &loaded.manifest.traces {
        let result = unsafe { replay_library_trace(&rebuilt_library, &loaded.resolve(&trace.path)) }?;
        if result.agent != trace.expected_agent {
            bail!(
                "BundleReplayMismatch: trace `{}` replayed agent `{}` instead of `{}`",
                trace.name,
                result.agent,
                trace.expected_agent
            );
        }
        if result.result_json != trace.expected_result_json {
            bail!(
                "BundleReplayMismatch: trace `{}` result diverged (expected {}, got {})",
                trace.name,
                trace.expected_result_json,
                result.result_json
            );
        }
        if let Some(expected_observation) = trace.expected_observation {
            if result.observation_present != expected_observation {
                bail!(
                    "BundleReplayMismatch: trace `{}` observation presence diverged (expected {}, got {})",
                    trace.name,
                    expected_observation,
                    result.observation_present
                );
            }
        }
    }

    Ok(())
}

struct RestoredFile {
    path: std::path::PathBuf,
    original: Vec<u8>,
}

impl RestoredFile {
    fn capture(path: std::path::PathBuf) -> Result<Self> {
        let original =
            fs::read(&path).with_context(|| format!("read committed artifact `{}`", path.display()))?;
        Ok(Self { path, original })
    }

    fn original_bytes(&self) -> &[u8] {
        &self.original
    }
}

impl Drop for RestoredFile {
    fn drop(&mut self) {
        let _ = fs::write(&self.path, &self.original);
    }
}

fn verify_hash(label: &str, path: &Path, expected: &str, is_dir: bool) -> Result<()> {
    let actual = if is_dir {
        sha256_dir(path)?
    } else {
        sha256_file(path)?
    };
    if actual != expected {
        bail!(
            "BundleHashMismatch: {label} expected {} but found {} for `{}`",
            expected,
            actual,
            path.display()
        );
    }
    Ok(())
}

#[repr(C)]
#[derive(Default)]
struct CorvidApprovalRequired {
    site_name: *const c_char,
    predicate_json: *const c_char,
    args_json: *const c_char,
    rationale_prompt: *const c_char,
}

type CorvidCallAgentFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    usize,
    *mut *mut c_char,
    *mut usize,
    *mut u64,
    *mut CorvidApprovalRequired,
) -> u32;

type CorvidFreeResultFn = unsafe extern "C" fn(*mut c_char);
type CorvidObservationReleaseFn = unsafe extern "C" fn(u64);

struct ReplayOutput {
    agent: String,
    result_json: String,
    observation_present: bool,
}

unsafe fn replay_library_trace(library_path: &Path, trace_path: &Path) -> Result<ReplayOutput> {
    let events = read_events_from_path(trace_path)
        .with_context(|| format!("read trace `{}`", trace_path.display()))?;
    validate_supported_schema(&events)
        .with_context(|| format!("validate trace `{}`", trace_path.display()))?;
    let (agent, args) = last_run_started(&events)?;
    let deterministic_seed = derive_deterministic_seed(&events);
    let replay_model = last_recorded_model(&events);

    let deterministic_seed_string = deterministic_seed.to_string();
    let trace_guard = EnvGuard::set(&[
        ("CORVID_REPLAY_TRACE_PATH", Some(trace_path.as_os_str())),
        ("CORVID_TRACE_DISABLE", Some(std::ffi::OsStr::new("1"))),
        (
            "CORVID_DETERMINISTIC_SEED",
            Some(std::ffi::OsStr::new(&deterministic_seed_string)),
        ),
    ]);
    let model_guard = replay_model.as_deref().map(|model| {
        EnvGuard::set(&[("CORVID_MODEL", Some(std::ffi::OsStr::new(model)))])
    });

    // `libloading::Library::new` calls `dlopen(path, RTLD_LAZY)` by
    // default. When this `Library` value drops at the end of the
    // function, libloading calls `dlclose`, which unmaps the
    // cdylib. That unmap invalidates every TLS destructor the
    // cdylib's Rust code registered via `__cxa_thread_atexit_impl`
    // (the destructor function pointers live in the cdylib's
    // `.text` section). Tokio worker threads the cdylib spawned
    // are still alive at that point; when they later exit,
    // glibc's `__call_tls_dtors` jumps to those now-dangling
    // pointers and the process dies with SIGSEGV at ip=0 inside
    // `__call_tls_dtors` (cxa_thread_atexit_impl.c:156).
    //
    // The standard fix is `RTLD_NODELETE` — the cdylib stays
    // mapped after `dlclose`, so its TLS destructor pointers
    // remain valid for the process lifetime. The OS reclaims the
    // mapping at process exit. We use `libloading::os::unix::Library::open`
    // with explicit flags on Unix; on Windows DLLs handle TLS
    // teardown at the OS level and the default flags are fine.
    let library = {
        #[cfg(unix)]
        {
            // RTLD_LAZY = 0x1, RTLD_NODELETE = 0x1000 on every
            // glibc + musl we ship to. libloading doesn't expose
            // the constants through `os::unix`, so we pass the
            // raw bitmask.
            const RTLD_LAZY: std::os::raw::c_int = 0x1;
            const RTLD_NODELETE: std::os::raw::c_int = 0x1000;
            let lib = unsafe {
                libloading::os::unix::Library::open(
                    Some(library_path),
                    RTLD_LAZY | RTLD_NODELETE,
                )
            }
            .with_context(|| {
                format!("load rebuilt library `{}`", library_path.display())
            })?;
            libloading::Library::from(lib)
        }
        #[cfg(not(unix))]
        {
            libloading::Library::new(library_path)
                .with_context(|| format!("load rebuilt library `{}`", library_path.display()))?
        }
    };
    let call_agent: libloading::Symbol<CorvidCallAgentFn> = library
        .get(b"corvid_call_agent")
        .context("resolve corvid_call_agent")?;
    let free_result: libloading::Symbol<CorvidFreeResultFn> = library
        .get(b"corvid_free_result")
        .context("resolve corvid_free_result")?;
    let observation_release: Option<libloading::Symbol<CorvidObservationReleaseFn>> =
        library.get(b"corvid_observation_release").ok();

    let args_json = serde_json::Value::Array(args).to_string();
    let agent_c = CString::new(agent.clone()).context("agent name contained NUL")?;
    let args_c = CString::new(args_json).context("args JSON contained NUL")?;
    let mut result_ptr: *mut c_char = std::ptr::null_mut();
    let mut result_len = 0usize;
    let mut observation_handle = 0u64;
    let mut approval = CorvidApprovalRequired::default();
    let status = call_agent(
        agent_c.as_ptr(),
        args_c.as_ptr(),
        args_c.as_bytes().len(),
        &mut result_ptr,
        &mut result_len,
        &mut observation_handle,
        &mut approval,
    );
    if status != 0 {
        bail!(
            "BundleReplayMismatch: replayed library returned status {} for trace `{}`",
            status,
            trace_path.display()
        );
    }
    let result_json = if !result_ptr.is_null() {
        let bytes = std::slice::from_raw_parts(result_ptr as *const u8, result_len);
        let text = String::from_utf8_lossy(bytes).into_owned();
        free_result(result_ptr);
        text
    } else {
        "null".to_string()
    };
    if let Some(release) = observation_release {
        if observation_handle != 0 {
            release(observation_handle);
        }
    }
    drop(model_guard);
    drop(trace_guard);

    let output = ReplayOutput {
        agent,
        result_json,
        observation_present: observation_handle != 0,
    };

    // Explicitly drop the function-pointer symbols before
    // forgetting the library so the borrows end. `observation_release`
    // was already consumed by the `if let Some(release) = ...` above.
    drop(call_agent);
    drop(free_result);

    // Leak the library handle entirely. The previous attempt at
    // `RTLD_NODELETE` alone was not sufficient: even with the
    // mapping pinned, the `dlclose` codepath in glibc's
    // `_dl_close_worker` walks every thread's TLS destructor
    // list and may mark destructor function pointers for the
    // closing DSO as cleared (via PTR_MANGLE'd NULL stores), so
    // a later `__call_tls_dtors` on the corvid-cli worker
    // thread crashes when iterating those entries (the GDB
    // backtrace from the bundle_rebuild Linux CI coredump
    // confirmed exactly this: `__GI___call_tls_dtors () at
    // cxa_thread_atexit_impl.c:156`, ip=0, in the corvid-cli
    // worker after RTLD_NODELETE landed).
    //
    // By `std::mem::forget`ing the Library handle, `dlclose` is
    // never invoked by us at all, so glibc's destructor-clearing
    // codepath never runs. The OS reclaims the mapping at
    // process exit. The downside is a one-time leak per
    // `bundle verify --rebuild` invocation; corvid CLI is not a
    // long-running daemon, so accumulation isn't a concern.
    std::mem::forget(library);

    Ok(output)
}

fn last_run_started(events: &[TraceEvent]) -> Result<(String, Vec<serde_json::Value>)> {
    events
        .iter()
        .find_map(|event| match event {
            TraceEvent::RunStarted { agent, args, .. } => Some((agent.clone(), args.clone())),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("trace had no run_started event"))
}

fn derive_deterministic_seed(events: &[TraceEvent]) -> u64 {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            TraceEvent::SeedRead { purpose, value, .. } if purpose == "rollout_default_seed" => {
                Some(*value)
            }
            _ => None,
        })
        .or_else(|| {
            events.iter().find_map(|event| match event {
                TraceEvent::SchemaHeader { ts_ms, .. } => Some(*ts_ms),
                _ => None,
            })
        })
        .unwrap_or(0)
}

fn last_recorded_model(events: &[TraceEvent]) -> Option<String> {
    events.iter().rev().find_map(|event| match event {
        TraceEvent::LlmCall {
            model: Some(model), ..
        }
        | TraceEvent::LlmResult {
            model: Some(model), ..
        } => Some(model.clone()),
        _ => None,
    })
}

struct EnvGuard {
    saved: Vec<(String, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn set(entries: &[(&str, Option<&std::ffi::OsStr>)]) -> Self {
        let mut saved = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            saved.push(((*key).to_string(), std::env::var_os(key)));
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(&key, value),
                None => std::env::remove_var(&key),
            }
        }
    }
}
