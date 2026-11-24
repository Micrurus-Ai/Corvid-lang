//! SHA-2 digests (FIPS 180-4).

use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::Bytes;

/// `bytes.sha256()`: the 32-byte SHA-256 digest.
pub fn sha256(bytes: &Bytes) -> Bytes {
    Bytes::new(&Sha256::digest(bytes.as_slice()))
}

/// `bytes.sha384()`: the 48-byte SHA-384 digest.
pub fn sha384(bytes: &Bytes) -> Bytes {
    Bytes::new(&Sha384::digest(bytes.as_slice()))
}

/// `bytes.sha512()`: the 64-byte SHA-512 digest.
pub fn sha512(bytes: &Bytes) -> Bytes {
    Bytes::new(&Sha512::digest(bytes.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers from FIPS 180-4's examples (NIST CSRC "Examples
    // with intermediate values") for "abc" and the empty message.

    #[test]
    fn sha256_matches_the_known_answers() {
        assert_eq!(
            sha256(&Bytes::from_utf8_text("abc")).to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(&Bytes::new(&[])).to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha384_matches_the_known_answers() {
        assert_eq!(
            sha384(&Bytes::from_utf8_text("abc")).to_hex(),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed\
             8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            sha384(&Bytes::new(&[])).to_hex(),
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da\
             274edebfe76f65fbd51ad2f14898b95b"
        );
    }

    #[test]
    fn sha512_matches_the_known_answers() {
        assert_eq!(
            sha512(&Bytes::from_utf8_text("abc")).to_hex(),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
             2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            sha512(&Bytes::new(&[])).to_hex(),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
             47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
    }

    #[test]
    fn digests_have_their_fixed_lengths() {
        let message = Bytes::from_utf8_text("any message");
        assert_eq!(sha256(&message).len(), 32);
        assert_eq!(sha384(&message).len(), 48);
        assert_eq!(sha512(&message).len(), 64);
    }
}
