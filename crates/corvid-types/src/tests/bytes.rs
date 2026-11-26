//! `Bytes` in the checker: the methods and type methods, the operator
//! rules (constant-time `==`, no ordering, no `+`, no mixing with
//! text), purity, and the boundaries a byte string may cross.

use super::check;
use crate::errors::TypeErrorKind;

fn assert_clean(src: &str) {
    let checked = check(src);
    assert!(checked.errors.is_empty(), "got {:?}", checked.errors);
}

/// The hint of the one `BytesOperatorMismatch` error `src` produces.
fn operator_hint(src: &str) -> String {
    let errors = check(src).errors;
    let error = errors
        .iter()
        .find(|e| matches!(e.kind, TypeErrorKind::BytesOperatorMismatch { .. }))
        .unwrap_or_else(|| panic!("got {errors:?}"));
    error.kind.hint().expect("a bytes operator error has a hint")
}

#[test]
fn methods_and_type_methods_typecheck() {
    assert_clean(
        "agent main(body: String, received: String) -> Bool:
    digest: Bytes = body.utf8().sha256()
    longer: Bytes = digest.sha384().sha512()
    hex: String = digest.to_hex()
    base: String = digest.to_base64() + digest.to_base64_url()
    size: Int = longer.length()
    text: Result<String, String> = digest.to_utf8()
    from_hex: Result<Bytes, String> = Bytes.from_hex(received)
    from_base: Result<Bytes, String> = Bytes.from_base64(received)
    from_url: Result<Bytes, String> = Bytes.from_base64_url(received)
    return from_hex == Ok(digest) and size == 64
",
    );
}

#[test]
fn equality_between_byte_strings_is_allowed() {
    assert_clean(
        "agent main(a: Bytes, b: Bytes) -> Bool:
    return a == b or a != b
",
    );
}

#[test]
fn byte_strings_have_no_order() {
    for op in ["<", "<=", ">", ">="] {
        let hint = operator_hint(&format!("agent main(a: Bytes, b: Bytes) -> Bool:\n    return a {op} b\n"));
        assert!(hint.contains("no order") && hint.contains("constant-time"), "{op}: {hint}");
    }
}

#[test]
fn byte_strings_do_not_add() {
    let hint = operator_hint("agent main(a: Bytes, b: Bytes) -> Bytes:\n    return a + b\n");
    assert!(hint.contains("only `==` and `!=`"), "{hint}");
}

#[test]
fn bytes_and_text_do_not_mix() {
    let hint = operator_hint("agent main(a: Bytes, b: String) -> Bool:\n    return a == b\n");
    assert!(hint.contains("utf8()") && hint.contains("to_hex()"), "{hint}");
    let hint = operator_hint("agent main(a: Bytes) -> String:\n    return \"digest: \" + a\n");
    assert!(hint.contains("bytes and text do not mix"), "{hint}");
}

#[test]
fn the_type_methods_check_their_argument() {
    let errors = check("agent main() -> Result<Bytes, String>:\n    return Bytes.from_hex(42)\n").errors;
    assert!(
        errors.iter().any(|e| matches!(&e.kind, TypeErrorKind::TypeMismatch { expected, .. } if expected == "String")),
        "got {errors:?}"
    );
    let errors = check("agent main() -> Result<Bytes, String>:\n    return Bytes.from_hex()\n").errors;
    assert!(errors.iter().any(|e| matches!(e.kind, TypeErrorKind::ArityMismatch { .. })), "got {errors:?}");
}

#[test]
fn hashing_is_allowed_in_deterministic_agents() {
    assert_clean(
        "@deterministic
agent fingerprint(body: String) -> String:
    return body.utf8().sha256().to_hex()
",
    );
}

#[test]
fn bytes_cross_json_tools_prompts_and_routes() {
    assert_clean(
        "type Signed:
    body: String
    signature: Bytes

tool deliver(message: Signed) -> Bytes

prompt summarize(digest: Bytes) -> String:
    \"Summarize {digest}\"

agent sign(body: String) -> Signed:
    return Signed(body, body.utf8().sha256())

server Webhooks:
    route GET \"/latest\" -> json Signed:
        return sign(\"hello\")

agent main(text: String) -> String:
    back = Signed.from_json(text)
    return sign(\"hello\").to_json()
",
    );
}

/// B09 P8c: `Bytes` is a binary table column, so a table may hold one.
#[test]
fn a_table_column_of_bytes_is_accepted() {
    let errors = check(
        "database files:
    engine: sqlite
    location: \"files.sqlite\"

table Upload:
    key id: String
    digest: Bytes
",
    )
    .errors;
    assert!(errors.is_empty(), "got {errors:?}");
}
