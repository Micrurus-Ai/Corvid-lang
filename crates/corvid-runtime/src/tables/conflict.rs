//! Why a table write did not happen (B09): the runtime's side of the
//! language's `Conflict<Row>`. The write decides the case where the
//! database outcome is known, so no caller tells a changed row from a
//! taken value by comparing keys.

use super::Row;
use corvid_types::ConflictCase;

#[derive(Debug, Clone, PartialEq)]
pub enum WriteConflict {
    /// The row changed since it was read; this is the row stored now.
    Changed(Row),
    /// The row is gone.
    Deleted,
    /// Another row holds the written row's key or one of its `unique`
    /// values; this is that row.
    Taken(Row),
}

impl WriteConflict {
    /// A revision check that failed: the row stored now, if any.
    pub fn stale(current: Option<Row>) -> Self {
        match current {
            Some(row) => Self::Changed(row),
            None => Self::Deleted,
        }
    }

    /// The conflict for `case` carrying `row`, or `None` when the row is
    /// missing for a case that has one or present for `Deleted`.
    pub fn new(case: ConflictCase, row: Option<Row>) -> Option<Self> {
        Some(match (case, row) {
            (ConflictCase::Changed, Some(row)) => Self::Changed(row),
            (ConflictCase::Deleted, None) => Self::Deleted,
            (ConflictCase::Taken, Some(row)) => Self::Taken(row),
            _ => return None,
        })
    }

    pub fn case(&self) -> ConflictCase {
        match self {
            Self::Changed(_) => ConflictCase::Changed,
            Self::Deleted => ConflictCase::Deleted,
            Self::Taken(_) => ConflictCase::Taken,
        }
    }

    /// The row the case carries: the current row or the holder.
    pub fn row(&self) -> Option<&Row> {
        match self {
            Self::Changed(row) | Self::Taken(row) => Some(row),
            Self::Deleted => None,
        }
    }

    pub fn into_row(self) -> Option<Row> {
        match self {
            Self::Changed(row) | Self::Taken(row) => Some(row),
            Self::Deleted => None,
        }
    }
}
