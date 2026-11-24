//! `TimeZone`: an IANA time zone from the bundled database.

use crate::error::TimeError;
use chrono_tz::{Tz, TZ_VARIANTS};
use std::fmt;

/// The version of the IANA time zone database compiled into Corvid,
/// such as `2025b`. Zone rules change between versions, so a recorded
/// trace names the version it ran with.
pub const TZDB_VERSION: &str = chrono_tz::IANA_TZDB_VERSION;

/// An IANA time zone, such as `Europe/Berlin`. A zone carries its
/// daylight-saving rules; a fixed offset such as `+02:00` is not a zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeZone {
    zone: Tz,
}

impl TimeZone {
    pub const UTC: TimeZone = TimeZone { zone: Tz::UTC };

    /// Looks a zone up by its IANA name. An unknown name is an error
    /// that suggests the closest known name, so `Europe/Berln` points
    /// at `Europe/Berlin` and `europe/berlin` at its correct spelling.
    pub fn parse(name: &str) -> Result<TimeZone, TimeError> {
        if let Ok(zone) = name.parse::<Tz>() {
            return Ok(TimeZone { zone });
        }
        Err(TimeError::UnknownZone { name: name.to_string(), suggestion: closest_zone_name(name) })
    }

    /// Corvid's `text.parse_time_zone()`: trims whitespace; the error
    /// suggests the closest name. Both tiers call this.
    pub fn parse_user_text(text: &str) -> Result<TimeZone, String> {
        TimeZone::parse(text.trim()).map_err(|error| error.to_string())
    }

    pub fn name(&self) -> &'static str {
        self.zone.name()
    }

    /// The `chrono-tz` zone, for runtime code that schedules with it.
    pub fn chrono_zone(&self) -> Tz {
        self.zone
    }
}

impl fmt::Display for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The known zone name nearest to `name`: an exact match ignoring case,
/// or otherwise the name within a small edit distance, if one exists.
fn closest_zone_name(name: &str) -> Option<&'static str> {
    let wanted = name.to_ascii_lowercase();
    if let Some(zone) = TZ_VARIANTS.iter().find(|zone| zone.name().to_ascii_lowercase() == wanted) {
        return Some(zone.name());
    }
    let limit = (wanted.len() / 4).clamp(1, 3);
    TZ_VARIANTS
        .iter()
        .map(|zone| (edit_distance(&wanted, &zone.name().to_ascii_lowercase()), zone.name()))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, name)| (*distance, *name))
        .map(|(_, name)| name)
}

/// Levenshtein distance over bytes; zone names are ASCII.
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, &cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_is_trimmed_and_its_error_suggests_a_name() {
        assert_eq!(TimeZone::parse_user_text(" Europe/Berlin ").unwrap().name(), "Europe/Berlin");
        assert_eq!(
            TimeZone::parse_user_text("Europe/Berln").unwrap_err(),
            "`Europe/Berln` is not a time zone in the IANA database; did you mean `Europe/Berlin`?"
        );
    }

    #[test]
    fn known_zones_parse() {
        assert_eq!(TimeZone::parse("Europe/Berlin").unwrap().name(), "Europe/Berlin");
        assert_eq!(TimeZone::parse("UTC").unwrap(), TimeZone::UTC);
        assert_eq!(TimeZone::parse("America/Argentina/Buenos_Aires").unwrap().to_string(), "America/Argentina/Buenos_Aires");
    }

    #[test]
    fn unknown_zones_suggest_the_closest_name() {
        let suggestion = |name: &str| match TimeZone::parse(name) {
            Err(TimeError::UnknownZone { suggestion, .. }) => suggestion,
            other => panic!("expected an unknown zone, got {other:?}"),
        };
        assert_eq!(suggestion("Europe/Berln"), Some("Europe/Berlin"));
        assert_eq!(suggestion("europe/berlin"), Some("Europe/Berlin"));
        assert_eq!(suggestion("Mars/Olympus_Mons"), None);
        let message = TimeZone::parse("Europe/Berln").unwrap_err().to_string();
        assert_eq!(message, "`Europe/Berln` is not a time zone in the IANA database; did you mean `Europe/Berlin`?");
    }

    #[test]
    fn a_fixed_offset_is_not_a_zone() {
        assert!(TimeZone::parse("+02:00").is_err());
    }

    #[test]
    fn the_database_version_is_named() {
        assert!(TZDB_VERSION.len() >= 5 && TZDB_VERSION.starts_with("20"), "{TZDB_VERSION}");
    }
}
