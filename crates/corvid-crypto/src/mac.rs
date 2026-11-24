//! HMAC message authentication (RFC 2104) over SHA-256 and SHA-512.
//!
//! A payment provider signs each webhook with an HMAC of its body under
//! a shared key; the receiver computes the same HMAC and compares the
//! two with `==` on `Bytes`, which is constant-time. The key is either
//! `Bytes` or a `Secret`. A secret keys the HMAC with its bytes without
//! them leaving this crate, so the key never becomes a value the
//! program could print.

use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha512};

use crate::{Bytes, Secret};

/// The key of `bytes.hmac_sha256(key)` and `bytes.hmac_sha512(key)`.
#[derive(Clone, Copy)]
pub enum HmacKey<'a> {
    /// A key the program holds as bytes.
    Bytes(&'a Bytes),
    /// A secret: its text's UTF-8 bytes, or the key decoded by
    /// `decode_base64_after`.
    Secret(&'a Secret),
}

impl HmacKey<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            HmacKey::Bytes(bytes) => bytes.as_slice(),
            HmacKey::Secret(secret) => secret.key_bytes(),
        }
    }
}

/// `message.hmac_sha256(key)`: the 32-byte HMAC-SHA256 tag.
pub fn hmac_sha256(message: &Bytes, key: HmacKey<'_>) -> Bytes {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.bytes()).expect("HMAC takes a key of any length");
    mac.update(message.as_slice());
    Bytes::new(&mac.finalize().into_bytes())
}

/// `message.hmac_sha512(key)`: the 64-byte HMAC-SHA512 tag.
pub fn hmac_sha512(message: &Bytes, key: HmacKey<'_>) -> Bytes {
    let mut mac = Hmac::<Sha512>::new_from_slice(key.bytes()).expect("HMAC takes a key of any length");
    mac.update(message.as_slice());
    Bytes::new(&mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers from RFC 4231 section 4: test case 1 (a 20-byte
    // key), test case 2 (a key shorter than the output) and test case 6
    // (a 131-byte key, longer than the hash's block, which is hashed
    // first).

    fn case_1() -> (Bytes, Bytes) {
        (Bytes::from_utf8_text("Hi There"), Bytes::new(&[0x0b; 20]))
    }

    fn case_2() -> (Bytes, Bytes) {
        (Bytes::from_utf8_text("what do ya want for nothing?"), Bytes::from_utf8_text("Jefe"))
    }

    fn case_6() -> (Bytes, Bytes) {
        (Bytes::from_utf8_text("Test Using Larger Than Block-Size Key - Hash Key First"), Bytes::new(&[0xaa; 131]))
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        let expected = [
            (case_1(), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"),
            (case_2(), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"),
            (case_6(), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"),
        ];
        for ((message, key), tag) in expected {
            assert_eq!(hmac_sha256(&message, HmacKey::Bytes(&key)).to_hex(), tag);
        }
    }

    #[test]
    fn hmac_sha512_matches_rfc_4231() {
        let expected = [
            (
                case_1(),
                "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cde\
                 daa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
            ),
            (
                case_2(),
                "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea250554\
                 9758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
            ),
            (
                case_6(),
                "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f352\
                 6b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598",
            ),
        ];
        for ((message, key), tag) in expected {
            assert_eq!(hmac_sha512(&message, HmacKey::Bytes(&key)).to_hex(), tag);
        }
    }

    #[test]
    fn a_secret_keys_the_hmac_with_its_bytes() {
        let (message, _) = case_2();
        let text_key = Secret::new("Jefe");
        assert_eq!(
            hmac_sha256(&message, HmacKey::Secret(&text_key)).to_hex(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // "SmVmZQ==" is the base64 of "Jefe".
        let decoded_key = Secret::new("whsec_SmVmZQ==").decode_base64_after("whsec_").unwrap();
        assert_eq!(hmac_sha256(&message, HmacKey::Secret(&decoded_key)), hmac_sha256(&message, HmacKey::Secret(&text_key)));
    }

    #[test]
    fn tags_have_their_fixed_lengths_and_depend_on_the_key() {
        let message = Bytes::from_utf8_text("order.paid");
        let a = Bytes::from_utf8_text("key-a");
        let b = Bytes::from_utf8_text("key-b");
        assert_eq!(hmac_sha256(&message, HmacKey::Bytes(&a)).len(), 32);
        assert_eq!(hmac_sha512(&message, HmacKey::Bytes(&a)).len(), 64);
        assert_ne!(hmac_sha256(&message, HmacKey::Bytes(&a)), hmac_sha256(&message, HmacKey::Bytes(&b)));
        assert_ne!(
            hmac_sha256(&message, HmacKey::Bytes(&a)),
            hmac_sha256(&Bytes::from_utf8_text("order.paif"), HmacKey::Bytes(&a))
        );
    }
}
