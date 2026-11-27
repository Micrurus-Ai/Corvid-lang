//! Live subscriptions' side of the tables (B14 RT-2): which tables a
//! committed write changed, and the changes a subscriber receives.
//!
//! A write outside an `atomic:` block announces its table once it has
//! succeeded; a block announces every table it wrote when it commits,
//! and nothing when it rolls back, so a subscriber never sees a write
//! that was undone. A conflict writes nothing and announces nothing.
//!
//! A subscription keeps the rows it last sent. On a commit that touches
//! its table, the transport runs the `live` declaration's snapshot agent
//! again, which runs its query in the database, and [`live_changes`]
//! turns the two ordered results into the agreed wire changes. When the
//! commit names its rows, the agent reads only those rows under its
//! query's condition and `live_order` places them (live L2); the database
//! stays the one judge of which rows match.

use serde_json::Value as J;
use std::collections::{BTreeMap, BTreeSet};

/// The tables of one database that a committed write changed, and the
/// rows it wrote in each.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableCommit {
    pub database: String,
    /// Every table written. Empty means any table of the database may
    /// have changed: a listener that lost its connection says so.
    pub tables: BTreeSet<String>,
    /// For each table, the rows written and the revision each holds
    /// after the commit. A table may be missing here though it is in
    /// `tables` (a write whose key the caller did not give, or a
    /// notification from another process too large to carry its rows);
    /// a subscription then cannot name a removal's revision and sends a
    /// snapshot.
    pub rows: BTreeMap<String, WrittenRows>,
}

impl TableCommit {
    /// A commit of `tables` with no row detail.
    pub fn of_tables(database: impl Into<String>, tables: impl IntoIterator<Item = String>) -> Self {
        Self { database: database.into(), tables: tables.into_iter().collect(), rows: BTreeMap::new() }
    }
}

/// Rows written in one table, by key (the [`LiveRow::key`] text), with
/// the revision each holds after the write. An insert gives revision 1,
/// a save the row's new revision, and a delete the revision after the
/// row's last one, so every committed write moves a row's revision on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WrittenRows(BTreeMap<String, i64>);

impl WrittenRows {
    /// Note a write; a later write of the same row keeps the higher
    /// revision.
    pub fn note(&mut self, key: impl Into<String>, revision: i64) {
        let held = self.0.entry(key.into()).or_insert(revision);
        *held = (*held).max(revision);
    }

    /// Fold in the rows of a later commit, for a subscription that
    /// handles several commits at once.
    pub fn merge(&mut self, other: &WrittenRows) {
        for (key, revision) in &other.0 {
            self.note(key.clone(), *revision);
        }
    }

    /// The revision a row holds after the writes, if it was written.
    pub fn revision(&self, key: &str) -> Option<i64> {
        self.0.get(key).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, i64)> {
        self.0.iter().map(|(key, revision)| (key.as_str(), *revision))
    }
}

/// One row of a live result, as the wire carries it: the row in the
/// public JSON mapping, its key, and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveRow {
    /// The key's values as compact JSON text, which identifies the row.
    pub key: String,
    pub revision: i64,
    pub row: J,
}

impl LiveRow {
    /// A row from its public JSON object, keyed by the table's `keys`.
    /// `None` when the object lacks a key or a whole-number revision.
    pub fn from_json(row: J, keys: &[String]) -> Option<Self> {
        let object = row.as_object()?;
        let key = J::Array(keys.iter().map(|k| object.get(k).cloned()).collect::<Option<Vec<_>>>()?).to_string();
        let revision = object.get(corvid_ast::TableDecl::REVISION_FIELD)?.as_i64()?;
        Some(Self { key, revision, row })
    }
}

/// What a subscription receives after a commit.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveMessage {
    /// Nothing it can see changed.
    Unchanged,
    /// Changes to apply in order, each leaving the client's list equal
    /// to a prefix-consistent step toward the new result.
    Changes(Vec<LiveChange>),
    /// The whole result again, when it is shorter than the changes
    /// that would describe it.
    Snapshot(Vec<LiveRow>),
}

/// One change, as the `change` wire message carries it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveChange {
    pub kind: LiveChangeKind,
    /// The revision the change made: an added or updated row's revision,
    /// or for a removal the revision of the write that removed it.
    pub revision: i64,
    /// The row's zero-based place in the ordered result after this
    /// change; absent for a removal.
    pub index: Option<usize>,
    /// For a removal, the row as the subscriber last saw it.
    pub row: J,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveChangeKind {
    Added,
    Updated,
    Removed,
}

impl LiveChangeKind {
    /// The `kind` the wire message names it by.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Updated => "updated",
            Self::Removed => "removed",
        }
    }
}

/// The changes that take a subscriber from `previous` to `next`, both
/// in the query's order, after the commits that wrote `written` (the
/// live's table's rows from those commits, merged). Removals come first;
/// then, walking `next` in order, each row not already in place is added
/// or updated at its index. A client that applies them in order holds
/// `next` exactly. When more changes than `next` has rows would be
/// needed, a snapshot is the smaller message and is sent instead.
///
/// A removal carries the revision of the write that removed the row,
/// which only `written` knows: a row saved out of the query is absent
/// from `next`. When a removed row is not in `written`, its revision
/// cannot be named, and a snapshot is sent rather than a guess.
pub fn live_changes(previous: &[LiveRow], next: &[LiveRow], written: &WrittenRows) -> LiveMessage {
    let next_keys: BTreeSet<&str> = next.iter().map(|row| row.key.as_str()).collect();
    let mut changes = Vec::new();
    // The client's list as the changes are applied.
    let mut working: Vec<&LiveRow> = Vec::with_capacity(previous.len());
    for row in previous {
        if next_keys.contains(row.key.as_str()) {
            working.push(row);
            continue;
        }
        let Some(revision) = written.revision(&row.key) else {
            return LiveMessage::Snapshot(next.to_vec());
        };
        changes.push(LiveChange { kind: LiveChangeKind::Removed, revision, index: None, row: row.row.clone() });
    }
    for (index, row) in next.iter().enumerate() {
        if working.get(index).is_some_and(|held| held.key == row.key && held.revision == row.revision) {
            continue;
        }
        let kind = match working.iter().position(|held| held.key == row.key) {
            Some(at) => {
                working.remove(at);
                LiveChangeKind::Updated
            }
            None => LiveChangeKind::Added,
        };
        working.insert(index, row);
        changes.push(LiveChange { kind, revision: row.revision, index: Some(index), row: row.row.clone() });
    }
    if changes.is_empty() {
        LiveMessage::Unchanged
    } else if changes.len() > next.len() {
        LiveMessage::Snapshot(next.to_vec())
    } else {
        LiveMessage::Changes(changes)
    }
}

/// Apply `changes` to `rows` as a client does; the tests hold
/// [`live_changes`] to it.
pub fn apply_live_changes(rows: &mut Vec<LiveRow>, changes: &[LiveChange], keys: &[String]) {
    for change in changes {
        let Some(changed) = LiveRow::from_json(change.row.clone(), keys) else { continue };
        if let Some(at) = rows.iter().position(|row| row.key == changed.key) {
            rows.remove(at);
        }
        if let Some(index) = change.index {
            rows.insert(index.min(rows.len()), changed);
        }
    }
}

/// What an open `atomic:` block has written, announced when it commits.
#[derive(Debug, Default)]
pub(crate) struct WrittenTables(std::sync::Mutex<Written>);

#[derive(Debug, Default)]
pub(crate) struct Written {
    pub(crate) tables: BTreeSet<String>,
    pub(crate) rows: BTreeMap<String, WrittenRows>,
}

impl Written {
    /// A write to `table`, with the row's key and revision when known.
    pub(crate) fn note(&mut self, table: &str, row: Option<(&str, i64)>) {
        self.tables.insert(table.to_string());
        if let Some((key, revision)) = row {
            self.rows.entry(table.to_string()).or_default().note(key, revision);
        }
    }
}

impl WrittenTables {
    pub(crate) fn note(&self, table: &str, row: Option<(&str, i64)>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).note(table, row);
    }

    pub(crate) fn take(&self) -> Written {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys() -> Vec<String> {
        vec!["card".to_string()]
    }

    fn row(card: i64, revision: i64, column: &str) -> LiveRow {
        LiveRow::from_json(json!({"card": card, "column": column, "revision": revision}), &keys()).expect("a row")
    }

    /// The commit that took `previous` to `next`: every row whose
    /// revision moved or that left, removed rows one revision on.
    fn written(previous: &[LiveRow], next: &[LiveRow]) -> WrittenRows {
        let mut written = WrittenRows::default();
        for row in next {
            if !previous.iter().any(|held| held.key == row.key && held.revision == row.revision) {
                written.note(row.key.clone(), row.revision);
            }
        }
        for row in previous {
            if !next.iter().any(|now| now.key == row.key) {
                written.note(row.key.clone(), row.revision + 1);
            }
        }
        written
    }

    /// Whatever changes [`live_changes`] sends, a client that applies
    /// them in order holds exactly the new result.
    #[track_caller]
    fn round_trip(previous: Vec<LiveRow>, next: Vec<LiveRow>) -> LiveMessage {
        let message = live_changes(&previous, &next, &written(&previous, &next));
        let mut client = previous.clone();
        match &message {
            LiveMessage::Unchanged => {}
            LiveMessage::Changes(changes) => apply_live_changes(&mut client, changes, &keys()),
            LiveMessage::Snapshot(rows) => client = rows.clone(),
        }
        assert_eq!(client, next, "the client must hold the new result after {message:?}");
        message
    }

    #[test]
    fn an_unchanged_result_sends_nothing() {
        assert_eq!(round_trip(vec![row(1, 1, "todo")], vec![row(1, 1, "todo")]), LiveMessage::Unchanged);
    }

    #[test]
    fn an_insert_update_and_delete_become_added_updated_and_removed() {
        let previous = vec![row(1, 1, "todo"), row(2, 1, "todo"), row(3, 1, "todo")];
        let next = vec![row(1, 2, "done"), row(3, 1, "todo"), row(4, 1, "todo")];
        let LiveMessage::Changes(changes) = round_trip(previous, next) else { panic!("changes") };
        let kinds: Vec<(&str, Option<usize>)> = changes.iter().map(|c| (c.kind.wire_name(), c.index)).collect();
        assert_eq!(kinds, [("removed", None), ("updated", Some(0)), ("added", Some(2))]);
        assert_eq!(changes[0].row["card"], 2, "a removal carries the row last seen");
        assert_eq!(changes[0].revision, 2, "and the revision of the write that removed it");
    }

    #[test]
    fn a_row_saved_out_of_the_query_is_removed_at_its_new_revision() {
        // Card 1 was saved twice and now fails the filter: the subscriber
        // last saw revision 1, the commits left it at revision 3.
        let previous = vec![row(1, 1, "todo"), row(2, 1, "todo")];
        let next = vec![row(2, 1, "todo")];
        let mut written = WrittenRows::default();
        written.note(row(1, 2, "doing").key, 2);
        written.note(row(1, 3, "done").key, 3);
        let LiveMessage::Changes(changes) = live_changes(&previous, &next, &written) else { panic!("changes") };
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].kind, changes[0].revision), (LiveChangeKind::Removed, 3));
        assert_eq!(changes[0].row["column"], "todo", "the row as the subscriber last saw it");
    }

    #[test]
    fn a_removal_whose_revision_is_unknown_sends_a_snapshot() {
        let previous = vec![row(1, 1, "todo"), row(2, 1, "todo")];
        let next = vec![row(2, 1, "todo")];
        assert_eq!(
            live_changes(&previous, &next, &WrittenRows::default()),
            LiveMessage::Snapshot(next.clone()),
            "no guessed revision: the snapshot is always right"
        );
    }

    #[test]
    fn merged_commits_keep_each_rows_latest_revision() {
        let mut first = WrittenRows::default();
        first.note("[1]", 2);
        first.note("[2]", 5);
        let mut second = WrittenRows::default();
        second.note("[1]", 4);
        second.note("[2]", 3);
        first.merge(&second);
        assert_eq!((first.revision("[1]"), first.revision("[2]"), first.revision("[3]")), (Some(4), Some(5), None));
    }

    #[test]
    fn a_row_that_moves_is_updated_at_its_new_place() {
        let previous = vec![row(1, 1, "a"), row(2, 1, "b"), row(3, 1, "c")];
        let next = vec![row(2, 1, "b"), row(3, 1, "c"), row(1, 2, "z")];
        round_trip(previous, next);
    }

    #[test]
    fn heavy_churn_sends_a_snapshot_instead() {
        let previous: Vec<LiveRow> = (1..=5).map(|n| row(n, 1, "old")).collect();
        let next = vec![row(9, 1, "new")];
        assert!(matches!(round_trip(previous, next), LiveMessage::Snapshot(_)));
    }

    #[test]
    fn every_reordering_of_a_small_result_round_trips() {
        let base: Vec<LiveRow> = (1..=4).map(|n| row(n, 1, "x")).collect();
        let orders = [[0, 1, 2, 3], [3, 2, 1, 0], [1, 0, 3, 2], [2, 3, 0, 1], [0, 2, 1, 3]];
        for order in orders {
            for drop in 0..=4 {
                // Odd rows were also saved again, so their revision moved on.
                let next: Vec<LiveRow> =
                    order.iter().filter(|&&i| i != drop).map(|&i| row(i as i64 + 1, 1 + (i as i64 % 2), "x")).collect();
                round_trip(base.clone(), next);
            }
        }
    }
}
