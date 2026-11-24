//! Exact time for Corvid's `Duration` and `Instant` types (B06).
//!
//! Every rule behind the two types lives here once: checked
//! arithmetic, the ISO 8601 duration text and the RFC 3339 instant
//! text. The interpreter and the native runtime execute through these
//! functions, and the text forms are the ones traces, tools and
//! prompts carry, so the tiers cannot disagree on a nanosecond. The
//! crate depends on no other Corvid crate.
//!
//! Both types count nanoseconds in a signed 128-bit integer. An
//! instant lies between 0001-01-01 and 9999-12-31 UTC, the range RFC
//! 3339 can write, and a duration is at most the span of that range.
//! Every operation either produces the exact result or returns a
//! [`TimeError`].

mod duration;
mod duration_text;
mod error;
mod instant;
mod unit;

pub use duration::Duration;
pub use error::TimeError;
pub use instant::Instant;
pub use unit::TimeUnit;
