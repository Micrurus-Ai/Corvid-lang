//! Password hashing with Argon2id (RFC 9106), at a declared cost.
//!
//! A password, or any secret the program only needs to check later such
//! as an API key, is stored as its Argon2id hash in the PHC string
//! format: `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`. The string
//! records the algorithm, its version, the cost and the salt, so
//! verification reads everything it needs from the stored hash, and a
//! hash made at an older cost still verifies after the cost is raised.
//!
//! The cost is always declared by the caller ([`PasswordCost`]): Argon2id's
//! memory, iterations and lanes trade login latency against the cost of
//! guessing, and the project rule forbids choosing that silently.
//! [`PasswordCost::OWASP_2023`] is the OWASP Password Storage Cheat
//! Sheet's minimum, and [`PasswordCost::new`] refuses any cost weaker
//! than the settings OWASP lists as equivalent to it.
//!
//! Both the language (`hash_password` in std/crypto and
//! `secret.verify_password(hash)`) and the runtime's API-key store hash
//! and verify through this file, so they cannot disagree.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

/// The bytes of salt drawn for each hash: 128 bits, the size RFC 9106
/// recommends.
const SALT_BYTES: usize = 16;

/// The most memory a cost may declare: 4 GiB, in KiB. A login that
/// needs more would exhaust a server's memory with a handful of
/// concurrent attempts. Verification refuses a stored hash that claims
/// more, so a corrupted or planted hash cannot do the same.
pub const MAX_MEMORY_KIB: u32 = 4 * 1024 * 1024;

/// The Argon2id settings OWASP's Password Storage Cheat Sheet (2023)
/// lists as equally strong, as (memory in KiB, iterations), all with
/// one lane. A cost is at least the minimum when it has at least the
/// memory and at least the iterations of one of them.
const OWASP_2023_EQUIVALENT_SETTINGS: [(u32, u32); 5] =
    [(47_104, 1), (19_456, 2), (12_288, 3), (9_216, 4), (7_168, 5)];

/// The cost of one Argon2id hash: memory in KiB, iterations (passes
/// over the memory) and lanes (degree of parallelism).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordCost {
    memory_kib: u32,
    iterations: u32,
    lanes: u32,
}

impl PasswordCost {
    /// `owasp_2023()`: 19 MiB (19456 KiB), 2 iterations, 1 lane. The
    /// OWASP 2023 minimum for Argon2id, and the setting the `argon2`
    /// crate's `Argon2::default()` uses.
    pub const OWASP_2023: PasswordCost = PasswordCost { memory_kib: 19_456, iterations: 2, lanes: 1 };

    /// A declared cost, refused when it is weaker than the OWASP 2023
    /// minimum, above [`MAX_MEMORY_KIB`], or not a cost Argon2id can
    /// run. A cost may be higher than the minimum in any dimension.
    pub fn new(memory_kib: i64, iterations: i64, lanes: i64) -> Result<PasswordCost, String> {
        if lanes < 1 {
            return Err(format!("a password cost needs at least 1 lane, not {lanes}"));
        }
        if iterations < 1 {
            return Err(format!("a password cost needs at least 1 iteration, not {iterations}"));
        }
        if memory_kib > i64::from(MAX_MEMORY_KIB) {
            return Err(format!(
                "a password cost of {memory_kib} KiB is above the limit of {MAX_MEMORY_KIB} KiB (4 GiB), which would exhaust a server's memory under a few concurrent logins"
            ));
        }
        let meets_minimum = OWASP_2023_EQUIVALENT_SETTINGS
            .iter()
            .any(|&(memory, passes)| memory_kib >= i64::from(memory) && iterations >= i64::from(passes));
        if !meets_minimum {
            return Err(format!(
                "a password cost of {memory_kib} KiB and {iterations} iteration{} is below the OWASP 2023 minimum for Argon2id; use owasp_2023() (19456 KiB, 2 iterations, 1 lane) or a setting OWASP lists as equal: 47104 KiB with 1 iteration, 12288 KiB with 3, 9216 KiB with 4 or 7168 KiB with 5",
                if iterations == 1 { "" } else { "s" }
            ));
        }
        let narrow = |value: i64, what: &str| {
            u32::try_from(value).map_err(|_| format!("a password cost of {value} {what} is too large"))
        };
        let cost = PasswordCost {
            memory_kib: narrow(memory_kib, "KiB")?,
            iterations: narrow(iterations, "iterations")?,
            lanes: narrow(lanes, "lanes")?,
        };
        cost.params()?;
        Ok(cost)
    }

    pub fn memory_kib(&self) -> u32 {
        self.memory_kib
    }

    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    pub fn lanes(&self) -> u32 {
        self.lanes
    }

    fn params(&self) -> Result<Params, String> {
        Params::new(self.memory_kib, self.iterations, self.lanes, None)
            .map_err(|problem| format!("Argon2id cannot run this password cost: {problem}"))
    }
}

/// `hash_password(password, cost)`: the Argon2id hash of `password` at
/// `cost` under a fresh random salt, as a PHC string. An error only when
/// the operating system's entropy source fails.
pub fn hash_password(password: &[u8], cost: PasswordCost) -> Result<String, String> {
    let mut salt = [0u8; SALT_BYTES];
    crate::fill_random(&mut salt)?;
    hash_password_with_salt(password, cost.params()?, &salt)
}

/// The Argon2id hash of `password` with the given parameters and salt.
/// Not public: a caller never chooses a salt, and the parameters reach
/// here only through a validated [`PasswordCost`] (and, in this file's
/// tests, the reference implementation's test parameters).
fn hash_password_with_salt(password: &[u8], params: Params, salt: &[u8]) -> Result<String, String> {
    let salt = SaltString::encode_b64(salt).map_err(|problem| format!("the salt cannot be encoded: {problem}"))?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password, &salt)
        .map(|hash| hash.to_string())
        .map_err(|problem| format!("Argon2id failed: {problem}"))
}

/// Whether `password` is the one `hash` was made from. `hash` is a PHC
/// string, whose parameters and salt are used, so a hash made at any
/// cost verifies. The comparison of the computed hash with the stored
/// one is constant-time (the `password-hash` crate's `Output` equality).
///
/// `Err` when `hash` is not an Argon2id PHC string this function can
/// verify: malformed, another algorithm (argon2i, argon2d, bcrypt), an
/// Argon2 version other than 0x13, or a memory cost above
/// [`MAX_MEMORY_KIB`]. The message never quotes the hash.
pub fn verify_password(password: &[u8], hash: &str) -> Result<bool, String> {
    let parsed = PasswordHash::new(hash).map_err(|problem| format!("the hash is not a PHC string: {problem}"))?;
    if parsed.algorithm != argon2::ARGON2ID_IDENT {
        return Err(format!("the hash is `{}`, not `argon2id`", parsed.algorithm));
    }
    if parsed.version != Some(Version::V0x13.into()) {
        return Err("the hash is not Argon2 version 19 (0x13)".to_string());
    }
    let params = Params::try_from(&parsed).map_err(|problem| format!("the hash's parameters are invalid: {problem}"))?;
    if params.m_cost() > MAX_MEMORY_KIB {
        return Err(format!(
            "the hash declares {} KiB of memory, above the limit of {MAX_MEMORY_KIB} KiB",
            params.m_cost()
        ));
    }
    if parsed.salt.is_none() || parsed.hash.is_none() {
        return Err("the hash has no salt or no hash value".to_string());
    }
    // The verifier hashes with the parameters, version and salt read
    // from `parsed`, not its own.
    match Argon2::default().verify_password(password, &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(problem) => Err(format!("the hash cannot be verified: {problem}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Known answers computed outside this crate with OpenSSL 3.6's
    // Argon2id (`openssl kdf -keylen 32 -kdfopt pass:... -kdfopt
    // salt:... -kdfopt iter:T -kdfopt memcost:M -kdfopt lanes:P
    // ARGON2ID`). The first is also a test vector of the reference
    // implementation's test suite (phc-winner-argon2 `src/test.c`,
    // version 0x13, t=2, m=2^8, p=1).

    fn tag_of(phc: &str) -> String {
        let parsed = PasswordHash::new(phc).unwrap();
        corvid_hex::encode(parsed.hash.unwrap().as_bytes())
    }

    #[test]
    fn matches_the_reference_implementation() {
        let params = Params::new(256, 2, 1, None).unwrap();
        let phc = hash_password_with_salt(b"password", params, b"somesalt").unwrap();
        assert_eq!(tag_of(&phc), "9dfeb910e80bad0311fee20f9c0e2b12c17987b4cac90c2ef54d5b3021c68bfe");
        assert!(phc.starts_with("$argon2id$v=19$m=256,t=2,p=1$c29tZXNhbHQ$"), "{phc}");
    }

    #[test]
    fn hashes_at_the_owasp_2023_cost_match_an_independent_implementation() {
        let salt: Vec<u8> = (0u8..16).collect();
        let phc = hash_password_with_salt(b"correct horse battery staple", PasswordCost::OWASP_2023.params().unwrap(), &salt)
            .unwrap();
        assert_eq!(
            phc,
            "$argon2id$v=19$m=19456,t=2,p=1$AAECAwQFBgcICQoLDA0ODw$gYJZtjEAJqjg26xdLmknq8/bB7MiWPrE9hsYuA+SkIU"
        );
        assert_eq!(verify_password(b"correct horse battery staple", &phc), Ok(true));
    }

    #[test]
    fn a_hash_round_trips_and_rejects_another_password() {
        let phc = hash_password(b"hunter2 but longer", PasswordCost::OWASP_2023).unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        assert_eq!(verify_password(b"hunter2 but longer", &phc), Ok(true));
        assert_eq!(verify_password(b"hunter2 but longeR", &phc), Ok(false));
        assert_eq!(verify_password(b"", &phc), Ok(false));
    }

    #[test]
    fn two_hashes_of_one_password_differ_by_their_salt() {
        let first = hash_password(b"same password", PasswordCost::OWASP_2023).unwrap();
        let second = hash_password(b"same password", PasswordCost::OWASP_2023).unwrap();
        assert_ne!(first, second);
        assert_eq!(verify_password(b"same password", &second), Ok(true));
    }

    #[test]
    fn a_hash_records_its_cost_so_a_higher_cost_still_verifies() {
        let cost = PasswordCost::new(47_104, 1, 1).unwrap();
        let phc = hash_password(b"long-lived", cost).unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=47104,t=1,p=1$"), "{phc}");
        assert_eq!(verify_password(b"long-lived", &phc), Ok(true));
    }

    #[test]
    fn a_cost_below_the_owasp_minimum_is_refused() {
        assert_eq!(PasswordCost::new(19_456, 2, 1), Ok(PasswordCost::OWASP_2023));
        for (memory, iterations) in [(47_104, 1), (12_288, 3), (9_216, 4), (7_168, 5), (65_536, 3)] {
            assert!(PasswordCost::new(memory, iterations, 1).is_ok(), "{memory} KiB, t={iterations}");
        }
        let problem = PasswordCost::new(19_455, 2, 1).unwrap_err();
        assert!(problem.starts_with("a password cost of 19455 KiB and 2 iterations is below the OWASP 2023 minimum"), "{problem}");
        assert!(problem.contains("owasp_2023()"), "{problem}");
        for (memory, iterations) in [(19_456, 1), (7_168, 4), (4_096, 100), (47_103, 1)] {
            assert!(PasswordCost::new(memory, iterations, 1).is_err(), "{memory} KiB, t={iterations}");
        }
        assert_eq!(
            PasswordCost::new(19_456, 2, 0),
            Err("a password cost needs at least 1 lane, not 0".to_string())
        );
        assert!(PasswordCost::new(19_456, 0, 1).unwrap_err().contains("at least 1 iteration"));
        assert!(PasswordCost::new(i64::from(MAX_MEMORY_KIB) + 1, 2, 1).unwrap_err().contains("4 GiB"));
        assert!(PasswordCost::new(19_456, i64::from(u32::MAX) + 1, 1).unwrap_err().contains("too large"));
        // Argon2id needs 8 KiB per lane.
        assert!(PasswordCost::new(19_456, 2, 4_096).unwrap_err().starts_with("Argon2id cannot run"));
    }

    #[test]
    fn a_malformed_or_foreign_hash_is_an_error_that_never_quotes_it() {
        let reference = "$argon2id$v=19$m=19456,t=2,p=1$AAECAwQFBgcICQoLDA0ODw$gYJZtjEAJqjg26xdLmknq8/bB7MiWPrE9hsYuA+SkIU";
        for malformed in ["", "not a hash", "$argon2id$v=19$m=19456", &reference[..reference.len() - 50]] {
            let problem = verify_password(b"x", malformed).unwrap_err();
            assert!(malformed.is_empty() || !problem.contains(malformed), "{problem}");
        }
        let argon2i = reference.replacen("argon2id", "argon2i", 1);
        assert_eq!(verify_password(b"x", &argon2i), Err("the hash is `argon2i`, not `argon2id`".to_string()));
        let old_version = reference.replacen("v=19", "v=16", 1);
        assert!(verify_password(b"x", &old_version).unwrap_err().contains("version 19"));
        let huge = reference.replacen("m=19456", "m=4294967295", 1);
        assert!(verify_password(b"x", &huge).unwrap_err().contains("above the limit"));
        let bcrypt = "$2b$12$R9h/cIPz0gi.URNNX3kh2OPST9/PgBkqquzi.Ss7KIUgO2t0jWMUW";
        assert!(verify_password(b"x", bcrypt).is_err());
    }

    #[test]
    fn a_hash_made_by_the_argon2_crate_default_verifies() {
        // The API-key store hashed with `Argon2::default()` before it
        // moved here; those stored hashes must keep verifying.
        let salt = SaltString::encode_b64(&[7u8; 16]).unwrap();
        let legacy = Argon2::default().hash_password(b"ak_live_legacy", &salt).unwrap().to_string();
        assert!(legacy.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{legacy}");
        assert_eq!(verify_password(b"ak_live_legacy", &legacy), Ok(true));
        assert_eq!(verify_password(b"ak_live_other", &legacy), Ok(false));
    }
}
