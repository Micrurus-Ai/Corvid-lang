//! Ed25519 signature verification (RFC 8032), strict.
//!
//! A release, a signed token or a webhook from a provider that signs
//! with a public key carries an Ed25519 signature: 64 bytes made with a
//! private key the receiver never sees, checked against the signer's
//! 32-byte public key. Verification is pure, so
//! `message.verify_ed25519(signature, public_key)` is a builtin method
//! allowed in `@deterministic` code and never traced.
//!
//! Verification is strict ([`ed25519_dalek::VerifyingKey::verify_strict`]).
//! RFC 8032 lets verifiers differ on edge cases, and the permissive
//! reading accepts signatures nobody made: with a public key of small
//! order (the identity point among them) a signature of all-but-one
//! zero bytes verifies for every message. Strict verification refuses
//! a public key or a signature point `R` of small order, and, like
//! every verification here, an `S` that is not reduced below the group
//! order, so a valid signature cannot be altered into a second valid
//! one. Signatures made by any conforming signer are unaffected.
//!
//! The answer is a `Bool`. A signature that is not 64 bytes, a public
//! key that is not 32 bytes or not a point on the curve verifies
//! nothing, so the answer is `false`, the same as for a forged
//! signature: a program checking a signature branches on one question.

use ed25519_dalek::{Signature, VerifyingKey};

use crate::Bytes;

/// The length of an Ed25519 signature: the point `R` and the scalar `S`.
pub const ED25519_SIGNATURE_BYTES: usize = 64;

/// The length of an Ed25519 public key: one compressed point.
pub const ED25519_PUBLIC_KEY_BYTES: usize = 32;

/// `message.verify_ed25519(signature, public_key)`: whether `signature`
/// is a valid Ed25519 signature of `message` under `public_key`, with
/// strict verification. Wrong lengths and a public key that is not a
/// curve point are `false`.
pub fn verify_ed25519(message: &Bytes, signature: &Bytes, public_key: &Bytes) -> bool {
    let Ok(signature) = <[u8; ED25519_SIGNATURE_BYTES]>::try_from(signature.as_slice()) else {
        return false;
    };
    let Ok(public_key) = <[u8; ED25519_PUBLIC_KEY_BYTES]>::try_from(public_key.as_slice()) else {
        return false;
    };
    let Ok(public_key) = VerifyingKey::from_bytes(&public_key) else {
        return false;
    };
    verify_ed25519_signature(&public_key, message.as_slice(), &Signature::from_bytes(&signature))
}

/// Whether `signature` is a valid Ed25519 signature of `message` under
/// `key`: strict verification, and a weak (small-order) key is never
/// accepted. Every Ed25519 check in Corvid goes through here: the
/// language's `verify_ed25519`, release and bundle attestations, signed
/// traces, dimension artifacts and the package registry. Permissive
/// verification accepts the identity-key forgery, where one signature
/// is valid for every message under a small-order key.
pub fn verify_ed25519_signature(key: &VerifyingKey, message: &[u8], signature: &Signature) -> bool {
    !key.is_weak() && key.verify_strict(message, signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

    fn hex(text: &str) -> Bytes {
        Bytes::from_hex(text).expect("test hex")
    }

    /// A test vector from RFC 8032 section 7.1, as (message, signature,
    /// public key).
    struct Vector {
        message: Bytes,
        signature: Bytes,
        public_key: Bytes,
    }

    fn vector(message: &str, signature: &str, public_key: &str) -> Vector {
        Vector { message: hex(message), signature: hex(signature), public_key: hex(public_key) }
    }

    fn test_1() -> Vector {
        vector(
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        )
    }

    fn test_2() -> Vector {
        vector(
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        )
    }

    fn test_3() -> Vector {
        vector(
            "af82",
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
            "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
        )
    }

    /// TEST 1024: a 1023-byte message.
    fn test_1024() -> Vector {
        let message = [
        "08b8b2b733424243760fe426a4b54908632110a66c2f6591eabd3345e3e4eb98",
        "fa6e264bf09efe12ee50f8f54e9f77b1e355f6c50544e23fb1433ddf73be84d8",
        "79de7c0046dc4996d9e773f4bc9efe5738829adb26c81b37c93a1b270b20329d",
        "658675fc6ea534e0810a4432826bf58c941efb65d57a338bbd2e26640f89ffbc",
        "1a858efcb8550ee3a5e1998bd177e93a7363c344fe6b199ee5d02e82d522c4fe",
        "ba15452f80288a821a579116ec6dad2b3b310da903401aa62100ab5d1a36553e",
        "06203b33890cc9b832f79ef80560ccb9a39ce767967ed628c6ad573cb116dbef",
        "efd75499da96bd68a8a97b928a8bbc103b6621fcde2beca1231d206be6cd9ec7",
        "aff6f6c94fcd7204ed3455c68c83f4a41da4af2b74ef5c53f1d8ac70bdcb7ed1",
        "85ce81bd84359d44254d95629e9855a94a7c1958d1f8ada5d0532ed8a5aa3fb2",
        "d17ba70eb6248e594e1a2297acbbb39d502f1a8c6eb6f1ce22b3de1a1f40cc24",
        "554119a831a9aad6079cad88425de6bde1a9187ebb6092cf67bf2b13fd65f270",
        "88d78b7e883c8759d2c4f5c65adb7553878ad575f9fad878e80a0c9ba63bcbcc",
        "2732e69485bbc9c90bfbd62481d9089beccf80cfe2df16a2cf65bd92dd597b07",
        "07e0917af48bbb75fed413d238f5555a7a569d80c3414a8d0859dc65a46128ba",
        "b27af87a71314f318c782b23ebfe808b82b0ce26401d2e22f04d83d1255dc51a",
        "ddd3b75a2b1ae0784504df543af8969be3ea7082ff7fc9888c144da2af58429e",
        "c96031dbcad3dad9af0dcbaaaf268cb8fcffead94f3c7ca495e056a9b47acdb7",
        "51fb73e666c6c655ade8297297d07ad1ba5e43f1bca32301651339e22904cc8c",
        "42f58c30c04aafdb038dda0847dd988dcda6f3bfd15c4b4c4525004aa06eeff8",
        "ca61783aacec57fb3d1f92b0fe2fd1a85f6724517b65e614ad6808d6f6ee34df",
        "f7310fdc82aebfd904b01e1dc54b2927094b2db68d6f903b68401adebf5a7e08",
        "d78ff4ef5d63653a65040cf9bfd4aca7984a74d37145986780fc0b16ac451649",
        "de6188a7dbdf191f64b5fc5e2ab47b57f7f7276cd419c17a3ca8e1b939ae49e4",
        "88acba6b965610b5480109c8b17b80e1b7b750dfc7598d5d5011fd2dcc5600a3",
        "2ef5b52a1ecc820e308aa342721aac0943bf6686b64b2579376504ccc493d97e",
        "6aed3fb0f9cd71a43dd497f01f17c0e2cb3797aa2a2f256656168e6c496afc5f",
        "b93246f6b1116398a346f1a641f3b041e989f7914f90cc2c7fff357876e506b5",
        "0d334ba77c225bc307ba537152f3f1610e4eafe595f6d9d90d11faa933a15ef1",
        "369546868a7f3a45a96768d40fd9d03412c091c6315cf4fde7cb68606937380d",
        "b2eaaa707b4c4185c32eddcdd306705e4dc1ffc872eeee475a64dfac86aba41c",
        "0618983f8741c5ef68d3a101e8a3b8cac60c905c15fc910840b94c00a0b9d0",
        ]
        .concat();
        vector(
            &message,
            "0aab4c900501b3e24d7cdf4663326a3a87df5e4843b2cbdb67cbf6e460fec350aa5371b1508f9f4528ecea23c436d94b5e8fcd4f681e30a6ac00a9704a188a03",
            "278117fc144c72340f67d0f2316e8386ceffbf2b2428c9c51fef7c597f1d426e",
        )
    }

    /// TEST SHA(abc): the message is the SHA-512 digest of "abc".
    fn test_sha_abc() -> Vector {
        vector(
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            "dc2a4459e7369633a52b1bf277839a00201009a3efbf3ecb69bea2186c26b58909351fc9ac90b3ecfdfbc7c66431e0303dca179c138ac17ad9bef1177331a704",
            "ec172b93ad5e563bf4932c70e1245034c35467ef2efd4d64ebf819683467e2bf",
        )
    }

    fn all_vectors() -> [Vector; 5] {
        [test_1(), test_2(), test_3(), test_1024(), test_sha_abc()]
    }

    fn with_byte_flipped(bytes: &Bytes, index: usize) -> Bytes {
        let mut changed = bytes.as_slice().to_vec();
        changed[index] ^= 0x01;
        Bytes::new(&changed)
    }

    #[test]
    fn the_rfc_8032_vectors_verify() {
        let test_1024 = test_1024();
        assert_eq!(test_1024.message.len(), 1023);
        assert_eq!(crate::sha512(&Bytes::from_utf8_text("abc")), test_sha_abc().message);
        for (index, v) in all_vectors().iter().enumerate() {
            assert!(verify_ed25519(&v.message, &v.signature, &v.public_key), "vector {index} must verify");
        }
    }

    #[test]
    fn a_changed_message_signature_or_key_is_rejected() {
        for (index, v) in all_vectors().iter().enumerate() {
            if !v.message.is_empty() {
                let last = v.message.len() - 1;
                assert!(!verify_ed25519(&with_byte_flipped(&v.message, last), &v.signature, &v.public_key), "{index}");
            }
            for byte in [0, 31, 32, 63] {
                assert!(!verify_ed25519(&v.message, &with_byte_flipped(&v.signature, byte), &v.public_key), "{index}/{byte}");
            }
            // Another vector's key, and a single changed bit of the right key
            // (which may not even be a curve point).
            let other = &all_vectors()[(index + 1) % 5].public_key;
            assert!(!verify_ed25519(&v.message, &v.signature, other), "{index}");
            assert!(!verify_ed25519(&v.message, &v.signature, &with_byte_flipped(&v.public_key, 0)), "{index}");
        }
        // An empty message is still a message: TEST 1's signature does not
        // sign a one-byte message.
        let v = test_1();
        assert!(!verify_ed25519(&Bytes::new(&[0]), &v.signature, &v.public_key));
    }

    #[test]
    fn wrong_lengths_are_false() {
        let v = test_2();
        let signature = v.signature.as_slice();
        let key = v.public_key.as_slice();
        for short_or_long in [&signature[..63], &[signature, &[0]].concat()[..], &[]] {
            assert!(!verify_ed25519(&v.message, &Bytes::new(short_or_long), &v.public_key));
        }
        for short_or_long in [&key[..31], &[key, &[0]].concat()[..], &[]] {
            assert!(!verify_ed25519(&v.message, &v.signature, &Bytes::new(short_or_long)));
        }
        // A 64-byte key is the usual mistake: a private key with its public
        // half appended, as RFC 8032's sources print it.
        let secret_and_public = hex(
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        );
        assert!(!verify_ed25519(&v.message, &v.signature, &secret_and_public));
    }

    #[test]
    fn a_small_order_key_is_refused_though_permissive_verification_accepts_it() {
        // The identity point as the public key and as R, with S = 0:
        // [S]B = R + [k]A holds for every message, because both sides are
        // the identity.
        let identity = {
            let mut point = [0u8; 32];
            point[0] = 1;
            point
        };
        let signature = [&identity[..], &[0u8; 32]].concat();
        let message = Bytes::from_utf8_text("pay 1000000 to mallory");
        let permissive = VerifyingKey::from_bytes(&identity)
            .expect("the identity is a curve point")
            .verify(message.as_slice(), &Signature::from_slice(&signature).unwrap());
        assert!(permissive.is_ok(), "the permissive check accepts the forgery, which is why it is not used");
        assert!(!verify_ed25519(&message, &Bytes::new(&signature), &Bytes::new(&identity)));
    }

    #[test]
    fn a_signature_altered_to_a_second_form_is_refused() {
        // S + L, where L is the group order, is the same scalar unreduced.
        // A verifier that accepted it would let anyone turn one valid
        // signature into another.
        const ORDER: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
        ];
        for v in all_vectors() {
            let mut altered = v.signature.as_slice().to_vec();
            let mut carry = 0u16;
            for (byte, order) in altered[32..].iter_mut().zip(ORDER) {
                let sum = u16::from(*byte) + u16::from(order) + carry;
                *byte = sum as u8;
                carry = sum >> 8;
            }
            assert_eq!(carry, 0, "S + L fits in 32 bytes");
            assert!(!verify_ed25519(&v.message, &Bytes::new(&altered), &v.public_key));
        }
    }
}
