//! B06 — a trace records the tz database it ran with. Replaying a
//! trace recorded with another database (or with none, from before
//! schema v3) still replays, and each zone-dependent operation it runs
//! warns once, naming the operation and the zone. A trace that touches
//! no zone replays silently.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_trace_schema::{read_events_from_path, write_events_to_path, TraceEvent};

const ZONED: &str = r#"
agent main() -> String:
    saturday = match "2026-03-28T09:00".parse_local_time("Europe/Berlin"):
        Ok(z) -> z
        Err(e) -> Instant.from_epoch_milliseconds(0).in_zone("UTC")
    return saturday.add_days(1).to_string()
"#;

const PLAIN: &str = r#"
agent main() -> Int:
    return 6 * 7
"#;

/// Records `source`, rewrites the header's tz database version, and
/// replays. Returns the replayed value, the recorded one, and the
/// replay's zone-rules warnings.
async fn replay_with_tzdb(source: &str, tzdb: Option<&str>) -> (corvid_vm::Value, corvid_vm::Value, Vec<String>) {
    let ir = compile_to_ir(source).expect("source compiles");
    let trace_dir = tempfile::tempdir().unwrap();
    let record = Runtime::builder().trace_to(trace_dir.path()).build();
    let recorded = run_ir_with_runtime(&ir, Some("main"), vec![], &record).await.expect("recorded run");
    let trace = record.tracer().path().to_path_buf();
    drop(record);

    let mut events = read_events_from_path(&trace).unwrap();
    match &mut events[0] {
        TraceEvent::SchemaHeader { tzdb_version, .. } => {
            assert_eq!(tzdb_version.as_deref(), Some(corvid_time::TZDB_VERSION), "the recording names its database");
            *tzdb_version = tzdb.map(str::to_string);
        }
        other => panic!("expected a schema header, got {other:?}"),
    }
    write_events_to_path(&trace, &events).unwrap();

    let replay = Runtime::builder().replay_from(&trace).build();
    let replayed = run_ir_with_runtime(&ir, Some("main"), vec![], &replay).await.expect("replay runs");
    let warnings = replay.replay_zone_rules_warnings();
    (replayed, recorded, warnings)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_database_replays_without_warnings() {
    let (replayed, recorded, warnings) = replay_with_tzdb(ZONED, Some(corvid_time::TZDB_VERSION)).await;
    assert_eq!(replayed, recorded);
    assert!(warnings.is_empty(), "got {warnings:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn another_database_warns_at_each_zone_dependent_operation() {
    let (replayed, recorded, warnings) = replay_with_tzdb(ZONED, Some("2001a")).await;
    assert_eq!(replayed, recorded, "a warning never stops the replay");
    let names: Vec<_> = ["`parse_local_time` in Europe/Berlin", "`add_days` in Europe/Berlin", "`to_string` in Europe/Berlin"]
        .iter()
        .map(|needle| warnings.iter().any(|w| w.contains(needle) && w.contains("2001a")))
        .collect();
    assert_eq!(names, [true, true, true], "got {warnings:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trace_from_before_the_field_replays_and_says_the_version_is_unknown() {
    let (replayed, recorded, warnings) = replay_with_tzdb(ZONED, None).await;
    assert_eq!(replayed, recorded);
    assert!(warnings.iter().all(|w| w.contains("an unknown tz database")), "got {warnings:?}");
    assert!(!warnings.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trace_that_touches_no_zone_is_silent_whatever_its_database() {
    for tzdb in [Some("2001a"), None] {
        let (replayed, recorded, warnings) = replay_with_tzdb(PLAIN, tzdb).await;
        assert_eq!(replayed, recorded);
        assert!(warnings.is_empty(), "got {warnings:?}");
    }
}
