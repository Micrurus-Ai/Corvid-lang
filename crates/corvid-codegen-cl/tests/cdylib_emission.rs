use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::process::Command;

use corvid_abi::{descriptor_to_embedded_bytes, emit_catalog_abi, EmitOptions};
use corvid_ast::File;
use corvid_c_header::{emit_header, HeaderOptions};
use corvid_codegen_cl::{build_library_to_disk, BuildTarget};
use corvid_ir::lower;
use corvid_resolve::{resolve, Resolved};
use corvid_syntax::{lex, parse_file};
use corvid_types::{typecheck, Checked, EffectRegistry};
use libloading::Library;

const BOOL_SRC: &str = r#"
pub extern "c"
agent refund_bot(ticket_id: String, amount: Float) -> Bool:
    return ticket_id == "vip" and amount > 10.0
"#;

const STRING_SRC: &str = r#"
pub extern "c"
agent echo_name(name: String) -> String:
    return name
"#;

const FLOAT_SRC: &str = r#"
pub extern "c"
agent echo_amount(amount: Float) -> Float:
    return amount
"#;

const GROUNDED_STRING_SRC: &str = r#"
effect retrieval:
    data: grounded

tool grounded_echo(name: String) -> Grounded<String> uses retrieval

pub extern "c"
agent grounded_lookup(name: String) -> Grounded<String>:
    return grounded_echo(name)
"#;

struct FrontendBundle {
    file: File,
    resolved: Resolved,
    checked: Checked,
    effect_registry: EffectRegistry,
    ir: corvid_ir::IrFile,
}

fn frontend_of(src: &str) -> FrontendBundle {
    let tokens = lex(src).expect("lex");
    let (file, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
    let resolved = resolve(&file);
    assert!(
        resolved.errors.is_empty(),
        "resolve errors: {:?}",
        resolved.errors
    );
    let checked = typecheck(&file, &resolved);
    assert!(
        checked.errors.is_empty(),
        "type errors: {:?}",
        checked.errors
    );
    let effect_decls = file
        .decls
        .iter()
        .filter_map(|decl| match decl {
            corvid_ast::Decl::Effect(effect) => Some(effect.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let effect_registry = EffectRegistry::from_decls(&effect_decls);
    let ir = lower(&file, &resolved, &checked);
    FrontendBundle {
        file,
        resolved,
        checked,
        effect_registry,
        ir,
    }
}

fn embedded_descriptor_bytes(bundle: &FrontendBundle, src: &str) -> Vec<u8> {
    let descriptor = emit_catalog_abi(
        &bundle.file,
        &bundle.resolved,
        &bundle.checked,
        &bundle.ir,
        &bundle.effect_registry,
        &EmitOptions {
            source_path: "tests/cdylib_emission.cor",
            source_text: src,
            compiler_version: "0.6.0-phase22",
            generated_at: "1970-01-01T00:00:00Z",
        },
    );
    descriptor_to_embedded_bytes(&descriptor).expect("embed descriptor")
}

fn build_cdylib(src: &str, stem: &str) -> PathBuf {
    let bundle = frontend_of(src);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join(stem);
    let embedded = embedded_descriptor_bytes(&bundle, src);
    let produced = build_library_to_disk(
        &bundle.ir,
        stem,
        &out,
        BuildTarget::Cdylib,
        &[],
        Some(embedded.as_slice()),
        None,
    )
    .expect("build cdylib");
    let keep = tmp.keep();
    assert!(keep.exists());
    produced
}

fn test_tools_lib_path() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf();
    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("corvid-test-tools")
        .arg("--release")
        .current_dir(&root)
        .status()
        .expect("build corvid-test-tools");
    assert!(status.success(), "building corvid-test-tools failed");
    let path = if cfg!(windows) {
        root.join("target")
            .join("release")
            .join("corvid_test_tools.lib")
    } else {
        root.join("target")
            .join("release")
            .join("libcorvid_test_tools.a")
    };
    // Route the linker through `corvid_test_tools.lib` (which already
    // bundles `corvid-runtime` transitively) instead of pairing it
    // with the standalone `corvid_runtime.lib`. See
    // `corvid-codegen-cl::cdylib::runtime_staticlib_path`.
    unsafe {
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", &path);
    }
    path
}

fn build_cdylib_with_extra_libs(src: &str, stem: &str, extra_libs: &[PathBuf]) -> PathBuf {
    let bundle = frontend_of(src);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join(stem);
    let embedded = embedded_descriptor_bytes(&bundle, src);
    let extra_lib_refs = extra_libs
        .iter()
        .map(|path| path.as_path())
        .collect::<Vec<_>>();
    let produced = build_library_to_disk(
        &bundle.ir,
        stem,
        &out,
        BuildTarget::Cdylib,
        &extra_lib_refs,
        Some(embedded.as_slice()),
        None,
    )
    .expect("build cdylib");
    let keep = tmp.keep();
    assert!(keep.exists());
    produced
}

fn build_staticlib(src: &str, stem: &str) -> PathBuf {
    let bundle = frontend_of(src);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join(stem);
    let produced = build_library_to_disk(
        &bundle.ir,
        stem,
        &out,
        BuildTarget::Staticlib,
        &[],
        None,
        None,
    )
    .expect("build staticlib");
    let keep = tmp.keep();
    assert!(keep.exists());
    produced
}

fn load_library_leaked(path: &Path) -> &'static Library {
    // SAFETY: tests load a freshly-built library and intentionally keep it
    // resident for the life of the process. Repeated DLL unloads have been
    // flaky on Windows once the embedded runtime spins background state, and
    // that teardown noise is outside the ABI behavior this test is asserting.
    unsafe { Box::leak(Box::new(Library::new(path).expect("load shared library"))) }
}

#[test]
fn cdylib_target_produces_shared_library_file() {
    let produced = build_cdylib(BOOL_SRC, "refund_bot_cdylib");
    assert!(
        produced.exists(),
        "missing shared library: {}",
        produced.display()
    );
}

#[test]
fn cdylib_symbol_is_resolvable_via_dlopen() {
    let produced = build_cdylib(BOOL_SRC, "refund_bot_symbol");
    // SAFETY: test loads a library we just built and requests a known symbol.
    unsafe {
        let lib = load_library_leaked(&produced);
        let _: libloading::Symbol<unsafe extern "C" fn(*const c_char, f64, *mut u64) -> bool> =
            lib.get(b"refund_bot").expect("resolve symbol");
    }
}

#[test]
fn staticlib_target_produces_archive_file() {
    let produced = build_staticlib(BOOL_SRC, "refund_bot_static");
    assert!(produced.exists(), "missing archive: {}", produced.display());
    if cfg!(windows) {
        let compiler = cc::Build::new()
            .opt_level(0)
            .cargo_metadata(false)
            .cargo_warnings(false)
            .host(&target_lexicon::HOST.to_string())
            .target(&target_lexicon::HOST.to_string())
            .try_get_compiler()
            .expect("compiler");
        let lib_exe = compiler.path().with_file_name("lib.exe");
        let output = Command::new(lib_exe)
            .arg("/LIST")
            .arg(&produced)
            .output()
            .expect("list archive");
        assert!(output.status.success(), "lib /LIST failed");
    } else {
        let output = Command::new("ar")
            .arg("-t")
            .arg(&produced)
            .output()
            .expect("list archive");
        assert!(output.status.success(), "ar -t failed");
    }
}

#[test]
fn cdylib_minimal_c_harness_calls_and_returns_correct_value() {
    let bundle = frontend_of(BOOL_SRC);
    let tmp = tempfile::tempdir().unwrap();
    let stem = "refund_bot_harness";
    let requested = tmp.path().join(stem);
    let embedded = embedded_descriptor_bytes(&bundle, BOOL_SRC);
    let lib_path = build_library_to_disk(
        &bundle.ir,
        stem,
        &requested,
        BuildTarget::Cdylib,
        &[],
        Some(embedded.as_slice()),
        None,
    )
    .expect("build cdylib");
    let header = emit_header(
        &bundle.ir,
        &HeaderOptions {
            library_name: stem.into(),
        },
    );
    let header_path = tmp.path().join(format!("lib_{stem}.h"));
    std::fs::write(&header_path, header).unwrap();

    let harness_path = tmp.path().join("harness.c");
    std::fs::write(&harness_path, c_harness_source(&header_path, &lib_path)).unwrap();
    let harness_bin = compile_c_harness(&harness_path, tmp.path());

    let output = Command::new(&harness_bin).output().expect("run c harness");
    assert!(
        output.status.success(),
        "c harness failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ok"));
}

#[test]
fn cdylib_string_param_roundtrip() {
    let produced = build_cdylib(STRING_SRC, "echo_name_cdylib");
    // SAFETY: symbols are loaded from the just-built library and invoked with valid ABI values.
    unsafe {
        let lib = load_library_leaked(&produced);
        let echo: libloading::Symbol<
            unsafe extern "C" fn(*const c_char, *mut u64) -> *const c_char,
        > = lib.get(b"echo_name").expect("resolve echo_name");
        let free: libloading::Symbol<unsafe extern "C" fn(*const c_char)> = lib
            .get(b"corvid_free_string")
            .expect("resolve corvid_free_string");
        let input = CString::new("Grüße").unwrap();
        let mut observation = 0u64;
        let output_ptr = echo(input.as_ptr(), &mut observation as *mut u64);
        assert_ne!(observation, 0);
        let output = CStr::from_ptr(output_ptr).to_str().unwrap().to_owned();
        free(output_ptr);
        assert_eq!(output, "Grüße");
    }
}

#[test]
fn cdylib_float_precision_preserved() {
    let produced = build_cdylib(FLOAT_SRC, "echo_amount_cdylib");
    // SAFETY: symbol is loaded from the just-built library and invoked with a valid f64.
    unsafe {
        let lib = load_library_leaked(&produced);
        let echo: libloading::Symbol<unsafe extern "C" fn(f64, *mut u64) -> f64> =
            lib.get(b"echo_amount").expect("resolve echo_amount");
        let input = 0.12345678912345678_f64;
        let mut observation = 0u64;
        let output = echo(input, &mut observation as *mut u64);
        assert_ne!(observation, 0);
        assert_eq!(output.to_bits(), input.to_bits());
    }
}

#[test]
fn cdylib_bool_maps_to_c99_bool_size() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("bool_size.c");
    std::fs::write(
        &source,
        "#include <stdbool.h>\nint main(void) { return sizeof(bool) == 1 ? 0 : 1; }\n",
    )
    .unwrap();
    let bin = compile_c_harness(&source, tmp.path());
    let output = Command::new(bin).output().expect("run bool size harness");
    assert!(output.status.success());
}

#[test]
fn cdylib_grounded_string_return_exposes_attestation_handle() {
    let tools_lib = test_tools_lib_path();
    let produced =
        build_cdylib_with_extra_libs(GROUNDED_STRING_SRC, "grounded_lookup_cdylib", &[tools_lib]);
    // SAFETY: symbols are loaded from the just-built library and invoked with valid ABI values.
    unsafe {
        let lib = load_library_leaked(&produced);
        let grounded_lookup: libloading::Symbol<
            unsafe extern "C" fn(*const c_char, *mut u64, *mut u64) -> *const c_char,
        > = lib
            .get(b"grounded_lookup")
            .expect("resolve grounded_lookup");
        let grounded_sources: libloading::Symbol<
            unsafe extern "C" fn(u64, *mut *const c_char, usize) -> i32,
        > = lib
            .get(b"corvid_grounded_sources")
            .expect("resolve corvid_grounded_sources");
        let grounded_confidence: libloading::Symbol<unsafe extern "C" fn(u64) -> f64> = lib
            .get(b"corvid_grounded_confidence")
            .expect("resolve corvid_grounded_confidence");
        let grounded_release: libloading::Symbol<unsafe extern "C" fn(u64)> = lib
            .get(b"corvid_grounded_release")
            .expect("resolve corvid_grounded_release");
        let free: libloading::Symbol<unsafe extern "C" fn(*const c_char)> = lib
            .get(b"corvid_free_string")
            .expect("resolve corvid_free_string");

        // The cdylib agent dispatches `grounded_echo` through
        // the runtime tool registry (target-conditional dispatch
        // shipped in `dfd98eb`), so the host must register a
        // callback for the tool BEFORE invoking the agent or the
        // cdylib panics at `corvid_invoke_tool_*` with
        // `corvid tool grounded_echo is not registered`. The
        // proc-macro-emitted Rust wrappers in the cdylib itself
        // are dead-stripped by the linker (see the cdylib link
        // path's doc comment for why), so the test provides its
        // own implementation matching the Rust tool's
        // return-the-input semantics. Same architectural shape
        // as the C-host integration in
        // `examples/cdylib_catalog_demo/host_c/host.c`.
        type CorvidToolFn = unsafe extern "C" fn(
            args_json: *const c_char,
            args_len: usize,
            user_data: *mut std::ffi::c_void,
        ) -> *mut c_char;
        unsafe extern "C" fn echo_first_string_arg(
            args_json: *const c_char,
            args_len: usize,
            _user_data: *mut std::ffi::c_void,
        ) -> *mut c_char {
            let bytes = std::slice::from_raw_parts(args_json as *const u8, args_len);
            if bytes.len() < 2 || bytes[0] != b'[' || bytes[bytes.len() - 1] != b']' {
                return std::ptr::null_mut();
            }
            let inner = bytes[1..bytes.len() - 1].to_vec();
            match CString::new(inner) {
                Ok(cstr) => cstr.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        }
        let register_tool: libloading::Symbol<
            unsafe extern "C" fn(
                *const c_char,
                Option<CorvidToolFn>,
                *mut std::ffi::c_void,
            ),
        > = lib
            .get(b"corvid_register_tool")
            .expect("resolve corvid_register_tool");
        let tool_name = CString::new("grounded_echo").unwrap();
        register_tool(
            tool_name.as_ptr(),
            Some(echo_first_string_arg),
            std::ptr::null_mut(),
        );

        let input = CString::new("lookup-me").unwrap();
        let mut handle = 0u64;
        let mut observation = 0u64;
        let output_ptr = grounded_lookup(
            input.as_ptr(),
            &mut handle as *mut u64,
            &mut observation as *mut u64,
        );
        assert_ne!(handle, 0);
        assert_ne!(observation, 0);
        let output = CStr::from_ptr(output_ptr).to_str().unwrap().to_owned();
        assert_eq!(output, "lookup-me");

        let mut source_ptrs = [std::ptr::null(); 4];
        let count = grounded_sources(handle, source_ptrs.as_mut_ptr(), source_ptrs.len());
        assert_eq!(count, 1);
        let source = CStr::from_ptr(source_ptrs[0]).to_str().unwrap().to_owned();
        assert_eq!(source, "grounded_echo");
        let confidence = grounded_confidence(handle);
        assert!((confidence - 1.0).abs() < 1e-9);

        grounded_release(handle);
        free(output_ptr);
    }
}

fn compile_c_harness(source: &Path, out_dir: &Path) -> PathBuf {
    let compiler = cc::Build::new()
        .opt_level(0)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .host(&target_lexicon::HOST.to_string())
        .target(&target_lexicon::HOST.to_string())
        .try_get_compiler()
        .expect("compiler");
    let output_path = if cfg!(windows) {
        out_dir.join("harness.exe")
    } else {
        out_dir.join("harness")
    };
    let mut cmd = Command::new(compiler.path());
    for (k, v) in compiler.env() {
        cmd.env(k, v);
    }
    if compiler.is_like_msvc() {
        cmd.arg(source)
            .arg(format!("/Fe:{}", output_path.display()));
    } else {
        cmd.arg(source)
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("-ldl")
            .arg("-o")
            .arg(&output_path);
    }
    let output = cmd.output().expect("compile c harness");
    assert!(
        output.status.success(),
        "c harness compile failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output_path
}

fn c_harness_source(header_path: &Path, library_path: &Path) -> String {
    let header = header_path.to_string_lossy().replace('\\', "\\\\");
    let library = library_path.to_string_lossy().replace('\\', "\\\\");
    format!(
        r#"#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include "{header}"

#ifdef _WIN32
#include <windows.h>
int main(void) {{
    HMODULE lib = LoadLibraryA("{library}");
    if (!lib) return 1;
    bool (*refund_bot)(const char*, double, uint64_t*) =
        (bool (*)(const char*, double, uint64_t*))GetProcAddress(lib, "refund_bot");
    if (!refund_bot) return 2;
    uint64_t observation = 0;
    if (!refund_bot("vip", 20.0, &observation)) return 3;
    if (observation == 0) return 4;
    FreeLibrary(lib);
    puts("ok");
    return 0;
}}
#else
#include <dlfcn.h>
int main(void) {{
    void* lib = dlopen("{library}", RTLD_NOW);
    if (!lib) return 1;
    bool (*refund_bot)(const char*, double, uint64_t*) =
        (bool (*)(const char*, double, uint64_t*))dlsym(lib, "refund_bot");
    if (!refund_bot) return 2;
    uint64_t observation = 0;
    if (!refund_bot("vip", 20.0, &observation)) return 3;
    if (observation == 0) return 4;
    dlclose(lib);
    puts("ok");
    return 0;
}}
#endif
"#
    )
}

/// 33Q12c acceptance — maintainer-as-reviewer-2026-06-05 Minor.
/// Pre-33Q12c, building a cdylib from a source with NO
/// `pub extern "c"` agent produced this error:
///
/// ```text
/// error: [0..0] native codegen does not yet support: library targets require at least one `pub extern "c"` agent
/// ```
///
/// Two complaints: (1) the `[0..0]` span is a zero-width anchor at
/// the file start — useless for a reviewer trying to locate where
/// to add the keyword, (2) the "not yet support: library targets
/// require..." phrasing reads awkwardly because of the colon's
/// parse. 33Q12c fixes both:
///
/// - When the file has any agent, the diagnostic anchors at the
///   first agent's span so the reviewer's editor can highlight
///   "add `pub extern \"c\"` to this agent".
/// - The message names what the user must do AND references
///   `docs/reference/exported-abi.md` for the full ABI surface.
#[test]
fn cdylib_missing_pub_extern_c_error_anchors_at_first_agent_and_names_doc_page() {
    // Source has one agent that is NOT `pub extern "c"` — the error
    // should fire and anchor at that agent's span.
    const SRC: &str = "agent foo(x: Int) -> Int:\n    return x\n";
    let bundle = frontend_of(SRC);
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("missing_extern_test");

    let err = build_library_to_disk(
        &bundle.ir,
        "missing_extern_test",
        &out,
        BuildTarget::Cdylib,
        &[],
        None,
        None,
    )
    .expect_err("must error when no pub extern \"c\" agent is declared");

    // Assertion 1: error message must point reviewers at the doc page
    // we created in 33Q12c.
    let msg = format!("{err}");
    assert!(
        msg.contains("exported-abi.md"),
        "error message must reference docs/reference/exported-abi.md \
         so the reviewer can read up on the ABI surface. got: {msg}"
    );

    // Assertion 2: the span anchor must NOT be the prior `[0..0]`
    // zero-width-at-file-start. The agent is on the first line so
    // the new anchor falls inside the first ~40 characters.
    let first_agent_span = bundle.ir.agents[0].span;
    assert!(
        !msg.starts_with("[0..0]"),
        "33Q12c MUST point the span away from `[0..0]` toward the \
         first agent's actual location ({first_agent_span:?}). \
         got: {msg}"
    );

    // Assertion 3: the message must name what the operator should
    // do (add `pub extern \"c\"`), not just complain.
    assert!(
        msg.contains("pub extern \"c\""),
        "error must name the missing construct verbatim so the \
         operator can grep for it. got: {msg}"
    );
}

const STRUCT_BOUNDARY_SRC: &str = r#"
type Ticket:
    id: String
    amount: Int

type Receipt:
    ok: Bool
    note: String

pub extern "c"
agent finalize_ticket(ticket: Ticket @borrowed) -> Receipt:
    return Receipt(true, ticket.id)
"#;

/// Slice 33Q8 acceptance — the headline lift. `pub extern "c"` agents
/// with struct parameters AND struct returns must build into a cdylib,
/// dlopen, and round-trip through the C ABI as JSON-encoded strings.
///
/// Pre-33Q8 the typechecker rejected `agent finalize_ticket(ticket:
/// Ticket) -> Receipt` outright with `extern \"c\" agent uses
/// unsupported ABI type `struct` in parameter`. The reviewer's
/// production HTTP backend (any shape that takes a structured
/// request and returns a structured response) was killed at the
/// signed-cdylib boundary. 33Q8 lifts the rejection by reusing
/// 20n-C's struct decoder/encoder at the C ABI boundary: the
/// param arrives as `const char* /* JSON */`, the wrapper decodes
/// it; the return leaves as `const char* /* JSON */`, the caller
/// frees via `corvid_free_string`.
#[test]
fn cdylib_struct_param_and_return_roundtrip_via_json() {
    let produced = build_cdylib(STRUCT_BOUNDARY_SRC, "finalize_ticket_cdylib");
    // SAFETY: symbols are loaded from the just-built library and invoked with valid ABI values.
    unsafe {
        let lib = load_library_leaked(&produced);
        let finalize: libloading::Symbol<
            unsafe extern "C" fn(*const c_char, *mut u64) -> *const c_char,
        > = lib
            .get(b"finalize_ticket")
            .expect("resolve finalize_ticket");
        let free: libloading::Symbol<unsafe extern "C" fn(*const c_char)> = lib
            .get(b"corvid_free_string")
            .expect("resolve corvid_free_string");

        let input = CString::new(r#"{"id":"vip-007","amount":42}"#).unwrap();
        let mut observation = 0u64;
        let output_ptr = finalize(input.as_ptr(), &mut observation as *mut u64);
        assert!(!output_ptr.is_null(), "extern wrapper returned NULL");
        assert_ne!(observation, 0, "observation handle was not populated");
        let output = CStr::from_ptr(output_ptr).to_str().unwrap().to_owned();
        free(output_ptr);

        // Acceptance: the returned JSON parses, has the right
        // fields, and propagates the input id into the note (the
        // agent body returns `Receipt(true, ticket.id)` so the
        // note must equal the input id — proves both decode AND
        // encode actually marshal the user-provided value through
        // the wrapper without dropping it).
        let parsed: serde_json::Value =
            serde_json::from_str(&output).expect("parse returned JSON");
        assert_eq!(parsed["ok"], serde_json::Value::Bool(true));
        assert_eq!(parsed["note"], serde_json::Value::String("vip-007".into()));
    }
}

/// Slice 33Q8 — the emitted C header must document the struct
/// boundary as a `const char*` (JSON) carrying a JSON schema
/// block-comment so a C caller knows what to send / decode without
/// reading the .cor source.
#[test]
fn cdylib_struct_boundary_c_header_documents_json_schema() {
    let bundle = frontend_of(STRUCT_BOUNDARY_SRC);
    let header = emit_header(
        &bundle.ir,
        &HeaderOptions {
            library_name: "finalize_ticket".into(),
        },
    );

    // The parameter must travel as `const char*` not as a C struct
    // mirror.
    assert!(
        header.contains("const char* ticket"),
        "header must declare struct param as `const char*` JSON. got:\n{header}"
    );
    // The return must travel as `const char*` not as a C struct.
    assert!(
        header.contains("const char* finalize_ticket("),
        "header must declare struct return as `const char*` JSON. got:\n{header}"
    );
    // Both param + return must carry a JSON-schema block comment so
    // the C caller knows the shape (per the slice's prompt-format
    // re-use clause).
    assert!(
        header.contains("// JSON shape for parameter `ticket`:"),
        "header must emit parameter JSON-schema comment. got:\n{header}"
    );
    assert!(
        header.contains("// JSON shape for return value `return`:"),
        "header must emit return JSON-schema comment. got:\n{header}"
    );
    // The schemas must reference the actual field names — proves the
    // schema was generated from the real Type, not stubbed.
    assert!(
        header.contains("\"amount\""),
        "schema for parameter must mention the `amount` field. got:\n{header}"
    );
    assert!(
        header.contains("\"note\""),
        "schema for return value must mention the `note` field. got:\n{header}"
    );
}
