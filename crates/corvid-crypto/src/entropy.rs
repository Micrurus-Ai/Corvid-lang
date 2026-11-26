//! Random bytes from the operating system.
//!
//! Every random value Corvid draws for security (a token, a password
//! salt, a UUID) comes from the operating system's cryptographically
//! secure generator through this one function: `getrandom` on Linux,
//! `getentropy` on macOS, `BCryptGenRandom` on Windows, through
//! `rand_core`'s `OsRng`. There is no seeded or user-space generator to
//! misconfigure.

use rand_core::{OsRng, RngCore};

/// Fills `bytes` from the operating system's secure generator. An error
/// only when the operating system cannot supply entropy.
pub fn fill_random(bytes: &mut [u8]) -> Result<(), String> {
    OsRng
        .try_fill_bytes(bytes)
        .map_err(|problem| format!("the operating system's entropy source failed: {problem}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_draws_differ() {
        let mut first = [0u8; 32];
        let mut second = [0u8; 32];
        fill_random(&mut first).unwrap();
        fill_random(&mut second).unwrap();
        assert_ne!(first, second);
        assert_ne!(first, [0u8; 32]);
    }
}
