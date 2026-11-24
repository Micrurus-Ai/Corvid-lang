//! B06 — warning when a replay's time zone rules may differ from the
//! recording's.
//!
//! Zone-dependent results (a local hour, a calendar move, reading a
//! local time) are pure functions of the IANA database compiled into
//! the binary, so replay recomputes them instead of reading them from
//! the trace. A trace recorded with another database version, or with
//! an unknown one (a trace from before schema v3), can therefore give a
//! different answer for a zone whose rules changed in between.
//!
//! Replay never refuses such a trace: most traces touch no zone at all.
//! Instead the interpreter reports each zone-dependent operation it
//! runs, and the first use of each operation in each zone produces one
//! warning that names the operation, the zone and both versions.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(crate) struct ReplayZoneRules {
    /// The recording's database version; `None` when the trace
    /// predates the field.
    recorded: Option<String>,
    warned: Arc<Mutex<BTreeSet<(String, String)>>>,
    warnings: Arc<Mutex<Vec<String>>>,
}

impl ReplayZoneRules {
    pub(crate) fn new(recorded: Option<String>) -> Self {
        Self { recorded, ..Self::default() }
    }

    /// Whether replaying zone-dependent work here may differ from the
    /// recording.
    fn may_differ(&self) -> bool {
        self.recorded.as_deref() != Some(corvid_time::TZDB_VERSION)
    }

    /// Reports one zone-dependent `operation` in `zone`. Returns the
    /// warning the first time this operation and zone may differ, and
    /// `None` otherwise.
    pub(crate) fn note_use(&self, operation: &str, zone: &str) -> Option<String> {
        if !self.may_differ() {
            return None;
        }
        let key = (operation.to_string(), zone.to_string());
        if !self.warned.lock().expect("zone warning lock").insert(key) {
            return None;
        }
        let current = corvid_time::TZDB_VERSION;
        let recorded = match &self.recorded {
            Some(version) => format!("tz database {version}"),
            None => "an unknown tz database (a trace from before schema v3)".to_string(),
        };
        let warning = format!(
            "warning: this trace was recorded with {recorded} and is replaying with tz database {current}; \
             `{operation}` in {zone} uses zone rules that may have changed between them, so its result can differ from the recording"
        );
        self.warnings.lock().expect("zone warning lock").push(warning.clone());
        Some(warning)
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.warnings.lock().expect("zone warning lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_version_never_warns() {
        let rules = ReplayZoneRules::new(Some(corvid_time::TZDB_VERSION.to_string()));
        assert_eq!(rules.note_use("add_days", "Europe/Berlin"), None);
        assert!(rules.warnings().is_empty());
    }

    #[test]
    fn another_version_warns_once_per_operation_and_zone() {
        let rules = ReplayZoneRules::new(Some("2001a".to_string()));
        let first = rules.note_use("add_days", "Europe/Berlin").expect("a differing version warns");
        assert!(first.contains("tz database 2001a"), "{first}");
        assert!(first.contains(corvid_time::TZDB_VERSION), "{first}");
        assert!(first.contains("`add_days` in Europe/Berlin"), "{first}");
        assert_eq!(rules.note_use("add_days", "Europe/Berlin"), None, "once per operation and zone");
        assert!(rules.note_use("add_days", "Asia/Tokyo").is_some());
        assert!(rules.note_use("hour", "Europe/Berlin").is_some());
        assert_eq!(rules.warnings().len(), 3);
    }

    #[test]
    fn an_unknown_version_warns_without_refusing() {
        let rules = ReplayZoneRules::new(None);
        let warning = rules.note_use("parse_local_time", "America/New_York").unwrap();
        assert!(warning.contains("an unknown tz database"), "{warning}");
    }
}
