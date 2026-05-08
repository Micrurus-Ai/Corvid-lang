//! Compile-time string-literal pool for the WASM target.
//!
//! Every `IrLiteral::String` encountered during codegen is interned
//! into a contiguous byte buffer and assigned a stable offset. The
//! buffer is later emitted as a single `DataSection` segment at
//! linear-memory offset `HEAP_BASE` (see `allocator.rs`); the
//! allocator's `$heap_top` global is initialised to
//! `HEAP_BASE + pool.total_bytes()` so the heap starts immediately
//! after the literal pool with no addressable overlap.
//!
//! Why interning by content: agents commonly reuse short literals
//! ("ok", "error", domain enum strings). Deduplication by content
//! shrinks the WASM binary and keeps `$heap_top`'s initial value
//! tighter, which means fewer `memory.grow` calls before the first
//! real allocation.
//!
//! ## What's NOT in v1
//!
//! - **Range coalescing.** Two literals like "hello" and "ello"
//!   could share storage if pools were range-aware (the second is
//!   a suffix of the first). v1 stores each unique full literal
//!   independently. Suffix sharing is a size optimisation, not a
//!   correctness concern.
//! - **Reference counting / garbage collection of unused literals.**
//!   The pool keeps every literal it sees during the walk, even if
//!   the literal is on a dead-code branch. Matches the codegen-as-a-
//!   whole's "we lower what the IR contains" stance.

use std::collections::HashMap;

/// A contiguous byte buffer of compile-time-known string literals,
/// indexed by content for deduplication.
#[derive(Default)]
pub(crate) struct StringPool {
    /// Maps each unique literal value to its offset within the
    /// pool. Lookups by content for deduplication; offsets are
    /// stable once issued.
    interned: HashMap<String, u32>,
    /// The concatenated bytes. Each literal lands at offset
    /// `interned[value]` and runs for `value.as_bytes().len()`
    /// bytes.
    bytes: Vec<u8>,
}

impl StringPool {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Intern `value` into the pool. Returns `(offset, len)` where
    /// `offset` is relative to the pool start (NOT the absolute
    /// linear-memory address — callers add `HEAP_BASE` to get the
    /// runtime pointer) and `len` is the UTF-8 byte length of the
    /// value.
    ///
    /// Calling `intern` with the same `value` twice returns the
    /// same `(offset, len)` pair both times (interning by content).
    pub(crate) fn intern(&mut self, value: &str) -> (u32, u32) {
        let len = value.as_bytes().len() as u32;
        if let Some(&existing_offset) = self.interned.get(value) {
            return (existing_offset, len);
        }
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(value.as_bytes());
        self.interned.insert(value.to_string(), offset);
        (offset, len)
    }

    /// Look up a previously-interned literal. Panics if the value
    /// hasn't been interned yet — callers should `intern` during a
    /// pre-codegen walk before lowering bodies that reference the
    /// literal.
    pub(crate) fn lookup(&self, value: &str) -> (u32, u32) {
        let len = value.as_bytes().len() as u32;
        let offset = *self
            .interned
            .get(value)
            .expect("string literal looked up before intern");
        (offset, len)
    }

    /// Total byte length of the pool. Equals `bytes.len()` when the
    /// pool is the only allocator-relevant data segment.
    pub(crate) fn total_bytes(&self) -> u32 {
        self.bytes.len() as u32
    }

    /// The concatenated literal bytes. Caller emits this as a
    /// `DataSection` active segment with offset = `HEAP_BASE`.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::StringPool;

    #[test]
    fn intern_assigns_sequential_offsets() {
        let mut pool = StringPool::new();
        assert_eq!(pool.intern("hello"), (0, 5));
        assert_eq!(pool.intern("world"), (5, 5));
        assert_eq!(pool.total_bytes(), 10);
        assert_eq!(pool.bytes(), b"helloworld");
    }

    #[test]
    fn intern_deduplicates_by_content() {
        let mut pool = StringPool::new();
        let first = pool.intern("hello");
        let dup = pool.intern("hello");
        assert_eq!(first, dup);
        assert_eq!(pool.total_bytes(), 5);
    }

    #[test]
    fn intern_handles_multi_byte_utf8() {
        let mut pool = StringPool::new();
        // "héllo 🦀" — 'é' is 2 bytes, '🦀' is 4 bytes; total 11.
        let value = "héllo 🦀";
        let (offset, len) = pool.intern(value);
        assert_eq!(offset, 0);
        assert_eq!(len as usize, value.as_bytes().len());
        assert_eq!(pool.bytes(), value.as_bytes());
    }

    #[test]
    fn lookup_returns_same_offset_as_intern() {
        let mut pool = StringPool::new();
        let (interned_offset, interned_len) = pool.intern("hello");
        let (lookup_offset, lookup_len) = pool.lookup("hello");
        assert_eq!(interned_offset, lookup_offset);
        assert_eq!(interned_len, lookup_len);
    }

    #[test]
    fn empty_string_round_trips() {
        let mut pool = StringPool::new();
        assert_eq!(pool.intern(""), (0, 0));
        assert_eq!(pool.total_bytes(), 0);
        // Calling intern again on the empty string still returns the
        // same (0, 0).
        assert_eq!(pool.intern(""), (0, 0));
    }
}
