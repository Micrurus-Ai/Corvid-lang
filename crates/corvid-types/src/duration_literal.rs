//! The value of a duration literal (B06): `30 seconds`, `1.5 hours`.
//!
//! One function reads it, so the checker's error and the value IR
//! lowering builds are the same answer. The count is read from its
//! source text as an exact decimal, never through a float, and the
//! result must be a whole number of nanoseconds inside the duration
//! range. A calendar unit has no fixed length and is refused with the
//! two things a program usually means.

use corvid_time::{Duration, TimeUnit};

/// The exact duration `count unit` names, or why it is not one.
/// `unit` is the unit word as written, singular or plural.
pub fn duration_literal_value(count: &str, unit: &str) -> Result<Duration, String> {
    let written = format!("{count} {unit}");
    let plural = corvid_ast::duration_unit(unit).ok_or_else(|| format!("`{written}`: `{unit}` is not a duration unit"))?;
    let Some(unit) = TimeUnit::from_name(plural) else {
        return Err(calendar_unit(&written, plural));
    };
    let decimal = corvid_decimal::Decimal::parse(count)
        .map_err(|_| format!("`{written}`: `{count}` is not a number a duration can count"))?;
    let scaled = decimal
        .coefficient()
        .checked_mul(unit.nanoseconds())
        .ok_or_else(|| out_of_range(&written))?;
    let divisor = 10_i128.pow(u32::from(decimal.scale()));
    if scaled % divisor != 0 {
        return Err(format!("`{written}` is not a whole number of nanoseconds"));
    }
    Duration::from_nanoseconds(scaled / divisor).map_err(|_| out_of_range(&written))
}

fn out_of_range(written: &str) -> String {
    format!("`{written}` is longer than any duration can be (the span of the instant range, 0001 to 9999)")
}

fn calendar_unit(written: &str, plural: &str) -> String {
    let calendar = match plural {
        "days" | "weeks" => "add_days",
        "months" => "add_months",
        _ => "add_years",
    };
    format!(
        "`{written}` has no fixed length: in a time zone a day is 23, 24 or 25 hours. \
         For an exact length write it in hours (`24 hours` is one day of exactly 24 hours); \
         for calendar time, move a `ZonedTime` with `{calendar}`, which keeps the local time"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_is_read_exactly() {
        assert_eq!(duration_literal_value("30", "seconds").unwrap().to_string(), "PT30S");
        assert_eq!(duration_literal_value("1.5", "hours").unwrap().to_string(), "PT1H30M");
        assert_eq!(duration_literal_value("0.000000001", "seconds").unwrap().nanoseconds(), 1);
        assert_eq!(duration_literal_value("250", "milliseconds").unwrap().nanoseconds(), 250_000_000);
    }

    #[test]
    fn what_is_not_a_fixed_exact_length_is_refused_with_the_reason() {
        let fraction = duration_literal_value("1.5", "nanoseconds").unwrap_err();
        assert!(fraction.contains("not a whole number of nanoseconds"), "{fraction}");
        let day = duration_literal_value("30", "days").unwrap_err();
        assert!(day.contains("has no fixed length") && day.contains("24 hours") && day.contains("add_days"), "{day}");
        assert!(duration_literal_value("2", "months").unwrap_err().contains("add_months"));
        assert!(duration_literal_value("1", "years").unwrap_err().contains("add_years"));
        let huge = duration_literal_value("99999999999999", "hours").unwrap_err();
        assert!(huge.contains("longer than any duration"), "{huge}");
    }

    /// The parser's unit words and the time crate's units are one list.
    #[test]
    fn the_fixed_units_are_the_time_crates_units() {
        let fixed: Vec<&str> = TimeUnit::ALL.iter().map(|unit| unit.name()).collect();
        assert_eq!(&corvid_ast::DURATION_UNITS[..fixed.len()], fixed.as_slice());
        assert_eq!(corvid_ast::duration_unit("second"), Some("seconds"));
        assert_eq!(corvid_ast::duration_unit("hours"), Some("hours"));
        assert_eq!(corvid_ast::duration_unit("day"), Some("days"));
        assert_eq!(corvid_ast::duration_unit("secs"), None);
    }
}
