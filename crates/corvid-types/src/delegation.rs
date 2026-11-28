//! How long a person may grant an assistant authority (B11): the
//! identity block's `delegation: up_to <duration>`.
//!
//! One function reads the cap, so the checker's error, the contract and
//! the grant endpoint's bound are the same answer. A grant can never be
//! permanent, and none can be shorter than a minute. The longest is 720
//! hours: a duration counts exact time, and a day is not a fixed length
//! (in a time zone it is 23, 24 or 25 hours), so the cap is written in
//! hours, minutes or seconds.

use corvid_ast::DelegationPolicy;

/// The shortest cap: one minute.
pub const SHORTEST_SECONDS: u64 = 60;
/// The longest cap: 720 hours, thirty days of exactly 24 hours.
pub const LONGEST_SECONDS: u64 = 720 * 60 * 60;

/// The cap in whole seconds, or why it is not a usable one.
pub fn cap_seconds(policy: &DelegationPolicy) -> Result<u64, String> {
    let written = format!("{} {}", policy.count, policy.unit);
    let duration = crate::duration_literal_value(&policy.count, &policy.unit)?;
    let nanoseconds = duration.nanoseconds();
    if nanoseconds % 1_000_000_000 != 0 {
        return Err(format!("`{written}` is not a whole number of seconds"));
    }
    let seconds = u64::try_from(nanoseconds / 1_000_000_000).unwrap_or(u64::MAX);
    if !(SHORTEST_SECONDS..=LONGEST_SECONDS).contains(&seconds) {
        return Err(format!(
            "`delegation: up_to {written}` is outside the allowed range: a grant may last from `1 minute` to `720 hours` (thirty days), so it can never be permanent"
        ));
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(count: &str, unit: &str) -> Result<u64, String> {
        cap_seconds(&DelegationPolicy { count: count.into(), unit: unit.into(), span: corvid_ast::Span::new(0, 0) })
    }

    #[test]
    fn a_cap_is_between_a_minute_and_thirty_days() {
        assert_eq!(cap("8", "hours"), Ok(8 * 3600));
        assert_eq!(cap("1", "minute"), Ok(60));
        assert_eq!(cap("720", "hours"), Ok(LONGEST_SECONDS));
        assert!(cap("59", "seconds").unwrap_err().contains("1 minute"));
        assert!(cap("721", "hours").unwrap_err().contains("never be permanent"));
        assert!(cap("30", "days").unwrap_err().contains("no fixed length"), "a day is not a fixed length");
    }
}
