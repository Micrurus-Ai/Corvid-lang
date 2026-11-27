//! Placing re-checked rows in a live query's order (live L2).
//!
//! After a commit, a live subscription asks the database for only the
//! rows the commit wrote, under its query's own condition, so the database
//! still decides which rows match. Those rows then go into the previous
//! result at the places the full query would give them. [`query_sql`]
//! orders every query totally: by the `order_by` column, then by the
//! key's columns. [`RowOrder`] compares two rows the same way, and only for
//! column kinds whose order it can reproduce exactly on that engine: whole
//! numbers, `Bool`s, `Decimal`s and `Duration`s by value, `Instant`s in time
//! order, and text on SQLite, which compares text bytewise as Rust does.
//! Any other kind (text under a Postgres locale, zoned times, time zones,
//! currencies, locales, optional columns) gives no `RowOrder`, and the
//! caller re-runs the full query, which is always correct.
//!
//! [`query_sql`]: super::query_sql

use super::live::{LiveRow, WrittenRows};
use super::{ColumnKind, TableEngine};
use corvid_decimal::Decimal;
use corvid_time::{Duration, Instant};
use serde_json::Value as J;
use std::cmp::Ordering;

/// The most written rows a commit may name for a re-check of only those
/// rows; a larger commit re-runs the full query.
pub const MOST_ROWS_RECHECKED: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderedAs {
    WholeNumber,
    Boolean,
    Bytes,
    /// By value, as the `corvid_decimal` collation and Postgres `numeric`
    /// compare: `1.0` equals `1.00`, and the key breaks the tie.
    Decimal,
    /// By nanoseconds, as the `corvid_duration` collation and Postgres
    /// `numeric(39, 0)` compare.
    Duration,
    /// In time order, which is the byte order of the stored text on both
    /// engines (`COLLATE "C"` on Postgres).
    Instant,
}

/// How a query orders its rows: each field, how it compares, and whether
/// it is descending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowOrder {
    fields: Vec<(String, OrderedAs, bool)>,
}

/// A column a query orders by, as the table stores it.
#[derive(Debug, Clone, Copy)]
pub struct OrderColumn<'a> {
    pub name: &'a str,
    pub kind: ColumnKind,
    pub nullable: bool,
}

impl RowOrder {
    /// The order `query_sql` gives a query ordered by `order` (when it has
    /// an `order_by`), then by `keys`. `None` when a column's order on
    /// `engine` is one this does not reproduce exactly.
    pub fn of_query(
        engine: TableEngine,
        order: Option<(OrderColumn<'_>, bool)>,
        keys: &[OrderColumn<'_>],
    ) -> Option<Self> {
        let ordered_as = |column: &OrderColumn<'_>| {
            if column.nullable {
                // SQLite puts missing values first and Postgres last.
                return None;
            }
            match column.kind {
                ColumnKind::Integer => Some(OrderedAs::WholeNumber),
                ColumnKind::Boolean => Some(OrderedAs::Boolean),
                ColumnKind::Text
                    if engine == TableEngine::Sqlite && column.kind.sqlite_collation().is_none() =>
                {
                    Some(OrderedAs::Bytes)
                }
                ColumnKind::Decimal => Some(OrderedAs::Decimal),
                ColumnKind::Duration => Some(OrderedAs::Duration),
                ColumnKind::Instant => Some(OrderedAs::Instant),
                _ => None,
            }
        };
        let mut fields = Vec::new();
        if let Some((column, descending)) = &order {
            fields.push((column.name.to_string(), ordered_as(column)?, *descending));
        }
        for key in keys {
            if order.as_ref().is_some_and(|(column, _)| column.name == key.name) {
                continue;
            }
            fields.push((key.name.to_string(), ordered_as(key)?, false));
        }
        Some(Self { fields })
    }

    /// How `a` compares with `b`. `None` when a row lacks a field or holds
    /// a value of another kind.
    fn compare(&self, a: &J, b: &J) -> Option<Ordering> {
        for (name, ordered_as, descending) in &self.fields {
            let (a, b) = (a.get(name)?, b.get(name)?);
            let ordering = match ordered_as {
                OrderedAs::WholeNumber => a.as_i64()?.cmp(&b.as_i64()?),
                OrderedAs::Boolean => a.as_bool()?.cmp(&b.as_bool()?),
                OrderedAs::Bytes => a.as_str()?.as_bytes().cmp(b.as_str()?.as_bytes()),
                OrderedAs::Decimal => by_value(a, b, |text| Decimal::parse(text).ok())?,
                OrderedAs::Duration => by_value(a, b, |text| Duration::parse(text).ok())?,
                OrderedAs::Instant => by_value(a, b, |text| Instant::parse(text).ok())?,
            };
            let ordering = if *descending { ordering.reverse() } else { ordering };
            if ordering != Ordering::Equal {
                return Some(ordering);
            }
        }
        Some(Ordering::Equal)
    }
}

/// Compare two public JSON texts by the values they name. `None` when
/// either is not text of that type.
fn by_value<T: Ord>(a: &J, b: &J, parse: impl Fn(&str) -> Option<T>) -> Option<Ordering> {
    Some(parse(a.as_str()?)?.cmp(&parse(b.as_str()?)?))
}

/// The query's result after a commit, from its previous result and the
/// written rows that still match (`rechecked`, read under the query's
/// condition): every written row leaves its old place, and each one that
/// still matches goes where the order puts it. `None` when a row cannot be
/// placed, or the database returned a row the commit did not write; the
/// caller then re-runs the full query.
pub fn merge_rechecked(
    previous: &[LiveRow],
    rechecked: Vec<LiveRow>,
    written: &WrittenRows,
    order: &RowOrder,
) -> Option<Vec<LiveRow>> {
    let mut next: Vec<LiveRow> = previous
        .iter()
        .filter(|row| written.revision(&row.key).is_none())
        .cloned()
        .collect();
    for row in rechecked {
        written.revision(&row.key)?;
        let mut placed = true;
        let at = next.partition_point(|held| match order.compare(&held.row, &row.row) {
            Some(ordering) => ordering != Ordering::Greater,
            None => {
                placed = false;
                false
            }
        });
        if !placed || next.get(at).is_some_and(|held| held.key == row.key) {
            return None;
        }
        next.insert(at, row);
    }
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(card: i64, rank: i64, title: &str, revision: i64) -> LiveRow {
        LiveRow::from_json(
            json!({"card": card, "rank": rank, "title": title, "revision": revision}),
            &["card".to_string()],
        )
        .unwrap()
    }

    fn column(name: &str, kind: ColumnKind) -> OrderColumn<'_> {
        OrderColumn { name, kind, nullable: false }
    }

    fn written(rows: &[&LiveRow]) -> WrittenRows {
        let mut written = WrittenRows::default();
        for row in rows {
            written.note(row.key.clone(), row.revision);
        }
        written
    }

    #[test]
    fn rechecked_rows_take_the_places_the_full_query_gives_them() {
        let by_rank = RowOrder::of_query(
            TableEngine::Sqlite,
            Some((column("rank", ColumnKind::Integer), true)),
            &[column("card", ColumnKind::Integer)],
        )
        .unwrap();
        let previous = vec![card(1, 9, "a", 1), card(2, 5, "b", 1), card(3, 5, "c", 1)];
        // Card 1 drops to rank 5 (ties go by key), card 4 arrives at the
        // top, card 3 leaves the query.
        let moved = card(1, 5, "a", 2);
        let arrived = card(4, 10, "d", 1);
        let left = card(3, 5, "c", 2);
        let next = merge_rechecked(
            &previous,
            vec![moved.clone(), arrived.clone()],
            &written(&[&moved, &arrived, &left]),
            &by_rank,
        )
        .unwrap();
        assert_eq!(next, vec![arrived, moved, card(2, 5, "b", 1)]);
    }

    #[test]
    fn only_orders_it_reproduces_exactly_are_used() {
        let key = [column("card", ColumnKind::Integer)];
        let title = column("title", ColumnKind::Text);
        assert!(RowOrder::of_query(TableEngine::Sqlite, Some((title, false)), &key).is_some());
        let postgres = TableEngine::from_declaration("postgres", None, Some(4)).unwrap();
        assert!(RowOrder::of_query(postgres, Some((title, false)), &key).is_none());
        let optional = OrderColumn { nullable: true, ..column("rank", ColumnKind::Integer) };
        assert!(RowOrder::of_query(TableEngine::Sqlite, Some((optional, false)), &key).is_none());
        let zoned = column("signed", ColumnKind::ZonedTime);
        assert!(RowOrder::of_query(TableEngine::Sqlite, Some((zoned, false)), &key).is_none());
        for kind in [ColumnKind::Decimal, ColumnKind::Duration, ColumnKind::Instant] {
            assert!(RowOrder::of_query(postgres, Some((column("at", kind), false)), &key).is_some());
        }
    }

    #[test]
    fn decimals_durations_and_instants_are_placed_by_value() {
        let key = [column("id", ColumnKind::Integer)];
        let place = |kind, values: &[&str], arriving: &str| {
            let order = RowOrder::of_query(TableEngine::Sqlite, Some((column("v", kind), false)), &key).unwrap();
            let row = |id: i64, v: &str| LiveRow::from_json(json!({"id": id, "v": v, "revision": 1}), &["id".to_string()]).unwrap();
            let previous: Vec<LiveRow> = values.iter().enumerate().map(|(n, v)| row(n as i64, v)).collect();
            let new = row(99, arriving);
            let next = merge_rechecked(&previous, vec![new.clone()], &written(&[&new]), &order).unwrap();
            next.iter().position(|r| r.key == new.key).unwrap()
        };
        // `9.50` sorts before `10.0` by value, not text; `1.00` ties `1.0`
        // and follows it by key.
        assert_eq!(place(ColumnKind::Decimal, &["-2", "1.0", "10.0"], "9.50"), 2);
        assert_eq!(place(ColumnKind::Decimal, &["-2", "1.0", "10.0"], "1.00"), 2);
        assert_eq!(place(ColumnKind::Duration, &["PT1S", "PT1M", "PT1H"], "PT2M"), 2);
        assert_eq!(
            place(ColumnKind::Instant, &["2026-01-01T00:00:00Z", "2026-06-01T00:00:00Z"], "2026-03-01T12:00:00Z"),
            1
        );
    }

    #[test]
    fn a_row_the_commit_did_not_write_falls_back_to_the_full_query() {
        let order = RowOrder::of_query(TableEngine::Sqlite, None, &[column("card", ColumnKind::Integer)]).unwrap();
        let stray = card(7, 1, "x", 1);
        assert!(merge_rechecked(&[], vec![stray], &WrittenRows::default(), &order).is_none());
    }
}
