//! Phase 33R5b-b — end-to-end tests that prove a real Corvid
//! program (compiled through the driver pipeline, run through
//! the interpreter) reaches the executing JSON dispatch in both
//! shapes the umbrella ships:
//!
//!   1. **Opaque path** — `json_parse(text) -> Result<JsonValue,
//!      String>`, typed accessors (`json_get_int`,
//!      `json_get_string`), builder side (`json_object_new` +
//!      `json_object_set_*` + `json_object_finish`).
//!
//!   2. **Typed decoding** — `User.from_json(text)` decodes into a
//!      declared record through the public JSON mapping. These tests
//!      also pin the refusal of the retired `decode_<Type>_from_json`
//!      tool convention it replaced; the method surface itself is
//!      covered in `typed_json_e2e.rs`.
//!
//! Result handling uses the `?` operator, which propagates `Err` to
//! the enclosing agent's `Result<...>` return type. The test
//! assertions then check `Value::ResultOk` / `Value::ResultErr`.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::fs;
use std::path::Path;

/// Stage a fresh project at `dir` whose `src/std/` is a vendored
/// copy of the workspace stdlib (incl. the new `json.cor`) and
/// whose `corvid.toml` carries a minimal `[io] root`. Returns the
/// path to `src/main.cor`.
fn stage_project(dir: &Path, main_source: &str) -> std::path::PathBuf {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root resolves to two parents up from this crate's manifest");
    let src = dir.join("src");
    let std_dir = src.join("std");
    fs::create_dir_all(&std_dir).unwrap();
    for file in ["effects.cor", "io.cor", "http.cor", "db.cor", "json.cor"] {
        fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    fs::write(dir.join("corvid.toml"), "[io]\nroot = \".\"\n").unwrap();
    let main_path = src.join("main.cor");
    fs::write(&main_path, main_source).unwrap();
    main_path
}

/// 33R5b-b load-bearing acceptance — opaque shape. Real Corvid
/// program parses JSON, accesses an Int field via typed getter,
/// returns it through `Result<Int, String>`. Proves every layer
/// of the dispatch path works against a real `.cor` source.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_corvid_program_round_trips_data_through_opaque_json_dispatch() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
import "./std/json" use json_parse, json_get_int

agent main() -> Result<Int, String>:
    parsed = json_parse("{\"id\": 42}")?
    n = json_get_int(parsed, "id")?
    return Ok(n)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("real corvid program should compile through the driver");
    let runtime = Runtime::builder().build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("opaque-shape JSON round trip should run end-to-end");

    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::Int(42) => {}
            other => panic!("expected ResultOk(Int(42)); got Ok({other:?})"),
        },
        other => panic!("expected main to return ResultOk; got {other:?}"),
    }
}

/// Typed decoding: `User.from_json(text)` decodes into the declared
/// record, and `map_err` + `?` carry a failure into an agent that
/// returns `Result<_, String>` — the one-line replacement for the
/// retired `decode_<Type>_from_json` tool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_corvid_program_decodes_typed_struct_via_from_json() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
type User:
    id: Int
    email: String

agent main() -> Result<Int, String>:
    user = User.from_json("{\"id\": 7, \"email\": \"alice@example.com\"}").map_err(fn (e) -> e.message)?
    return Ok(user.id)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("typed-decoding corvid program should compile");
    let runtime = Runtime::builder().build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("typed decoding should run end-to-end");

    match result {
        Value::ResultOk(inner) => match inner.get() {
            Value::Int(7) => {}
            other => panic!("expected ResultOk(Int(7)) from user.id; got Ok({other:?})"),
        },
        other => panic!("expected main to return ResultOk; got {other:?}"),
    }
}

/// A tool declared for the retired `decode_<Type>_from_json`
/// convention is refused at compile time, naming the replacement,
/// rather than compiling into a tool with no implementation.
#[test]
fn retired_decoder_tool_declaration_is_refused_with_the_replacement() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
effect json_decode_eff:
    reversible: true

type User:
    id: Int
    email: String

tool decode_user_from_json(text: String) -> Result<User, String> uses json_decode_eff

agent main() -> Result<Int, String>:
    user = decode_user_from_json("{\"id\": 7, \"email\": \"a\"}")?
    return Ok(user.id)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let diagnostics = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect_err("a retired decoder tool must not compile");
    let rendered = format!("{diagnostics:?}");
    assert!(
        rendered.contains("retired `decode_<Type>_from_json` convention"),
        "the refusal should name the retired convention; got {rendered}"
    );
    assert!(
        rendered.contains("User.from_json(text)"),
        "the refusal should name the replacement call; got {rendered}"
    );
}

/// The refusal matches only the shape the runtime used to decode: a
/// tool with that name but another return type was never intercepted
/// and still compiles as an ordinary tool.
#[test]
fn a_decoder_shaped_name_with_another_return_type_still_compiles() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
effect json_decode_eff:
    reversible: true

tool decode_score_from_json(text: String) -> Int uses json_decode_eff

agent main() -> Int:
    return decode_score_from_json("1")
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("a tool outside the retired shape is an ordinary tool");
}

/// 33R5b-b — **the load-bearing parse-safety property
/// end-to-end through a real Corvid program**. A program calls
/// `json_parse` on malformed text, the `try` operator propagates
/// the Err to the agent's return. The test asserts the agent
/// returned `Value::ResultErr`, proving the recoverable-error
/// path works at the language level — user code can route parse
/// failures up to its caller without crashes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_returns_result_err_through_real_corvid_program() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
import "./std/json" use json_parse, json_get_int

agent main() -> Result<Int, String>:
    parsed = json_parse("{not valid json at all")?
    n = json_get_int(parsed, "id")?
    return Ok(n)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("parse-failure corvid program should compile");
    let runtime = Runtime::builder().build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("json_parse on malformed input must not error the runtime — it returns Err");

    match result {
        Value::ResultErr(inner) => {
            // The Err payload should be a String naming the failure.
            match inner.get() {
                Value::String(s) => {
                    let msg = s.to_string();
                    assert!(
                        msg.contains("malformed JSON"),
                        "Err message should name the parse failure; got: {msg}"
                    );
                }
                other => panic!("expected Err to wrap a String; got Err({other:?})"),
            }
        }
        Value::ResultOk(_) => panic!(
            "json_parse on malformed input returned Ok — the parse-safety property is broken"
        ),
        other => panic!("expected main to return ResultErr; got {other:?}"),
    }
}

/// Companion: a JSON shape mismatch in typed decoding is a
/// `Result::Err`, not a panic. `User` expects `id: Int` but the
/// input has a String there; the `JsonError` names the path and the
/// agent propagates its message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typed_decoding_shape_mismatch_returns_result_err_through_real_corvid_program() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
type User:
    id: Int
    email: String

agent main() -> Result<Int, String>:
    user = User.from_json("{\"id\": \"not an int\", \"email\": \"alice\"}").map_err(fn (e) -> e.path + ": " + e.message)?
    return Ok(user.id)
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("shape-mismatch corvid program should compile");
    let runtime = Runtime::builder().build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("a shape mismatch must not error the runtime — it returns Err");

    match result {
        Value::ResultErr(inner) => match inner.get() {
            Value::String(s) => {
                let msg = s.to_string();
                assert!(
                    msg.starts_with("id: "),
                    "Err message should name the mismatched field's path; got: {msg}"
                );
            }
            other => panic!("expected Err to wrap a String; got Err({other:?})"),
        },
        Value::ResultOk(_) => panic!(
            "typed decoding returned Ok despite a shape mismatch — the field-type-safety property is broken"
        ),
        other => panic!("expected main to return ResultErr; got {other:?}"),
    }
}

/// 33R5b-b — pins the snapshot-not-consumer semantics of
/// `json_object_finish` through a real Corvid program. The
/// program sets a field, finishes (snapshot A), sets a different
/// value for the same field, finishes again (snapshot B). The
/// two snapshots must differ — proving the builder isn't
/// invalidated by finish and that subsequent set_* calls take
/// effect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn json_builder_finish_is_a_snapshot_through_real_corvid_program() {
    let project = tempfile::tempdir().expect("tempdir");
    let main_path = stage_project(
        project.path(),
        r#"
import "./std/json" use json_object_new, json_object_set_int, json_object_finish

agent main() -> Bool:
    builder = json_object_new()
    builder = json_object_set_int(builder, "version", 1)
    snapshot_a = json_object_finish(builder)
    builder = json_object_set_int(builder, "version", 2)
    snapshot_b = json_object_finish(builder)
    return snapshot_a != snapshot_b
"#,
    );

    let source = fs::read_to_string(&main_path).unwrap();
    let ir = compile_to_ir_with_config_at_path(&source, &main_path, None)
        .expect("snapshot-semantics corvid program should compile");
    let runtime = Runtime::builder().build();

    let result = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("builder snapshot round trip should run end-to-end");

    match result {
        Value::Bool(true) => {}
        Value::Bool(false) => panic!(
            "the two snapshots were equal — `json_object_finish` may have invalidated the \
             builder, or subsequent `json_object_set_int` calls had no effect. The snapshot-\
             not-consumer semantics is broken."
        ),
        other => panic!("expected main to return Bool(true); got {other:?}"),
    }
}
