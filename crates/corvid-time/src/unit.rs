//! The fixed-length units a `Duration` is counted in.

/// A unit of exact elapsed time. Days and longer are deliberately
/// absent: in a time zone a day is 23, 24 or 25 hours, so a calendar
/// day is a `ZonedTime` operation, not a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TimeUnit {
    Nanoseconds,
    Microseconds,
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
}

impl TimeUnit {
    pub const ALL: [TimeUnit; 6] = [
        TimeUnit::Nanoseconds,
        TimeUnit::Microseconds,
        TimeUnit::Milliseconds,
        TimeUnit::Seconds,
        TimeUnit::Minutes,
        TimeUnit::Hours,
    ];

    pub fn nanoseconds(self) -> i128 {
        match self {
            TimeUnit::Nanoseconds => 1,
            TimeUnit::Microseconds => 1_000,
            TimeUnit::Milliseconds => 1_000_000,
            TimeUnit::Seconds => 1_000_000_000,
            TimeUnit::Minutes => 60_000_000_000,
            TimeUnit::Hours => 3_600_000_000_000,
        }
    }

    /// The plural name, as in the `whole_seconds()` reading.
    pub fn name(self) -> &'static str {
        match self {
            TimeUnit::Nanoseconds => "nanoseconds",
            TimeUnit::Microseconds => "microseconds",
            TimeUnit::Milliseconds => "milliseconds",
            TimeUnit::Seconds => "seconds",
            TimeUnit::Minutes => "minutes",
            TimeUnit::Hours => "hours",
        }
    }

    pub fn from_name(name: &str) -> Option<TimeUnit> {
        TimeUnit::ALL.into_iter().find(|unit| unit.name() == name)
    }
}
