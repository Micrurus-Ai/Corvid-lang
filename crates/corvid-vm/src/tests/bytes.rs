//! `Bytes` and hashing on the interpreter, run from Corvid source. The
//! rules themselves are tested in `corvid-crypto` and `corvid-text`;
//! these tests pin the method surface, the Result values the decoders
//! return, constant-time equality wherever values compare, and the
//! base64url JSON form.

use super::*;

async fn run_main(src: &str) -> Value {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run")
}

fn text(value: &str) -> Value {
    Value::String(Arc::from(value))
}

fn err(value: &str) -> Value {
    Value::ResultErr(crate::value::BoxedValue::new(text(value)))
}

#[tokio::test]
async fn sha256_of_abc_is_the_fips_180_4_digest() {
    let src = "\
agent main() -> String:
    return \"abc\".utf8().sha256().to_hex()
";
    assert_eq!(run_main(src).await, text("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
}

#[tokio::test]
async fn sha384_and_sha512_of_the_empty_string_are_the_fips_180_4_digests() {
    let src = "\
agent main() -> List<String>:
    empty = \"\".utf8()
    return [empty.sha384().to_hex(), empty.sha512().to_hex()]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(
        items.iter_cloned(),
        vec![
            text("38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"),
            text("cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"),
        ]
    );
}

#[tokio::test]
async fn encodings_and_lengths() {
    let src = "\
agent main() -> List<String>:
    digest = \"abc\".utf8().sha256()
    return [digest.to_base64(), digest.to_base64_url(), digest.length().to_string(), \"café\".utf8().length().to_string()]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(
        items.iter_cloned(),
        vec![
            text("ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="),
            text("ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"),
            text("32"),
            text("5"),
        ]
    );
}

#[tokio::test]
async fn every_encoding_round_trips() {
    let src = "\
agent main() -> Bool:
    digest = \"abc\".utf8().sha512()
    hex = Bytes.from_hex(digest.to_hex()) == Ok(digest)
    upper = Bytes.from_hex(digest.to_hex().to_upper()) == Ok(digest)
    base = Bytes.from_base64(digest.to_base64()) == Ok(digest)
    url_safe = Bytes.from_base64_url(digest.to_base64_url()) == Ok(digest)
    text = \"naïve\".utf8().to_utf8() == Ok(\"naïve\")
    return hex and upper and base and url_safe and text
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}

#[tokio::test]
async fn malformed_input_is_an_err_value_not_a_trap() {
    for (call, expected) in [
        ("Bytes.from_hex(\"abc\")", err("not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte")),
        ("Bytes.from_hex(\"4g\")", err("not hex: unexpected character `g` at position 1")),
        ("Bytes.from_base64(\"Zm9v!A==\")", err("not base64: unexpected character `!` at position 4")),
        (
            "Bytes.from_base64_url(\"ab+c\")",
            err("not URL-safe base64: unexpected character `+` at position 2"),
        ),
    ] {
        let src = format!("agent main() -> Result<Bytes, String>:\n    return {call}\n");
        assert_eq!(run_main(&src).await, expected, "{call}");
    }
    let src = "\
agent main() -> Result<String, String>:
    return match Bytes.from_hex(\"61ff\"):
        Ok(b) -> b.to_utf8()
        Err(e) -> Err(e)
";
    assert_eq!(run_main(src).await, err("the bytes are not UTF-8 text (byte 1 starts an invalid sequence)"));
}

#[tokio::test]
async fn equality_compares_contents_and_lengths() {
    let src = "\
agent main() -> List<Bool>:
    a = \"abc\".utf8()
    same = \"abc\".utf8()
    other = \"abd\".utf8()
    shorter = \"ab\".utf8()
    return [a == same, a != same, a == other, a == shorter, shorter == a, a.sha256() == same.sha256()]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    let bools: Vec<Value> = [true, false, false, false, false, true].into_iter().map(Value::Bool).collect();
    assert_eq!(items.iter_cloned(), bools);
}

#[tokio::test]
async fn equality_inside_records_and_lists_compares_bytes() {
    let src = "\
type Signed:
    body: String
    signature: Bytes

agent main() -> List<Bool>:
    a = Signed(\"hi\", \"hi\".utf8().sha256())
    b = Signed(\"hi\", \"hi\".utf8().sha256())
    c = Signed(\"hi\", \"ho\".utf8().sha256())
    digests = [a.signature, c.signature]
    return [a == b, a == c, [a.signature] == [b.signature], digests.contains(b.signature), [c.signature].contains(a.signature)]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    let bools: Vec<Value> = [true, false, true, true, false].into_iter().map(Value::Bool).collect();
    assert_eq!(items.iter_cloned(), bools);
}

#[test]
fn ordering_is_refused_before_the_program_runs() {
    let src = "agent main(a: Bytes, b: Bytes) -> Bool:\n    return a < b\n";
    let tokens = lex(src).expect("lex");
    let (file, _) = parse_file(&tokens);
    let resolved = resolve(&file);
    let checked = typecheck(&file, &resolved);
    assert!(
        checked.errors.iter().any(|e| matches!(e.kind, corvid_types::TypeErrorKind::BytesOperatorMismatch { .. })),
        "got {:?}",
        checked.errors
    );
}

#[tokio::test]
async fn json_carries_bytes_as_unpadded_base64url() {
    let src = "\
type Signed:
    body: String
    signature: Bytes

agent main() -> String:
    signed = Signed(\"hello\", \"hello\".utf8().sha256())
    text = signed.to_json()
    back = match Signed.from_json(text):
        Ok(s) -> s == signed
        Err(e) -> false
    return text + \" \" + back.to_string()
";
    assert_eq!(
        run_main(src).await,
        text(r#"{"body":"hello","signature":"LPJNul-wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ"} true"#)
    );
}

#[tokio::test]
async fn json_decoding_accepts_padding_and_names_a_bad_field() {
    let src = "\
type Blob:
    data: Bytes

agent main() -> List<String>:
    padded = match Blob.from_json(\"{\\\"data\\\": \\\"3q2-7_8=\\\"}\"):
        Ok(b) -> b.data.to_hex()
        Err(e) -> e.message
    bad = match Blob.from_json(\"{\\\"data\\\": \\\"3q2+7w==\\\"}\"):
        Ok(b) -> b.data.to_hex()
        Err(e) -> e.path + \" \" + e.expected + \": \" + e.found
    return [padded, bad]
";
    let Value::List(items) = run_main(src).await else { panic!("expected a list") };
    assert_eq!(
        items.iter_cloned(),
        vec![text("deadbeefff"), text("data Bytes: `3q2+7w==`: not URL-safe base64: unexpected character `+` at position 3")]
    );
}

#[tokio::test]
async fn a_tool_receives_and_returns_bytes_as_base64url_text() {
    let src = "\
tool store_digest(digest: Bytes) -> Bytes

agent main() -> Bool:
    digest = \"hello\".utf8().sha256()
    return store_digest(digest) == digest.sha256()
";
    let ir = ir_of(src);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_by_tool = seen.clone();
    let rt = Runtime::builder()
        .approver(Arc::new(ProgrammaticApprover::always_yes()))
        .tool("store_digest", move |args| {
            let seen = seen_by_tool.clone();
            async move {
                seen.lock().unwrap().push(args.clone());
                // The SHA-256 of the received digest, as the tool's own
                // base64url text.
                Ok(json!("lZXJ35AHUUjrBoYDZd8zWEt1v_eCpRDGzUiDpBmDPVA"))
            }
        })
        .build();
    let result = run_agent(&ir, "main", vec![], &rt).await.expect("run");
    assert_eq!(seen.lock().unwrap().as_slice(), [vec![json!("LPJNul-wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ")]]);
    assert_eq!(result, Value::Bool(true));
}

#[tokio::test]
async fn hashing_is_allowed_in_deterministic_agents() {
    let src = "\
@deterministic
agent fingerprint(body: String) -> String:
    return body.utf8().sha256().to_hex()

agent main() -> String:
    return fingerprint(\"abc\")
";
    assert_eq!(run_main(src).await, text("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
}

#[tokio::test]
async fn the_reference_examples_run() {
    // docs/reference/cryptography.md
    let src = "\
agent content_hash(body: String) -> String:
    return body.utf8().sha256().to_hex()

agent digest_matches(body: String, received_hex: String) -> Bool:
    return match Bytes.from_hex(received_hex):
        Ok(received) -> body.utf8().sha256() == received
        Err(problem) -> false

agent main() -> Bool:
    digest = content_hash(\"abc\")
    known = digest == \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"
    matches = digest_matches(\"abc\", digest.to_upper())
    tampered = digest_matches(\"abd\", digest)
    malformed = digest_matches(\"abc\", \"not hex\")
    return known and matches and not tampered and not malformed
";
    assert_eq!(run_main(src).await, Value::Bool(true));
}
