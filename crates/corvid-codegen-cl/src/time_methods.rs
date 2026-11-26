//! The `Duration`, `Instant`, `TimeZone` and `ZonedTime` operations
//! native code calls (B06): which runtime symbol runs each operator and
//! method, and how many words it takes.
//!
//! Native code holds each of these types as a String containing its
//! canonical text, the text its JSON form carries. Every operation is
//! one call into `corvid-runtime/src/ffi_bridge/time.rs`, which parses
//! the operands, computes with `corvid-time` (the code the interpreter
//! calls) and returns canonical text or an `Int`. Every parameter and
//! return is one word: a String pointer, an `Int`, or a Result payload
//! slot. The import declarations, the lowering, the tier picker and
//! native match arms all read this table, so an operation is native
//! everywhere or nowhere. A misspelled symbol fails to link.
//!
//! `time_now()` is not here: it reads the clock through a `std/time`
//! tool, whose `Instant` return keeps a program that calls it on the
//! interpreter (see `crate::text_scalar_boundary`).

use crate::text_scalar::{scalar_of, BinaryReading, NativeScalarMethod, ScalarBinary};
use corvid_ast::BinaryOp;
use corvid_time::{TimeUnit, ZonedField};
use corvid_types::{BuiltinMethodKind, Type};

/// One runtime entry point for the time types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeOperation {
    DurationAdd,
    DurationSub,
    /// `d * n`; `n * d` swaps its operands into this order.
    DurationMul,
    DurationDiv,
    DurationNegate,
    DurationAbs,
    /// Order by length as -1, 0 or 1; every comparison of durations.
    DurationCompare,
    DurationWhole(TimeUnit),
    /// `i + d`; `d + i` swaps its operands into this order.
    InstantAddDuration,
    InstantSubDuration,
    InstantSubInstant,
    /// Order on the timeline as -1, 0 or 1; every comparison of instants.
    InstantCompare,
    InstantEpochMilliseconds,
    /// `Instant.from_epoch_milliseconds(n)`, whose receiver is the `Int`.
    InstantFromEpochMilliseconds,
    InstantInZone,
    /// `z + d`; `d + z` swaps its operands into this order.
    ZonedAddDuration,
    ZonedSubDuration,
    ZonedSubZoned,
    /// 1 when two zoned times are the same instant in the same zone.
    ZonedEqual,
    ZonedField(ZonedField),
    ZonedZone,
    ZonedInstant,
    ZonedAddDays,
    ZonedAddMonths,
    ZonedAddYears,
    ZonedWithZone,
    /// RFC 9557 text; a call, not the stored text, because writing it
    /// reads the zone's rules and replay reports that.
    ZonedToString,
    /// 1 when two zones are the same zone.
    TimeZoneEqual,
    ParseDuration,
    ParseInstant,
    ParseTimeZone,
    ParseZonedTime,
    /// `text.parse_local_time(zone)`: the text, the zone, the payload slot.
    ParseLocalTime,
}

impl TimeOperation {
    /// Every operation, for declaring the imports.
    pub fn all() -> Vec<TimeOperation> {
        use TimeOperation::*;
        let mut all = vec![
            DurationAdd,
            DurationSub,
            DurationMul,
            DurationDiv,
            DurationNegate,
            DurationAbs,
            DurationCompare,
            InstantAddDuration,
            InstantSubDuration,
            InstantSubInstant,
            InstantCompare,
            InstantEpochMilliseconds,
            InstantFromEpochMilliseconds,
            InstantInZone,
            ZonedAddDuration,
            ZonedSubDuration,
            ZonedSubZoned,
            ZonedEqual,
            ZonedZone,
            ZonedInstant,
            ZonedAddDays,
            ZonedAddMonths,
            ZonedAddYears,
            ZonedWithZone,
            ZonedToString,
            TimeZoneEqual,
            ParseDuration,
            ParseInstant,
            ParseTimeZone,
            ParseZonedTime,
            ParseLocalTime,
        ];
        all.extend(TimeUnit::ALL.into_iter().map(DurationWhole));
        all.extend(corvid_time::ZonedField::ALL.into_iter().map(ZonedField));
        all
    }

    /// The runtime symbol. A reading or field symbol carries the
    /// method's name (`corvid_duration_whole_seconds`,
    /// `corvid_zoned_time_offset_seconds`).
    pub fn symbol(self) -> String {
        use TimeOperation::*;
        match self {
            DurationAdd => "corvid_duration_add".into(),
            DurationSub => "corvid_duration_sub".into(),
            DurationMul => "corvid_duration_mul".into(),
            DurationDiv => "corvid_duration_div".into(),
            DurationNegate => "corvid_duration_negate".into(),
            DurationAbs => "corvid_duration_abs".into(),
            DurationCompare => "corvid_duration_compare".into(),
            DurationWhole(unit) => format!("corvid_duration_whole_{}", unit.name()),
            InstantAddDuration => "corvid_instant_add_duration".into(),
            InstantSubDuration => "corvid_instant_sub_duration".into(),
            InstantSubInstant => "corvid_instant_sub_instant".into(),
            InstantCompare => "corvid_instant_compare".into(),
            InstantEpochMilliseconds => "corvid_instant_epoch_milliseconds".into(),
            InstantFromEpochMilliseconds => "corvid_instant_from_epoch_milliseconds".into(),
            InstantInZone => "corvid_instant_in_zone".into(),
            ZonedAddDuration => "corvid_zoned_time_add_duration".into(),
            ZonedSubDuration => "corvid_zoned_time_sub_duration".into(),
            ZonedSubZoned => "corvid_zoned_time_sub_zoned_time".into(),
            ZonedEqual => "corvid_zoned_time_equal".into(),
            ZonedField(field) => format!("corvid_zoned_time_{}", field.name()),
            ZonedZone => "corvid_zoned_time_zone".into(),
            ZonedInstant => "corvid_zoned_time_instant".into(),
            ZonedAddDays => "corvid_zoned_time_add_days".into(),
            ZonedAddMonths => "corvid_zoned_time_add_months".into(),
            ZonedAddYears => "corvid_zoned_time_add_years".into(),
            ZonedWithZone => "corvid_zoned_time_with_zone".into(),
            ZonedToString => "corvid_zoned_time_to_string".into(),
            TimeZoneEqual => "corvid_time_zone_equal".into(),
            ParseDuration => "corvid_string_parse_duration".into(),
            ParseInstant => "corvid_string_parse_instant".into(),
            ParseTimeZone => "corvid_string_parse_time_zone".into(),
            ParseZonedTime => "corvid_string_parse_zoned_time".into(),
            ParseLocalTime => "corvid_string_parse_local_time".into(),
        }
    }

    /// How many one-word parameters the entry point takes; every entry
    /// point returns one word.
    pub fn arity(self) -> usize {
        use TimeOperation::*;
        match self {
            DurationNegate | DurationAbs | DurationWhole(_) | InstantEpochMilliseconds | InstantFromEpochMilliseconds
            | ZonedField(_) | ZonedZone | ZonedInstant | ZonedToString => 1,
            ParseLocalTime => 3,
            _ => 2,
        }
    }

    /// Whether this fills a `Result<T, String>` payload slot and returns its tag.
    pub fn is_parse(self) -> bool {
        use TimeOperation::*;
        matches!(self, ParseDuration | ParseInstant | ParseTimeZone | ParseZonedTime | ParseLocalTime)
    }
}

/// The operation behind `left op right` when either side is a time
/// type, per the checker's table in `corvid-types/src/checker/time.rs`.
pub fn time_binary(op: BinaryOp, left: &Type, right: &Type) -> Option<ScalarBinary<TimeOperation>> {
    use BinaryOp::*;
    use TimeOperation::*;
    use Type::{Duration, Instant, Int, TimeZone, ZonedTime};
    let (left, right) = (scalar_of(left), scalar_of(right));
    let value = |operation, swapped| ScalarBinary { operation, swapped, reading: BinaryReading::Value };
    let order = |operation| ScalarBinary { operation, swapped: false, reading: BinaryReading::Order };
    let equality = |operation| ScalarBinary { operation, swapped: false, reading: BinaryReading::Equality };
    Some(match (op, left, right) {
        (Add, Duration, Duration) => value(DurationAdd, false),
        (Sub, Duration, Duration) => value(DurationSub, false),
        (Mul, Duration, Int) => value(DurationMul, false),
        (Mul, Int, Duration) => value(DurationMul, true),
        (Div, Duration, Int) => value(DurationDiv, false),
        (Add, Instant, Duration) => value(InstantAddDuration, false),
        (Add, Duration, Instant) => value(InstantAddDuration, true),
        (Sub, Instant, Duration) => value(InstantSubDuration, false),
        (Sub, Instant, Instant) => value(InstantSubInstant, false),
        (Add, ZonedTime, Duration) => value(ZonedAddDuration, false),
        (Add, Duration, ZonedTime) => value(ZonedAddDuration, true),
        (Sub, ZonedTime, Duration) => value(ZonedSubDuration, false),
        (Sub, ZonedTime, ZonedTime) => value(ZonedSubZoned, false),
        (Eq | NotEq | Lt | LtEq | Gt | GtEq, Duration, Duration) => order(DurationCompare),
        (Eq | NotEq | Lt | LtEq | Gt | GtEq, Instant, Instant) => order(InstantCompare),
        (Eq | NotEq, ZonedTime, ZonedTime) => equality(ZonedEqual),
        (Eq | NotEq, TimeZone, TimeZone) => equality(TimeZoneEqual),
        _ => return None,
    })
}

/// Whether either operand is a time type, so the operator must lower
/// through [`time_binary`] or be refused, never as integer arithmetic
/// on the text pointers.
pub fn has_time_operand(left: &Type, right: &Type) -> bool {
    [left, right]
        .into_iter()
        .any(|ty| matches!(scalar_of(ty), Type::Duration | Type::Instant | Type::TimeZone | Type::ZonedTime))
}

/// The native form of a time method. Every `is_time()` kind has one.
pub fn native_time_method(kind: BuiltinMethodKind) -> Option<NativeScalarMethod<TimeOperation>> {
    use BuiltinMethodKind::*;
    use NativeScalarMethod::*;
    use TimeOperation as Op;
    Some(match kind {
        DurationWhole(unit) => Call(Op::DurationWhole(unit)),
        DurationAbs => Call(Op::DurationAbs),
        DurationToString | InstantToString | TimeZoneToString => CanonicalText,
        InstantEpochMilliseconds => Call(Op::InstantEpochMilliseconds),
        InstantFromEpochMilliseconds => Call(Op::InstantFromEpochMilliseconds),
        StringParseDuration => Call(Op::ParseDuration),
        StringParseInstant => Call(Op::ParseInstant),
        InstantInZone => Call(Op::InstantInZone),
        ZonedTimeField(field) => Call(Op::ZonedField(field)),
        ZonedTimeZone => Call(Op::ZonedZone),
        ZonedTimeInstant => Call(Op::ZonedInstant),
        ZonedTimeAddDays => Call(Op::ZonedAddDays),
        ZonedTimeAddMonths => Call(Op::ZonedAddMonths),
        ZonedTimeAddYears => Call(Op::ZonedAddYears),
        ZonedTimeWithZone => Call(Op::ZonedWithZone),
        ZonedTimeToString => Call(Op::ZonedToString),
        StringParseTimeZone => Call(Op::ParseTimeZone),
        StringParseZonedTime => Call(Op::ParseZonedTime),
        StringParseLocalTime => Call(Op::ParseLocalTime),
        _ => return None,
    })
}

/// Whether native code compiles this time method.
pub fn is_native_time_method(kind: BuiltinMethodKind) -> bool {
    native_time_method(kind).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_time_method_has_a_native_form() {
        use BuiltinMethodKind::*;
        let mut kinds = vec![
            DurationAbs, DurationToString, InstantEpochMilliseconds, InstantToString, InstantFromEpochMilliseconds,
            StringParseDuration, StringParseInstant, InstantInZone, ZonedTimeZone, ZonedTimeInstant, ZonedTimeAddDays,
            ZonedTimeAddMonths, ZonedTimeAddYears, ZonedTimeWithZone, ZonedTimeToString, TimeZoneToString,
            StringParseTimeZone, StringParseZonedTime, StringParseLocalTime,
        ];
        kinds.extend(TimeUnit::ALL.into_iter().map(DurationWhole));
        kinds.extend(ZonedField::ALL.into_iter().map(ZonedTimeField));
        for kind in kinds {
            assert!(kind.is_time(), "{kind:?}");
            assert!(is_native_time_method(kind), "{kind:?} needs a native form");
        }
    }

    #[test]
    fn symbols_are_distinct_and_readings_name_their_method() {
        let all = TimeOperation::all();
        let symbols: std::collections::HashSet<String> = all.iter().map(|op| op.symbol()).collect();
        assert_eq!(symbols.len(), all.len());
        assert_eq!(all.len(), 46, "a new operation needs a runtime entry point");
        assert_eq!(TimeOperation::DurationWhole(TimeUnit::Seconds).symbol(), "corvid_duration_whole_seconds");
        assert_eq!(TimeOperation::ZonedField(ZonedField::OffsetSeconds).symbol(), "corvid_zoned_time_offset_seconds");
    }

    #[test]
    fn operators_follow_the_checker_table() {
        use Type::*;
        let call = |op, l: Type, r: Type| time_binary(op, &l, &r);
        assert_eq!(call(BinaryOp::Mul, Int, Duration).map(|b| (b.operation, b.swapped)), Some((TimeOperation::DurationMul, true)));
        assert_eq!(call(BinaryOp::Add, Duration, ZonedTime).map(|b| b.operation), Some(TimeOperation::ZonedAddDuration));
        assert_eq!(call(BinaryOp::Lt, Instant, Instant).map(|b| b.reading), Some(BinaryReading::Order));
        assert_eq!(call(BinaryOp::NotEq, ZonedTime, ZonedTime).map(|b| b.reading), Some(BinaryReading::Equality));
        assert_eq!(call(BinaryOp::Lt, ZonedTime, ZonedTime), None);
        assert_eq!(call(BinaryOp::Add, Duration, Int), None);
        assert_eq!(call(BinaryOp::Add, Tainted(Box::new(Duration)), Duration).map(|b| b.operation), Some(TimeOperation::DurationAdd));
        assert!(has_time_operand(&Int, &Instant));
        assert!(!has_time_operand(&Int, &Decimal));
    }
}
