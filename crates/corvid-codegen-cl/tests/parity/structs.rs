use super::{assert_parity, assert_parity_bool, ir_of, test_tools_lib_path};

/// Phase 20n-C commit 5 enabled struct returns at the native
/// command-line boundary by emitting a per-struct JSON encoder
/// in the codegen-emitted main (see `crate::lowering::struct_encode`
/// + `Type::Struct(_)` arm of `build_native_to_disk` in
/// `crates/corvid-codegen-cl/src/lib.rs`). The previous shape of
/// this test asserted the pre-feature behaviour — that the build
/// would refuse with a `NotSupported` error mentioning "struct"
/// + "serialization" — and was never updated when the feature
/// shipped. This is the positive coverage that should have
/// replaced it: build the native binary, run it, and verify the
/// emitted JSON encodes the struct returned from the entry agent.
#[test]
fn struct_entry_return_builds_and_prints_json_encoded_struct() {
    use corvid_codegen_cl::build_native_to_disk;

    let ir = ir_of(
        "type Wrap:\n    v: Int\n\nagent f() -> Wrap:\n    return Wrap(42)\n",
    );
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("prog");
    let produced = build_native_to_disk(
        &ir,
        "corvid_parity_test",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .expect("struct entry-return build should succeed (Phase 20n-C feature)");
    assert!(
        produced.is_file(),
        "emitted native binary missing at `{}`",
        produced.display()
    );
    let output = std::process::Command::new(&produced)
        .output()
        .expect("run native binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "native binary exited non-zero: status={:?} stdout=`{stdout}` stderr=`{}`",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    // The per-struct JSON encoder emits `{"v":42}` for
    // `Wrap(42)`. Use a substring check to remain resilient to
    // whitespace / ordering choices the encoder may make in
    // future versions.
    assert!(
        stdout.contains("\"v\":42") || stdout.contains("\"v\": 42"),
        "expected stdout to contain the JSON-encoded struct value `\"v\":42`; got `{stdout}`"
    );
}

/// `Type::List(_)` returns are still blocked at the native
/// command-line boundary — they need their own dedicated encoder
/// primitives (per the comment in `build_native_to_disk` at
/// `crates/corvid-codegen-cl/src/lib.rs:131`). Preserve the
/// shape of the previous struct-blocked test as a regression
/// guard against the same wrong-spec drift happening on the
/// list-return arm.
#[test]
fn list_entry_return_is_blocked_with_clear_error() {
    use corvid_codegen_cl::{build_native_to_disk, CodegenErrorKind};

    let ir = ir_of("agent f() -> List<Int>:\n    return [1, 2, 3]\n");
    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("prog");
    let err = build_native_to_disk(
        &ir,
        "corvid_parity_test",
        &bin_path,
        &[test_tools_lib_path().as_path()],
    )
    .unwrap_err();
    match err.kind {
        CodegenErrorKind::NotSupported(ref msg) => {
            assert!(
                msg.contains("List") || msg.contains("list"),
                "expected message to mention list: {msg}"
            );
            assert!(
                msg.contains("encoder") || msg.contains("encoder primitives"),
                "expected message to point at missing encoder primitives: {msg}"
            );
        }
        other => panic!("expected NotSupported, got {other:?}"),
    }
}

#[test]
fn scalar_only_struct_construct_and_access() {
    assert_parity(
        "\
type Point:
    x: Int
    y: Int

agent main() -> Int:
    p = Point(3, 4)
    return p.x + p.y
",
        7,
    );
}

#[test]
fn struct_with_bool_field() {
    assert_parity_bool(
        "\
type Flag:
    enabled: Bool
    code: Int

agent main() -> Bool:
    f = Flag(true, 42)
    return f.enabled
",
        true,
    );
}

#[test]
fn struct_with_string_field_destructor_releases_field() {
    assert_parity_bool(
        "\
type Order:
    id: String
    amount: Float

agent main() -> Bool:
    o = Order(\"ord_1\", 49.99)
    return o.amount > 10.0
",
        true,
    );
}

#[test]
fn struct_with_string_field_extract_and_compare() {
    assert_parity_bool(
        "\
type Named:
    label: String

agent main() -> Bool:
    n = Named(\"hello\")
    return n.label == \"hello\"
",
        true,
    );
}

#[test]
fn struct_passed_to_another_agent() {
    assert_parity(
        "\
type Amount:
    cents: Int

agent total(a: Amount, b: Amount) -> Int:
    return a.cents + b.cents

agent main() -> Int:
    x = Amount(100)
    y = Amount(250)
    return total(x, y)
",
        350,
    );
}

#[test]
fn struct_reassignment_releases_old_instance() {
    assert_parity(
        "\
type Box:
    v: Int

agent main() -> Int:
    b = Box(1)
    b = Box(100)
    return b.v
",
        100,
    );
}

#[test]
fn nested_struct_field_access() {
    assert_parity(
        "\
type Inner:
    value: Int

type Outer:
    inner: Inner
    tag: Int

agent main() -> Int:
    i = Inner(7)
    o = Outer(i, 10)
    return o.inner.value + o.tag
",
        17,
    );
}
