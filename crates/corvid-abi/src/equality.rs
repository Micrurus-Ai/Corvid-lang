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
    /// Native dispatch N3 — a `Secret` object, compared in constant time
    /// by the runtime, which alone can read it.
    Secret,
}

impl EqualityLayout {
    /// The layout of a type the runtime compares as one slot by value:
    /// `String`, and each type native code holds as its canonical text
    /// (B06, B04). The comparison layouts and native map keys both ask
    /// this, so a map finds a key exactly when `==` calls them equal.
    pub fn of_text_slot(ty: &corvid_types::Type) -> Option<EqualityLayout> {
        use corvid_types::Type;
        Some(match ty {
            Type::String => Self::String,
            Type::Decimal => Self::Decimal,
            Type::Duration => Self::Duration,
            Type::Instant => Self::Instant,
            Type::TimeZone => Self::TimeZone,
            Type::ZonedTime => Self::ZonedTime,
            Type::Currency => Self::Currency,
            Type::Locale => Self::Locale,
            Type::Bytes => Self::Bytes,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    /// `ordered_map.c` names the `String` key kind `CORVID_KEY_STRING` 3.
    #[test]
    fn the_c_map_runtime_agrees_on_the_string_key_kind() {
        assert_eq!(super::EqualityLayout::String as u32, 3);
    }
}
