//! `Secret` in native code (native dispatch N3). A native secret is the
//! runtime's opaque object holding the interpreter's own
//! `corvid_crypto::Secret`. A program reads one with `secret(name)` or
//! draws one with `random_token(bytes)` through the runtime call, holds
//! it in records, lists and options, and compares it in constant time.
//! Both tiers give the same answers, the trace never holds the text, and
//! the uses native code cannot yet keep redacted stay on the interpreter.

use std::path::{Path, PathBuf};
use std::process::Command;

use corvid_driver::{build_or_get_cached_native, compile_to_ir_with_config_at_path, native_ability, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

const KEY_ENV: &str = "CORVID_TEST_NATIVE_SECRET_KEY";
const OTHER_ENV: &str = "CORVID_TEST_NATIVE_SECRET_OTHER";
const KEY_TEXT: &str = "whsec_native_key_0123";
const OTHER_TEXT: &str = "whsec_native_other_456";

const PROGRAM: &str = r#"
import "./std/secrets" use secret
import "./std/crypto" use random_token

type Credentials:
    user: String
    key: Secret

agent fallback() -> Secret:
    return random_token(16)

agent must(name: String) -> Secret:
    return match secret(name):
        Ok(key) -> key
        Err(problem) -> fallback()

agent problem(name: String) -> String:
    return match secret(name):
        Ok(key) -> "found"
        Err(message) -> message

agent same_holder(a: Credentials, b: Credentials) -> Bool:
    return a == b

agent main() -> String:
    key = must("CORVID_TEST_NATIVE_SECRET_KEY")
    again = must("CORVID_TEST_NATIVE_SECRET_KEY")
    other = must("CORVID_TEST_NATIVE_SECRET_OTHER")
    first = random_token(32)
    second = random_token(32)
    held = Credentials("ann", key)
    answers = [key == again, key != other, key == other, first == second, first == first, same_holder(held, Credentials("ann", again)), same_holder(held, Credentials("ann", other)), [key, other] == [again, other], Some(key) == Some(other)]
    return answers.to_json() + " " + problem("CORVID_TEST_NATIVE_SECRET_MISSING")
"#;

const EXPECTED_ANSWERS: &str = "[true,true,false,false,true,true,false,true,false]";

struct Project {
    _dir: tempfile::TempDir,
    source: PathBuf,
    ir: corvid_ir::IrFile,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn project(program: &str) -> Project {
    std::env::set_var(KEY_ENV, KEY_TEXT);
    std::env::set_var(OTHER_ENV, OTHER_TEXT);
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    copy_dir(&corvid_test_paths::workspace_root().join("std"), &src.join("std"));
    let source = src.join("main.cor");
    std::fs::write(&source, program).unwrap();
    let ir = compile_to_ir_with_config_at_path(program, &source, None).expect("program compiles");
    Project { _dir: dir, source, ir }
}

/// Links against the release `corvid-test-tools` staticlib, which bundles
/// the current `corvid-runtime`, as the codegen native tests do.
fn link_current_runtime() {
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "corvid-test-tools", "--release"])
            .current_dir(corvid_test_paths::workspace_root())
            .status()
            .expect("build corvid-test-tools");
        assert!(status.success(), "building corvid-test-tools failed");
        std::env::set_var("CORVID_RUNTIME_STATICLIB_OVERRIDE", corvid_test_paths::test_tools_staticlib());
    });
}

fn native_binary(project: &Project, program: &str) -> PathBuf {
    native_ability(&project.ir).expect("the program compiles to native code");
    link_current_runtime();
    build_or_get_cached_native(&project.source, program, &project.ir, None).expect("native build").path
}

fn run_native(binary: &Path, env: &[(&str, &Path)]) -> String {
    let mut command = Command::new(binary);
    command
        .env(KEY_ENV, KEY_TEXT)
        .env(OTHER_ENV, OTHER_TEXT)
        .env_remove("CORVID_TEST_NATIVE_SECRET_MISSING")
        .env_remove("CORVID_TRACE_PATH")
        .env_remove("CORVID_REPLAY_TRACE_PATH");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run native binary");
    assert!(
        output.status.success(),
        "native run failed: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secrets_compare_alike_on_both_tiers_and_never_reach_the_trace() {
    let project = project(PROGRAM);
    let traces = tempfile::tempdir().unwrap();

    let runtime = Runtime::builder().trace_to(traces.path()).build();
    let interpreted = run_ir_with_runtime(&project.ir, None, vec![], &runtime).await.expect("interpreter run");
    let Value::String(interpreted) = interpreted else { panic!("expected a String, got {interpreted:?}") };
    assert!(interpreted.starts_with(EXPECTED_ANSWERS), "{interpreted}");

    let binary = native_binary(&project, PROGRAM);
    let native_trace = traces.path().join("native.jsonl");
    let native = run_native(&binary, &[("CORVID_TRACE_PATH", &native_trace)]);
    assert_eq!(native, *interpreted, "both tiers give the same answers and the same missing-secret message");

    let raw = std::fs::read_to_string(&native_trace).unwrap();
    for text in [KEY_TEXT, OTHER_TEXT] {
        assert!(!raw.contains(text), "the native trace never holds a secret's text");
    }
    let redacted_results = raw.lines().filter(|line| line.contains("\"tool_result\"") && line.contains("<redacted>")).count();
    assert!(redacted_results >= 5, "every secret and token result is recorded as `<redacted>`:\n{raw}");

    let replayed = run_native(&binary, &[("CORVID_REPLAY_TRACE_PATH", &native_trace)]);
    assert_eq!(replayed, native, "native replay re-reads the secrets and redraws the tokens, with the same answers");

    // Every secret object is freed exactly once: the allocator's counts
    // balance at exit.
    let counted = Command::new(&binary)
        .env(KEY_ENV, KEY_TEXT)
        .env(OTHER_ENV, OTHER_TEXT)
        .env("CORVID_DEBUG_ALLOC", "1")
        .env_remove("CORVID_TRACE_PATH")
        .env_remove("CORVID_REPLAY_TRACE_PATH")
        .output()
        .expect("run native binary");
    let stderr = String::from_utf8_lossy(&counted.stderr);
    let count = |name: &str| -> i64 {
        stderr
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("no {name} in:\n{stderr}"))
            .trim()
            .parse()
            .unwrap()
    };
    assert_eq!(count("ALLOCS"), count("RELEASES"), "{stderr}");
}

fn refusal(program: &str) -> String {
    let project = project(program);
    native_ability(&project.ir).expect_err("the program stays on the interpreter").to_string()
}

/// A tool of the program's own records its arguments in native code
/// without redaction, so one that takes a secret, alone or in a record
/// field, keeps the program on the interpreter.
#[test]
fn a_tool_of_the_programs_own_that_takes_a_secret_stays_on_the_interpreter() {
    for program in [
        "import \"./std/crypto\" use random_token\n\ntool sign(key: Secret, body: String) -> String\n\nagent main() -> String:\n    return sign(random_token(16), \"body\")\n",
        "import \"./std/crypto\" use random_token\n\ntype Signer:\n    key: Secret\n\ntool sign(signer: Signer, body: String) -> String\n\nagent main() -> String:\n    return sign(Signer(random_token(16)), \"body\")\n",
    ] {
        let reason = refusal(program);
        assert!(reason.contains("tool of the program's own"), "{reason}");
    }
}
