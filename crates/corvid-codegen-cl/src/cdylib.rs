use crate::errors::CodegenError;
use crate::link::binary_path_for;
use crate::target::{
    object_extension, shared_library_path_for, static_library_path_for, BuildTarget,
};
use corvid_ir::{IrExternAbi, IrFile};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn build_library_to_disk(
    ir: &IrFile,
    module_name: &str,
    requested_path: &Path,
    build_target: BuildTarget,
    extra_tool_libs: &[&Path],
    embedded_descriptor: Option<&[u8]>,
    embedded_attestation: Option<&[u8]>,
) -> Result<PathBuf, CodegenError> {
    if !ir
        .agents
        .iter()
        .any(|agent| matches!(agent.extern_abi, Some(IrExternAbi::C)))
    {
        // Slice 33Q12c (maintainer-as-reviewer-2026-06-05 Minor):
        // anchor the diagnostic at the first agent's declaration
        // span when the file HAS any agents (so the reviewer sees
        // "add `pub extern \"c\"` before this agent" rendered
        // inline) instead of the prior `0..0` zero-width anchor at
        // the file start. Falls back to `0..0` only when the file
        // declares no agents at all — at that point there's no
        // useful source location to point at and the message text
        // is the only signal.
        let anchor_span = ir
            .agents
            .first()
            .map(|a| a.span)
            .unwrap_or_else(|| corvid_ast::Span::new(0, 0));
        return Err(CodegenError::not_supported(
            "library targets (cdylib / staticlib) need at least one \
             `pub extern \"c\"` agent — the C-ABI entry point the \
             linker exports. Add `pub extern \"c\"` to an agent that \
             takes scalar parameters (Int / Float / Bool / String) and \
             returns a scalar / `Grounded<scalar>` / `Nothing`. See \
             `docs/reference/exported-abi.md` for the full ABI surface",
            anchor_span,
        ));
    }

    let out_path = match build_target {
        BuildTarget::Cdylib => shared_library_path_for(
            requested_path.parent().unwrap_or(Path::new(".")),
            requested_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(module_name),
        ),
        BuildTarget::Staticlib => static_library_path_for(
            requested_path.parent().unwrap_or(Path::new(".")),
            requested_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(module_name),
        ),
        BuildTarget::Native => binary_path_for(
            requested_path.parent().unwrap_or(Path::new(".")),
            requested_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(module_name),
        ),
    };

    let obj_dir = tempfile::Builder::new()
        .prefix("corvid-lib-obj-")
        .tempdir()
        .map_err(|e| CodegenError::io(format!("tempdir: {e}")))?;
    let object_path = obj_dir
        .path()
        .join(format!("{module_name}.{}", object_extension()));
    crate::compile_to_object(
        ir,
        module_name,
        &object_path,
        None,
        embedded_descriptor,
        embedded_attestation,
        // Library target: dispatch tool calls through the runtime
        // registry so the cdylib/staticlib links without the host's
        // tools (the host registers them at load).
        true,
    )?;

    match build_target {
        BuildTarget::Cdylib => link_shared_library(
            &object_path,
            &out_path,
            extra_tool_libs,
            exported_symbols(ir, embedded_attestation.is_some()),
        )?,
        BuildTarget::Staticlib => link_static_archive(&object_path, &out_path)?,
        BuildTarget::Native => unreachable!("library builder called with native target"),
    }

    Ok(out_path)
}

fn exported_symbols(ir: &IrFile, has_attestation: bool) -> Vec<String> {
    let mut exports = ir
        .agents
        .iter()
        .filter(|agent| matches!(agent.extern_abi, Some(IrExternAbi::C)))
        .map(|agent| agent.name.clone())
        .collect::<Vec<_>>();
    if ir.agents.iter().any(|agent| {
        matches!(agent.extern_abi, Some(IrExternAbi::C))
            && match &agent.return_ty {
                corvid_types::Type::String => true,
                corvid_types::Type::Grounded(inner) => {
                    matches!(&**inner, corvid_types::Type::String)
                }
                _ => false,
            }
    }) {
        exports.push("corvid_free_string".into());
    }
    if has_attestation {
        exports.push(corvid_abi::CORVID_ABI_ATTESTATION_SYMBOL.to_string());
    }
    exports.extend(
        [
            corvid_abi::CORVID_ABI_DESCRIPTOR_SYMBOL,
            "corvid_abi_descriptor_json",
            "corvid_abi_descriptor_hash",
            "corvid_abi_verify",
            "corvid_list_agents",
            "corvid_find_agents_where",
            "corvid_agent_signature_json",
            "corvid_pre_flight",
            "corvid_call_agent",
            "corvid_free_result",
            "corvid_grounded_sources",
            "corvid_grounded_confidence",
            "corvid_grounded_release",
            "corvid_observation_cost_usd",
            "corvid_begin_direct_observation",
            "corvid_finish_direct_observation",
            "corvid_observation_latency_ms",
            "corvid_observation_tokens_in",
            "corvid_observation_tokens_out",
            "corvid_observation_exceeded_bound",
            "corvid_observation_release",
            "corvid_record_host_event",
            "corvid_register_approver",
            "corvid_register_approver_from_source",
            "corvid_clear_approver",
            "corvid_mark_preapproved_request",
            "corvid_approval_predicate_json",
            "corvid_evaluate_approval_predicate",
            // Host-registered tools (G0-tools): a host loading the cdylib
            // registers tool implementations through these, which the
            // codegen-emitted tool calls dispatch to.
            "corvid_register_tool",
            "corvid_clear_tools",
        ]
        .into_iter()
        .map(str::to_string),
    );
    exports
}

fn link_shared_library(
    object_path: &Path,
    output_path: &Path,
    extra_tool_libs: &[&Path],
    exported_symbols: Vec<String>,
) -> Result<(), CodegenError> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CodegenError::io(format!("create {}: {e}", parent.display())))?;
    }
    let compiler = cc::Build::new()
        .opt_level(2)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .host(&target_lexicon::HOST.to_string())
        .target(&target_lexicon::HOST.to_string())
        .try_get_compiler()
        .map_err(|e| CodegenError::link(format!("compiler discovery: {e}")))?;

    let runtime_staticlib_path = runtime_staticlib_path(&compiler)?;
    let link_standalone_runtime = extra_tool_libs.is_empty();
    let mut cmd = Command::new(compiler.path());
    for (k, v) in compiler.env() {
        cmd.env(k, v);
    }

    if compiler.is_like_msvc() {
        cmd.arg("/LD")
            .arg(object_path)
            .arg(format!("/Fe:{}", output_path.display()));
        if link_standalone_runtime {
            cmd.arg(&runtime_staticlib_path);
        }
        for lib in extra_tool_libs {
            cmd.arg(lib);
        }
        cmd.arg("/link")
            .arg(format!(
                "/IMPLIB:{}",
                output_path.with_extension("lib").display()
            ))
            // Make the PE deterministic so bundle rebuild verification
            // can compare committed and rebuilt shared libraries
            // byte-for-byte on MSVC hosts.
            .arg("/BREPRO")
            // `secur32.lib` is needed by the bundled `whoami` crate
            // (`GetUserNameExW` import). Mirror of the same fix in
            // `link.rs::link_binary` — Phase 35V-T1-H landed both
            // call sites together so the bilateral-verifier test
            // run that 35-H ships as evidence can actually link
            // its temporary cdylib on Windows.
            .arg("secur32.lib")
            .arg("bcrypt.lib")
            .arg("advapi32.lib")
            .arg("kernel32.lib")
            .arg("ntdll.lib")
            .arg("userenv.lib")
            .arg("ws2_32.lib")
            .arg("dbghelp.lib")
            .arg("legacy_stdio_definitions.lib");
        for symbol in exported_symbols {
            cmd.arg(format!("/EXPORT:{symbol}"));
        }
    } else {
        cmd.arg("-shared").arg(object_path);
        if link_standalone_runtime {
            cmd.arg(&runtime_staticlib_path);
        }
        // Tool staticlibs are linked positionally. The
        // `#[tool]` proc-macro's `inventory::submit!`
        // constructors get dead-stripped by GNU `ld` in this
        // mode, so the inventory-walked runtime tool registry
        // ends up empty at dlopen — agents that call a tool
        // then panic with `corvid tool <name> is not registered`.
        //
        // The previous formulation wrapped each lib in
        // `-Wl,--whole-archive` to force-include the inventory
        // entries. That worked for thin tool libs but broke the
        // test-time setup where `CORVID_RUNTIME_STATICLIB_OVERRIDE`
        // points the runtime override at a tools lib that
        // transitively bundles all of corvid-runtime — forcing
        // every object in that archive into the cdylib pulled
        // in static constructors that fired before
        // `corvid_runtime_init`, surfacing as silent
        // process-aborts on Linux (no panic message, empty
        // stderr).
        //
        // The architecturally correct integration is for the
        // host to register tools explicitly via the
        // `corvid_register_tool` C ABI after dlopen — see the
        // `examples/cdylib_catalog_demo/host_c/host.c` change
        // that lands alongside this commit for the pattern.
        // The `#[tool]` macro still emits `__corvid_tool_<name>`
        // wrappers that the host dlsym's, so the host's
        // registration call points at the cdylib's own
        // implementation of the tool.
        for lib in extra_tool_libs {
            cmd.arg(lib);
        }
        cmd.arg("-lpthread").arg("-ldl").arg("-lm");
        if cfg!(target_os = "macos") {
            cmd.arg("-framework").arg("Security");
            cmd.arg("-framework").arg("CoreFoundation");
            cmd.arg("-framework").arg("SystemConfiguration");
        } else if cfg!(target_os = "linux") {
            cmd.arg("-lutil");
        }
        cmd.arg("-o").arg(output_path);
    }

    let output = cmd.output().map_err(|e| {
        CodegenError::link(format!("spawn linker `{}`: {e}", compiler.path().display()))
    })?;
    if !output.status.success() {
        return Err(CodegenError::link(format!(
            "shared-library linker exited {}: {}{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        )));
    }
    Ok(())
}

fn link_static_archive(object_path: &Path, output_path: &Path) -> Result<(), CodegenError> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CodegenError::io(format!("create {}: {e}", parent.display())))?;
    }
    if cfg!(windows) {
        let compiler = cc::Build::new()
            .opt_level(2)
            .cargo_metadata(false)
            .cargo_warnings(false)
            .host(&target_lexicon::HOST.to_string())
            .target(&target_lexicon::HOST.to_string())
            .try_get_compiler()
            .map_err(|e| CodegenError::link(format!("compiler discovery: {e}")))?;
        let lib_exe = compiler.path().with_file_name("lib.exe");
        let output = Command::new(&lib_exe)
            .arg(format!("/OUT:{}", output_path.display()))
            .arg(object_path)
            .output()
            .map_err(|e| {
                CodegenError::link(format!(
                    "spawn static librarian `{}`: {e}",
                    lib_exe.display()
                ))
            })?;
        if !output.status.success() {
            return Err(CodegenError::link(format!(
                "static librarian exited {}: {}{}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout),
            )));
        }
        Ok(())
    } else {
        let output = Command::new("ar")
            .arg("crus")
            .arg(output_path)
            .arg(object_path)
            .output()
            .map_err(|e| CodegenError::link(format!("spawn `ar`: {e}")))?;
        if !output.status.success() {
            return Err(CodegenError::link(format!(
                "ar exited {}: {}{}",
                output.status,
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout),
            )));
        }
        Ok(())
    }
}

fn runtime_staticlib_path(compiler: &cc::Tool) -> Result<PathBuf, CodegenError> {
    // Tests can route through `CORVID_RUNTIME_STATICLIB_OVERRIDE` to
    // pick a Rust staticlib that already bundles `corvid-runtime`
    // transitively (e.g. `corvid_test_tools.lib`). Linking the
    // override on its own avoids the duplicate-`std` LNK2005 that
    // pairing it with the standalone `corvid_runtime.lib` would
    // produce on MSVC. Outside tests this stays unset and the
    // default lib is built and used.
    if let Some(override_path) = std::env::var_os("CORVID_RUNTIME_STATICLIB_OVERRIDE") {
        let path = PathBuf::from(override_path);
        if !path.exists() {
            return Err(CodegenError::link(format!(
                "CORVID_RUNTIME_STATICLIB_OVERRIDE points at non-existent path `{}`",
                path.display()
            )));
        }
        return Ok(path);
    }
    let runtime_lib_name = if compiler.is_like_msvc() {
        "corvid_runtime.lib"
    } else {
        "libcorvid_runtime.a"
    };
    // Fast path: the staticlib is already on disk in a location we
    // can discover. Skips the dev-mode auto-build entirely when it
    // isn't needed.
    if let Some(location) = crate::staticlib_discovery::discover_staticlib(runtime_lib_name) {
        return Ok(location.path);
    }
    // Slow path: no staticlib on disk. Fall back to the dev-mode
    // auto-build, writing into the canonical Cargo
    // `target/<profile>/` layout discovered from `current_exe()`.
    let staticlib_dir = crate::staticlib_discovery::guess_dev_target_profile_dir()
        .ok_or_else(|| CodegenError::link(
            "could not locate `corvid-runtime` staticlib and no `target/<profile>/` \
             directory derivable from the running binary's path. Set \
             `CORVID_STATICLIB_DIR` to the directory containing the staticlib \
             or `CORVID_RUNTIME_STATICLIB_OVERRIDE` to its full path."
        ))?;
    let runtime_staticlib_path = staticlib_dir.join(runtime_lib_name);
    build_runtime_staticlib(&staticlib_dir, &runtime_staticlib_path)?;
    if !runtime_staticlib_path.exists() {
        return Err(CodegenError::link(format!(
            "corvid-runtime staticlib missing at `{}` after auto-build.",
            runtime_staticlib_path.display()
        )));
    }
    Ok(runtime_staticlib_path)
}

fn build_runtime_staticlib(
    staticlib_dir: &Path,
    runtime_staticlib_path: &Path,
) -> Result<(), CodegenError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .ancestors()
        .nth(2)
        .ok_or_else(|| CodegenError::link("locate workspace root for corvid-runtime build"))?;
    let profile = staticlib_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg("corvid-runtime");
    if profile.eq_ignore_ascii_case("release") {
        cmd.arg("--release");
    }
    let output = cmd.current_dir(workspace_root).output().map_err(|err| {
        CodegenError::link(format!("spawn `cargo build -p corvid-runtime`: {err}"))
    })?;
    if !output.status.success() {
        return Err(CodegenError::link(format!(
            "auto-build of corvid-runtime staticlib for `{}` failed: {}{}",
            runtime_staticlib_path.display(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        )));
    }
    Ok(())
}
