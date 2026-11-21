/// Parse a signed decimal Corvid Int after trimming Unicode whitespace.
pub fn parse_int(text: &str) -> Result<i64, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("not an integer: `{text}`"))
}

/// Slice at Unicode scalar boundaries. Indices outside the string clamp to an end.
pub fn substring(text: &str, start: i64, end: i64) -> &str {
    let start = start.max(0);
    if end <= start {
        return "";
    }
    let mut boundaries = text.char_indices();
    let index = usize::try_from(start).unwrap_or(usize::MAX);
    let Some((first, _)) = boundaries.nth(index) else { return "" };
    let remaining = usize::try_from(end - start - 1).unwrap_or(usize::MAX);
    let last = boundaries.nth(remaining).map_or(text.len(), |(offset, _)| offset);
    &text[first..last]
}
