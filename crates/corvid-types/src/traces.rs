//! How long a served program keeps its run traces (lineage L3b): the
//! `traces:` block's `keep:`.
//!
//! One function reads it, so the checker's error and the server's
//! deletion use the same answer. A trace holds what people sent, so it is
//! never kept forever, and never for less than an hour, which would
//! delete a run before anyone could look at it. A day is not a fixed
//! length, so the time is written in hours, minutes or seconds, as the
//! delegation cap is.

/// The shortest retention: one hour.
pub const SHORTEST_SECONDS: u64 = 60 * 60;
/// The longest retention: 8,784 hours, a leap year.
pub const LONGEST_SECONDS: u64 = 8_784 * 60 * 60;

/// `keep:` in whole seconds, or why it is not usable.
pub fn keep_seconds(count: &str, unit: &str) -> Result<u64, String> {
    let written = format!("{count} {unit}");
    let duration = crate::duration_literal_value(count, unit)?;
    let nanoseconds = duration.nanoseconds();
    if nanoseconds % 1_000_000_000 != 0 {
        return Err(format!("`keep: {written}` is not a whole number of seconds"));
    }
    let seconds = u64::try_from(nanoseconds / 1_000_000_000).unwrap_or(u64::MAX);
    if !(SHORTEST_SECONDS..=LONGEST_SECONDS).contains(&seconds) {
        return Err(format!(
            "`keep: {written}` is outside the allowed range: a trace is kept from `1 hour` to `8784 hours` (a year)"
        ));
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_is_kept_between_an_hour_and_a_year() {
        assert_eq!(keep_seconds("720", "hours"), Ok(720 * 3600));
        assert_eq!(keep_seconds("1", "hour"), Ok(3600));
        assert!(keep_seconds("59", "minutes").unwrap_err().contains("1 hour"));
        assert!(keep_seconds("8785", "hours").unwrap_err().contains("a year"));
        assert!(keep_seconds("30", "days").is_err(), "a day is not a fixed length");
    }
}
