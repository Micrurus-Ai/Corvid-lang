//! Exact columns for money and time on SQLite (B09 P8a).
//!
//! [`corvid_types::table_columns`] decides which kind of column each
//! type gets. This module holds the parts only the runtime can run: the
//! collations that make SQLite compare decimal and duration text by value,
//! and the text a zoned time is stored as.

use std::cmp::Ordering;

use corvid_decimal::Decimal;
use corvid_time::{Duration, Instant, TimeZone, ZonedTime};
use corvid_types::table_columns::{DECIMAL_COLLATION, DURATION_COLLATION};
use rusqlite::Connection;

/// Register the collations exact columns compare with. Every SQLite
/// connection Corvid opens calls this, because SQLite refuses to create,
/// compare or index such a column on a connection without them; it
/// never compares them some other way.
pub fn register_sqlite_collations(connection: &Connection) -> rusqlite::Result<()> {
    connection.create_collation(DECIMAL_COLLATION, |a, b| by_value(a, b, |text| Decimal::parse(text).ok()))?;
    connection.create_collation(DURATION_COLLATION, |a, b| by_value(a, b, |text| Duration::parse(text).ok()))
}

/// Compare two stored texts by the values they name. Corvid only stores
/// text that parses; a text written by another tool that does not parse
/// sorts after every value, then bytewise, so the order stays total.
fn by_value<T: Ord>(a: &str, b: &str, parse: impl Fn(&str) -> Option<T>) -> Ordering {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// The text a `ZonedTime` column stores: its instant's sortable text and
/// its zone, such as `2026-03-29T01:30:00.000000000Z[Europe/Berlin]`.
/// The offset is left out because it follows from the two, so an update
/// to the time-zone database never makes equal zoned times differ.
pub fn zoned_time_column_text(zoned: &ZonedTime) -> String {
    format!("{}[{}]", zoned.instant().to_sortable_text(), zoned.zone().name())
}

/// Read [`zoned_time_column_text`] back.
pub fn parse_zoned_time_column_text(text: &str) -> Result<ZonedTime, String> {
    let malformed = || format!("`{text}` is not a stored zoned time");
    let (instant, zone) = text.strip_suffix(']').and_then(|rest| rest.split_once('[')).ok_or_else(malformed)?;
    let instant = Instant::parse(instant).map_err(|_| malformed())?;
    let zone = TimeZone::parse(zone).map_err(|_| malformed())?;
    Ok(ZonedTime::new(instant, zone))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(connection: &Connection, collation: &str, values: &[&str]) -> Vec<String> {
        connection.execute_batch(&format!("CREATE TABLE t (v TEXT COLLATE {collation})")).unwrap();
        for value in values {
            connection.execute("INSERT INTO t VALUES (?1)", [value]).unwrap();
        }
        let mut statement = connection.prepare("SELECT v FROM t ORDER BY v").unwrap();
        let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(Result::unwrap).collect()
    }

    #[test]
    fn decimals_sort_and_compare_by_value() {
        let connection = Connection::open_in_memory().unwrap();
        register_sqlite_collations(&connection).unwrap();
        assert_eq!(sorted(&connection, DECIMAL_COLLATION, &["10.0", "9.50", "-2", "0.001", "-10.5"]), ["-10.5", "-2", "0.001", "9.50", "10.0"]);
        let equal: i64 = connection.query_row("SELECT count(*) FROM t WHERE v = '10.000'", [], |row| row.get(0)).unwrap();
        assert_eq!(equal, 1);
    }

    #[test]
    fn durations_sort_by_value() {
        let connection = Connection::open_in_memory().unwrap();
        register_sqlite_collations(&connection).unwrap();
        let texts = ["PT2H", "PT59M", "-PT1S", "PT0.5S"].map(|t| Duration::parse(t).unwrap().to_string());
        let ordered = sorted(&connection, DURATION_COLLATION, &texts.each_ref().map(String::as_str));
        let parsed: Vec<_> = ordered.iter().map(|t| Duration::parse(t).unwrap()).collect();
        assert!(parsed.windows(2).all(|pair| pair[0] < pair[1]), "{ordered:?}");
    }

    #[test]
    fn a_zoned_time_reads_back_as_the_same_instant_and_zone() {
        let zoned = ZonedTime::new(Instant::parse("2026-03-29T03:30:00+02:00").unwrap(), TimeZone::parse("Europe/Berlin").unwrap());
        let text = zoned_time_column_text(&zoned);
        assert_eq!(text, "2026-03-29T01:30:00.000000000Z[Europe/Berlin]");
        assert_eq!(parse_zoned_time_column_text(&text).unwrap(), zoned);
        assert!(parse_zoned_time_column_text("2026-03-29T01:30:00Z").is_err());
    }
}
