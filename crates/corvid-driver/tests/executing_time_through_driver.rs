//! Slice 45m — end-to-end test that a real Corvid program
//! (compiled through the driver, run through the interpreter)
//! reaches the executing time + randomness dispatch, and that the
//! pure math methods compute correct values alongside them.
//!
//! Pinned semantics:
//! - `time_now_utc` returns a plausible instant whose `iso` field
//!   round-trips through `time_parse_iso`/`time_format_iso`.
//! - `time_parse_iso` returns Err (not a trap) on malformed input.
//! - `time_monotonic_ms` is non-decreasing across two reads.
//! - `random_float` is in [0, 1); `random_int(1, 6)` is in [1, 6].
//! - Math methods: checked `abs`/`pow`, `sqrt`, trapping
//!   `floor`/`ceil`/`round` with round = half away from zero.

use corvid_driver::{compile_to_ir_with_config_at_path, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;
use std::fs;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn time_math_random_end_to_end() {
    let project = tempfile::tempdir().expect("tempdir");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("repo root");
    fs::create_dir_all(project.path().join("src").join("std")).unwrap();
    for module in ["time.cor", "random.cor", "effects.cor"] {
        fs::copy(
            repo.join("std").join(module),
            project.path().join("src").join("std").join(module),
        )
        .unwrap();
    }

    let source = r#"
import "./std/time" use time_now_utc, time_monotonic_ms, time_parse_iso, time_format_iso
import "./std/random" use random_float, random_int

agent main() -> String:
    a = (-5).abs()
    b = 3.min(9)
    c = 2.pow(10)
    d = 2.0.sqrt()
    e = 3.7.floor()
    f = 3.2.ceil()
    g = (-2.5).round()
    ok_math = a == 5 and b == 3 and c == 1024 and e == 3 and f == 4 and g == -3
    ok_float = d > 1.41 and d < 1.42

    now = time_now_utc()
    rendered = time_format_iso(now.epoch_ms)
    reparsed = time_parse_iso(rendered).unwrap_or(-1)
    bad = time_parse_iso("not a date")
    ok_time = now.epoch_ms > 1700000000000 and reparsed == now.epoch_ms and bad.is_err()
    ok_iso = now.iso.contains("T") and rendered.ends_with("Z")

    t0 = time_monotonic_ms()
    t1 = time_monotonic_ms()
    ok_mono = t1 >= t0

    r = random_float()
    n = random_int(1, 6)
    ok_rand = r >= 0.0 and r < 1.0 and n >= 1 and n <= 6

    if ok_math and ok_float and ok_time and ok_iso and ok_mono and ok_rand:
        return "TIME MATH RANDOM WORK"
    return "MISMATCH"
"#;
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, source).unwrap();

    let ir = compile_to_ir_with_config_at_path(source, &main_path, None)
        .expect("45m e2e source must compile");
    let runtime = Runtime::builder().build();
    let out = run_ir_with_runtime(&ir, None, vec![], &runtime)
        .await
        .expect("45m e2e program must run");
    match out {
        Value::String(s) => assert_eq!(&*s, "TIME MATH RANDOM WORK"),
        other => panic!("expected String, got {other:?}"),
    }
}
