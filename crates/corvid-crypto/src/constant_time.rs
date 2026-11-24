//! Constant-time comparison, the meaning of `==` on `Bytes` and `Secret`.
//!
//! Comparing two byte strings of the same length takes the same time
//! whichever bytes differ, so comparing a computed signature with a
//! received one, or a supplied password with a stored one, cannot leak
//! it byte by byte. Byte strings of different lengths are unequal at
//! once, which reveals only their lengths.

use subtle::ConstantTimeEq;

/// Whether `a` and `b` hold the same bytes, in constant time for equal
/// lengths.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_every_byte_and_the_length() {
        assert!(constant_time_eq(b"abcd", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abce"));
        assert!(!constant_time_eq(b"abcd", b"xbcd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
