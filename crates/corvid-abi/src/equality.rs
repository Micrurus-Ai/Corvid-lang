//! Native comparison layouts are tables of u32 words. Child references are
//! word offsets from the start of the table, so recursive types need no relocations.
//! Records and sums store a count followed by child offsets. A sum's children
//! describe the positional fields of each variant, excluding its tag.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EqualityLayout {
    Int,
    Bool,
    Float,
    String,
    Record,
    Sum,
    List,
    Option,
    BoxedOption,
    Result,
    StringMap,
    Identity,
    /// B06 — a `Decimal` held as its canonical text, compared by
    /// numeric value, so `1.0` equals `1.00`.
    Decimal,
    /// B06 — the time and currency types, each held as its canonical
    /// text and compared by value through its own crate.
    Duration,
    Instant,
    TimeZone,
    ZonedTime,
    Currency,
    Locale,
    /// B04 — a `Bytes` value held as its canonical base64url text,
    /// compared in constant time, never bytewise like a `String`.
    Bytes,
}
