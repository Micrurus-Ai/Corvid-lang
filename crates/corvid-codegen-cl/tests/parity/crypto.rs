//! `Bytes`, SHA-2, HMAC and Ed25519 verification on the native tier.
//! Both tiers compute through `corvid-crypto`, so they must return the
//! same text, and the known answers below (FIPS 180-4, RFC 4231, RFC
//! 8032 and two recorded payment-provider webhooks) pin what that text
//! is. `==` on byte strings is the runtime's constant-time comparison
//! on both tiers, inside records, lists, options and results too.
//! Native code has no `Secret`; the last test pins that refusal.

use super::ir_of;
use super::scalar_harness::assert_parity_text;

#[test]
fn sha2_digests_match_the_fips_examples() {
    let out = assert_parity_text(
        r#"
agent main() -> String:
    abc = "abc".utf8()
    empty = "".utf8()
    return abc.sha256().to_hex() + "|" + empty.sha256().to_hex() + "|" + abc.sha384().to_hex() + "|" + abc.sha512().to_hex() + "|" + [abc.length(), "café".utf8().length(), abc.sha384().length(), empty.length()].to_json()
"#,
    );
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(parts[1], "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(
        parts[2],
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(
        parts[3],
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
    assert_eq!(parts[4], "[3,5,48,0]");
}

#[test]
fn encodings_round_trip_and_decoders_report_the_text_codec_messages() {
    let out = assert_parity_text(
        r#"
agent shown(r: Result<Bytes, String>) -> String:
    return match r:
        Ok(b) -> "ok:" + b.to_hex() + "/" + b.to_base64() + "/" + b.to_base64_url()
        Err(e) -> "err:" + e

agent utf8_text(r: Result<String, String>) -> String:
    return match r:
        Ok(t) -> "ok:" + t
        Err(e) -> "err:" + e

agent text_of(r: Result<Bytes, String>) -> String:
    return match r:
        Ok(b) -> utf8_text(b.to_utf8())
        Err(e) -> "decode:" + e

agent main() -> String:
    return shown(Bytes.from_hex("DEADbeef")) + "|" + shown(Bytes.from_base64("3q2+7w==")) + "|" + shown(Bytes.from_base64_url("3q2-7w==")) + "|" + shown(Bytes.from_hex("abc")) + "|" + shown(Bytes.from_base64_url("a+b=")) + "|" + shown(Bytes.from_base64("3q2-7w")) + "|" + shown(Bytes.from_hex("")) + "|" + text_of(Bytes.from_hex("636166c3a9")) + "|" + text_of(Bytes.from_hex("61ff")) + "|" + "café".utf8().to_base64_url()
"#,
    );
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "ok:deadbeef/3q2+7w==/3q2-7w");
    assert_eq!(parts[1], parts[0]);
    assert_eq!(parts[2], parts[0], "padded base64url reads back to the same bytes");
    assert_eq!(parts[3], "err:not hex: 3 digits cannot encode whole bytes; hex needs two digits per byte");
    assert_eq!(parts[4], "err:not URL-safe base64: unexpected character `+` at position 1");
    assert!(parts[5].starts_with("err:not base64: unexpected character `-`"), "{out}");
    assert_eq!(parts[6], "ok://");
    assert_eq!(parts[7], "ok:café");
    assert_eq!(parts[8], "err:the bytes are not UTF-8 text (byte 1 starts an invalid sequence)");
    assert_eq!(parts[9], "Y2Fmw6k");
}

#[test]
fn byte_strings_compare_by_their_bytes_inside_records_lists_options_and_results() {
    let out = assert_parity_text(
        r#"
type Signed:
    body: String
    tag: Bytes

agent main() -> String:
    a = "x".utf8().sha256()
    b = "x".utf8().sha256()
    c = "y".utf8().sha256()
    records = Signed("x", a) == Signed("x", b)
    records_differ = Signed("x", a) == Signed("x", c)
    lists = [a, c] == [b, c]
    lists_differ = [a] == [c]
    some: Option<Bytes> = Some(a)
    options = some == Some(b)
    results = Bytes.from_hex("00ff") == Bytes.from_base64("AP8=")
    results_differ = Bytes.from_hex("00ff") == Bytes.from_base64("AP4=")
    lengths = "ab".utf8() == "abc".utf8()
    return [records, records_differ, lists, lists_differ, options, results, results_differ, a == b, a != c, a == c, lengths].to_json()
"#,
    );
    assert_eq!(out, "[true,false,true,false,true,true,false,true,true,false,false]");
}

#[test]
fn hmac_matches_rfc_4231() {
    // RFC 4231 test case 6 keys with 131 bytes of 0xaa.
    let long_key = "aa".repeat(131);
    let out = assert_parity_text(&format!(
        r#"
agent hex(text: String) -> Bytes:
    return match Bytes.from_hex(text):
        Ok(bytes) -> bytes
        Err(problem) -> "not hex".utf8()

agent main() -> String:
    case_1 = "Hi There".utf8()
    key_1 = hex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b")
    case_2 = "what do ya want for nothing?".utf8()
    key_2 = "Jefe".utf8()
    key_6 = hex("{long_key}")
    case_6 = "Test Using Larger Than Block-Size Key - Hash Key First".utf8()
    return case_1.hmac_sha256(key_1).to_hex() + "|" + case_2.hmac_sha256(key_2).to_hex() + "|" + case_6.hmac_sha256(key_6).to_hex() + "|" + case_1.hmac_sha512(key_1).to_hex() + "|" + case_2.hmac_sha512(key_2).to_hex() + "|" + case_6.hmac_sha512(key_6).to_hex() + "|" + [key_6.length()].to_json()
"#
    ));
    let parts: Vec<&str> = out.split('|').collect();
    assert_eq!(parts[0], "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
    assert_eq!(parts[1], "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    assert_eq!(parts[2], "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    assert_eq!(
        parts[3],
        "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
    );
    assert_eq!(
        parts[4],
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
    );
    assert_eq!(
        parts[5],
        "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f3526b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598"
    );
    assert_eq!(parts[6], "[131]");
}

/// The recorded requests of `crates/corvid-driver/tests/webhook_verification.rs`,
/// verified with the signing key held as `Bytes`: native code has no
/// `Secret`, so the driver's programs, which read the key with
/// `secret(name)`, run on the interpreter. The key bytes and the
/// signatures are the same, so the verdicts must be too. The header
/// is read as the driver's programs read it: rebinding the timestamp
/// inside a loop over the header's fields, and returning from inside
/// the loop that finds a matching signature.
#[test]
fn the_recorded_stripe_and_standard_webhooks_verify_and_tampered_ones_are_rejected() {
    let out = assert_parity_text(
        r#"
agent verify_stripe(key: Bytes, signature_header: String, body: String) -> Bool:
    timestamp = ""
    for part in signature_header.split(","):
        if part.starts_with("t="):
            timestamp = part.substring(2, part.length())
    if timestamp == "":
        return false
    expected = (timestamp + "." + body).utf8().hmac_sha256(key)
    for part in signature_header.split(","):
        if part.starts_with("v1="):
            matched = match Bytes.from_hex(part.substring(3, part.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return true
    return false

agent verify_standard_webhook(key: Bytes, message_id: String, timestamp: String, signature_header: String, body: String) -> Bool:
    expected = (message_id + "." + timestamp + "." + body).utf8().hmac_sha256(key)
    for entry in signature_header.split(" "):
        if entry.starts_with("v1,"):
            matched = match Bytes.from_base64(entry.substring(3, entry.length())):
                Ok(received) -> received == expected
                Err(problem) -> false
            if matched:
                return true
    return false

agent standard_key(written: String) -> Bytes:
    return match Bytes.from_base64(written.substring(6, written.length())):
        Ok(key) -> key
        Err(problem) -> "".utf8()

agent main() -> String:
    body = "{\"id\":\"evt_1QZ\",\"type\":\"payment_intent.succeeded\",\"data\":{\"object\":{\"id\":\"pi_3Q\",\"amount\":2000,\"currency\":\"usd\"}}}"
    tampered = "{\"id\":\"evt_1QZ\",\"type\":\"payment_intent.succeeded\",\"data\":{\"object\":{\"id\":\"pi_3Q\",\"amount\":9000,\"currency\":\"usd\"}}}"
    stripe_key = "whsec_corvid_test_4eC39HqLyjWDarjtT1zdp7dc".utf8()
    other_key = "whsec_corvid_test_someone_elses_key".utf8()
    signature = "9d4aa1ff5638dd3708a9c29539870fc8fc470059f7feeb7ff559ab641d7f6dce"
    header = "t=1732480000,v1=" + signature
    stripe = [
        verify_stripe(stripe_key, header, body),
        verify_stripe(stripe_key, header, tampered),
        verify_stripe(stripe_key, "t=1732480001,v1=" + signature, body),
        verify_stripe(stripe_key, "t=1732480000,v1=9d4aa1ff5638dd3708a9c29539870fc8fc470059f7feeb7ff559ab641d7f6dcf", body),
        verify_stripe(stripe_key, "t=1732480000", body),
        verify_stripe(stripe_key, "t=1732480000,v1=0000000000000000000000000000000000000000000000000000000000000000,v1=" + signature, body),
        verify_stripe(other_key, header, body),
    ]
    key = standard_key("whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYn")
    wrong = standard_key("whsec_EBESExQVFhcYGRobHB0eHyAhIiMkJSYo")
    id = "msg_2KWPBgLlAfxdpx2AI54pPJ85f4W"
    sent = "v1,+dJkrlylFBxSVAn/YI6mFfwStrqxrlMnbfCAVbmRja4="
    standard = [
        verify_standard_webhook(key, id, "1732480000", sent, body),
        verify_standard_webhook(key, id, "1732480000", sent, tampered),
        verify_standard_webhook(key, id, "1732480001", sent, body),
        verify_standard_webhook(key, "msg_2KWPBgLlAfxdpx2AI54pPJ85f4X", "1732480000", sent, body),
        verify_standard_webhook(key, id, "1732480000", "v1,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= " + sent, body),
        verify_standard_webhook(wrong, id, "1732480000", sent, body),
    ]
    return stripe.to_json() + " " + standard.to_json() + " " + [key.length()].to_json()
"#,
    );
    assert_eq!(out, "[true,false,false,false,false,true,false] [true,false,false,false,true,false] [24]");
}

#[test]
fn ed25519_verifies_the_rfc_8032_vectors_and_rejects_forgeries() {
    let out = assert_parity_text(
        r#"
agent hex(text: String) -> Bytes:
    return match Bytes.from_hex(text):
        Ok(bytes) -> bytes
        Err(problem) -> "not hex".utf8()

agent main() -> String:
    key_1 = hex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
    signature_1 = hex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
    key_2 = hex("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
    signature_2 = hex("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00")
    key_3 = hex("fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025")
    signature_3 = hex("6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a")
    small_order_key = hex("0100000000000000000000000000000000000000000000000000000000000000")
    trivial = hex("01000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000")
    genuine = [
        "".utf8().verify_ed25519(signature_1, key_1),
        hex("72").verify_ed25519(signature_2, key_2),
        hex("af82").verify_ed25519(signature_3, key_3),
    ]
    forged = [
        hex("73").verify_ed25519(signature_2, key_2),
        hex("72").verify_ed25519(signature_1, key_2),
        hex("72").verify_ed25519(signature_2, key_1),
        "".utf8().verify_ed25519(signature_1.sha256(), key_1),
        "".utf8().verify_ed25519(signature_1, key_1.sha512()),
        "any message".utf8().verify_ed25519(trivial, small_order_key),
    ]
    return genuine.to_json() + " " + forged.to_json()
"#,
    );
    assert_eq!(out, "[true,true,true] [false,false,false,false,false,false]");
}

#[test]
fn byte_strings_cross_json_as_unpadded_base64url() {
    let out = assert_parity_text(
        r#"
type Envelope:
    name: String
    payload: Bytes

agent equal(a: Bytes, b: Bytes) -> String:
    return [a == b].to_json()

agent main() -> String:
    payload = match Bytes.from_hex("deadbeef"):
        Ok(bytes) -> bytes
        Err(problem) -> "".utf8()
    text = Envelope("a", payload).to_json()
    back = match Envelope.from_json("{\"name\": \"b\", \"payload\": \"3q2-7w==\"}"):
        Ok(envelope) -> envelope.payload.to_hex() + " " + equal(envelope.payload, payload)
        Err(problem) -> problem.path + ": " + problem.found
    refused = match Envelope.from_json("{\"name\": \"c\", \"payload\": \"3q2+7w\"}"):
        Ok(envelope) -> "decoded"
        Err(problem) -> problem.path + " " + problem.expected + ": " + problem.found
    return text + " | " + back + " | " + refused + " | " + [payload, payload.sha256()].to_json()
"#,
    );
    let parts: Vec<&str> = out.split(" | ").collect();
    assert_eq!(parts[0], r#"{"name":"a","payload":"3q2-7w"}"#);
    assert_eq!(parts[1], "deadbeef [true]");
    assert!(parts[2].starts_with("payload Bytes: "), "{out}");
    assert_eq!(parts[3], r#"["3q2-7w","X3jDMnTkP6neVlkmXB2RfiXANyLcsLjSfbjV_qqBOVM"]"#);
}

#[test]
fn an_entry_agent_returning_bytes_prints_them_as_the_interpreter_does() {
    let out = assert_parity_text(
        r#"
agent main() -> Bytes:
    return "abc".utf8()
"#,
    );
    assert_eq!(out, "Bytes(616263)");
}

/// Native code holds a `Secret` as the runtime's object (native dispatch
/// N3), so a program that compares secrets, keys an HMAC with one or
/// checks a password builds. The driver's `native_secrets.rs` runs such
/// programs on both tiers and compares the answers.
#[test]
fn a_native_build_compares_secrets_and_keys_cryptography_with_them() {
    for src in [
        "agent same(a: Secret, b: Secret) -> Bool:\n    return a == b\n\nagent main() -> Int:\n    return 1\n",
        "agent mac(key: Secret) -> Bytes:\n    return \"body\".utf8().hmac_sha256(key)\n\nagent main() -> Int:\n    return 1\n",
        "agent check(password: Secret) -> Bool:\n    return password.verify_password(\"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaA\")\n\nagent main() -> Int:\n    return 1\n",
    ] {
        let ir = ir_of(src);
        let tmp = tempfile::tempdir().expect("tempdir");
        corvid_codegen_cl::build_native_to_disk(&ir, "corvid_parity_test", &tmp.path().join("prog"), &[])
            .unwrap_or_else(|error| panic!("{error}; src=\n{src}"));
    }
}
