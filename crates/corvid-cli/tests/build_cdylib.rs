use std::path::{Path, PathBuf};
use std::process::Command;

fn write_project(src: &str, stem: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    let source_path = src_dir.join(format!("{stem}.cor"));
    std::fs::write(&source_path, src).expect("write source");
    (dir, source_path)
}

fn write_module_project(
    module_src: &str,
    module_stem: &str,
    main_src: &str,
) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("create src dir");
    std::fs::write(src_dir.join(format!("{module_stem}.cor")), module_src).expect("write module");
    let main_path = src_dir.join("main.cor");
    std::fs::write(&main_path, main_src).expect("write main");
    (dir, main_path)
}

fn run_corvid(args: &[&str], cwd: &Path) -> std::process::Output {
    let exe = env!("CARGO_BIN_EXE_corvid");
    Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run corvid")
}

fn shared_library_name(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    }
}

fn static_library_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.lib")
    } else {
        format!("lib{stem}.a")
    }
}

const SCALAR_SRC: &str = r#"
pub extern "c"
agent refund_bot(ticket_id: String, amount: Float) -> Bool:
    return ticket_id == "vip" and amount > 10.0
"#;

// Slice 33Q8 promoted flat scalar-field structs to supported ABI
// shapes (they cross as JSON-encoded C strings), so the offender
// here must be a NESTED struct — still beyond the encoder/decoder
// depth and still rejected by the checker.
const NON_SCALAR_SRC: &str = r#"
type Assignee:
    name: String

type Ticket:
    id: String
    assignee: Assignee

pub extern "c"
agent refund_bot(ticket: Ticket) -> Bool:
    return true
"#;

// A `dangerous` tool with no `.cor` body, called by an agent, plus a
// scalar `pub extern "c"` entrypoint. Before host-registered tool
// dispatch (G0-tools), a cdylib build failed at link with
// `unresolved external symbol __corvid_tool_use_tool` because codegen
// emitted a direct call to the (nonexistent) wrapper symbol. Under
// target-conditional dispatch, a library target routes tool calls
// through the runtime registry instead, so it links without the tool
// implementation baked in — a host registers it at load.
const TOOL_USING_SRC: &str = r#"
effect act:
    cost: $0.01
    trust: human_required
    data: external

tool use_tool(x: String) -> String dangerous uses act

agent run_it(x: String) -> String uses act:
    approve UseTool(x)
    return use_tool(x)

pub extern "c"
agent entry(note: String) -> String:
    return note
"#;

// An imported module that BOTH defines a struct AND has an agent that
// reads its fields. `lower_with_modules` appends the module agent to the
// cdylib's IR, so codegen lowers `describe_receipt`'s `r.id` access on a
// struct local to the module. The module's checker uses its own DefId
// space, so that expression type is `Struct(<module-local id>)` while
// `ir.types` is keyed by the cross-module-remapped id — codegen failed
// with "struct metadata missing for field access to `id`" until
// `lower_expr` remapped module-local struct types too.
const MODULE_FIELD_ACCESS_TYPES_SRC: &str = r#"
public type Receipt:
    id: String
    amount: Int

public agent describe_receipt(r: Receipt) -> String:
    return r.id
"#;

const MODULE_FIELD_ACCESS_MAIN_SRC: &str = r#"
import "./types" as t

agent run(r: t.Receipt) -> String:
    return t.describe_receipt(r)

pub extern "c"
agent entry(note: String) -> String:
    return note
"#;

#[test]
fn cli_build_cdylib_links_module_agent_field_access() {
    let (_dir, source_path) = write_module_project(
        MODULE_FIELD_ACCESS_TYPES_SRC,
        "types",
        MODULE_FIELD_ACCESS_MAIN_SRC,
    );
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "module-agent field-access cdylib failed to build: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join(shared_library_name("main"));
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
}

#[test]
fn cli_build_cdylib_links_tool_using_program_via_registry() {
    let (_dir, source_path) = write_project(TOOL_USING_SRC, "toolprog");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "tool-using cdylib failed to link — registry dispatch should remove the link-time \
         `__corvid_tool_*` dependency: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join(shared_library_name("toolprog"));
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
}

const DESCRIPTOR_SRC: &str = r#"
agent classify(ticket_id: String) -> Option<String>:
    if ticket_id == "vip":
        return Some(ticket_id)
    return None

pub extern "c"
agent refund_bot(ticket_id: String, amount: Float) -> Bool:
    decision = classify(ticket_id)
    return decision != None and amount > 10.0
"#;

// A module exporting a struct, plus a `main` that imports it, takes
// the imported struct as an agent parameter, and reads its fields. This
// is the reference-app shape (agents pass std types like `Actor` /
// `ApprovalContractRef` around). The `pub extern "c"` entrypoint is a
// separate scalar agent — the cdylib gate only needs one, and lowering
// compiles every agent (including the unreferenced `describe`).
const IMPORTED_TYPES_SRC: &str = r#"
public type Receipt:
    id: String
    amount: Int
"#;

const IMPORTED_STRUCT_MAIN_SRC: &str = r#"
import "./types" as t

agent describe(r: t.Receipt) -> String:
    return r.id

pub extern "c"
agent entry(note: String) -> String:
    return note
"#;

#[test]
fn cli_build_cdylib_succeeds_with_imported_struct_field_access() {
    // Slice G0 (imported-struct native codegen): an agent that accepts
    // an imported struct and reads its fields must compile + link as a
    // cdylib. Previously failed with "native codegen does not yet
    // support: imported struct".
    let (_dir, source_path) =
        write_module_project(IMPORTED_TYPES_SRC, "types", IMPORTED_STRUCT_MAIN_SRC);
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "imported-struct cdylib build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join(shared_library_name("main"));
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
}

#[test]
fn cli_build_cdylib_target_succeeds_on_scalar_agent() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join(shared_library_name("refund_bot"));
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
}

#[test]
fn cli_build_staticlib_target_succeeds_on_scalar_agent() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=staticlib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join(static_library_name("refund_bot"));
    assert!(lib_path.exists(), "missing static library: {}", lib_path.display());
}

#[test]
fn cli_build_cdylib_with_header_flag_writes_header_alongside_lib() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
            "--header",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let release_dir = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release");
    let lib_path = release_dir.join(shared_library_name("refund_bot"));
    let header_path = release_dir.join("lib_refund_bot.h");
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
    assert!(header_path.exists(), "missing header: {}", header_path.display());

    let header = std::fs::read_to_string(&header_path).expect("read header");
    // The header emitter appends an explicit `uint64_t*
    // out_observation_handle` out-parameter to every extern "c"
    // agent so callers from C can correlate the call with the
    // runtime's observation handle without a second
    // introspection round-trip. The pre-observation-handle
    // signature `bool refund_bot(…, double amount);` is gone —
    // see `corvid-c-header/src/lib.rs:57-61`.
    assert!(
        header.contains(
            "bool refund_bot(const char* ticket_id, double amount, uint64_t* out_observation_handle);"
        ),
        "header missing observation-handle signature: {header}"
    );
}

#[test]
fn cli_build_cdylib_fails_cleanly_on_non_scalar_signature() {
    let (_dir, source_path) = write_project(NON_SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The diagnostic must name the ABI violation AND surface the
    // actionable help enumerating the supported boundary set (the
    // help's old phase-numbered phrasing is gone; assert the
    // stable behavioral wording).
    assert!(
        stderr.contains("unsupported ABI type"),
        "stderr missing the ABI violation message: {stderr}"
    );
    assert!(
        stderr.contains("extern \"c\" accepts"),
        "stderr missing the supported-ABI-set help text: {stderr}"
    );
}

#[test]
fn cli_build_cdylib_with_abi_descriptor_flag_writes_json_alongside_library() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
            "--abi-descriptor",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let release_dir = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release");
    let lib_path = release_dir.join(shared_library_name("refund_bot"));
    let descriptor_path = release_dir.join("refund_bot.corvid-abi.json");
    assert!(lib_path.exists(), "missing shared library: {}", lib_path.display());
    assert!(
        descriptor_path.exists(),
        "missing abi descriptor: {}",
        descriptor_path.display()
    );

    let descriptor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&descriptor_path).expect("read descriptor"))
            .expect("parse descriptor");
    assert_eq!(descriptor["corvid_abi_version"], serde_json::json!(1));
    assert_eq!(descriptor["agents"][0]["name"], serde_json::json!("refund_bot"));
}

#[test]
fn cli_build_cdylib_without_abi_descriptor_flag_does_not_write_json() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let descriptor_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join("refund_bot.corvid-abi.json");
    assert!(
        !descriptor_path.exists(),
        "unexpected abi descriptor: {}",
        descriptor_path.display()
    );
}

#[test]
fn cli_build_cdylib_with_abi_descriptor_on_non_scalar_return_still_succeeds() {
    let (_dir, source_path) = write_project(DESCRIPTOR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
            "--abi-descriptor",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let descriptor_path = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release")
        .join("refund_bot.corvid-abi.json");
    let descriptor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&descriptor_path).expect("read descriptor"))
            .expect("parse descriptor");
    assert!(
        descriptor["agents"]
            .as_array()
            .expect("agent array")
            .iter()
            .any(|agent| {
                agent["name"] == serde_json::json!("refund_bot")
                    && agent["return_type"]["scalar"] == serde_json::json!("Bool")
            })
    );
    assert!(
        descriptor["agents"]
            .as_array()
            .expect("agent array")
            .iter()
            .any(|agent| {
                agent["name"] == serde_json::json!("classify")
                    && agent["return_type"]["option"]["inner"]["scalar"] == serde_json::json!("String")
            })
    );
}

#[test]
fn all_artifacts_flag_writes_lib_header_and_descriptor() {
    let (_dir, source_path) = write_project(SCALAR_SRC, "refund_bot");
    let output = run_corvid(
        &[
            "build",
            source_path.to_str().expect("utf8 source path"),
            "--target=cdylib",
            "--all-artifacts",
        ],
        source_path.parent().unwrap(),
    );

    assert!(
        output.status.success(),
        "build failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let release_dir = source_path
        .parent()
        .and_then(Path::parent)
        .expect("project root")
        .join("target")
        .join("release");
    assert!(release_dir.join(shared_library_name("refund_bot")).exists());
    assert!(release_dir.join("lib_refund_bot.h").exists());
    assert!(release_dir.join("refund_bot.corvid-abi.json").exists());
}
