//! A map finds a key in constant time: building one of n keys takes time
//! in proportion to n. A timing measurement, ignored by default; run it with
//! `cargo test -p corvid-vm --release --test map_scaling -- --ignored --nocapture`.

use corvid_vm::value::MapValue;
use corvid_vm::Value;
use std::sync::Arc;

fn build(n: i64) -> std::time::Duration {
    let start = std::time::Instant::now();
    let map = MapValue::new(std::iter::empty());
    for i in 0..n {
        map.insert_or_update(Value::String(Arc::from(format!("key-{i}"))), Value::Int(i));
    }
    for i in 0..n {
        assert!(map.get_by_key(&Value::String(Arc::from(format!("key-{i}")))).is_some());
    }
    assert_eq!(map.len() as i64, n);
    start.elapsed()
}

#[test]
#[ignore = "timing measurement"]
fn building_a_map_scales_with_its_size() {
    let small = build(20_000);
    let large = build(80_000);
    eprintln!("20k keys: {small:?}, 80k keys: {large:?}, ratio {:.1}", large.as_secs_f64() / small.as_secs_f64());
    // Four times the keys: about four times the work when lookup is
    // constant time, sixteen when it is a scan.
    assert!(large.as_secs_f64() < small.as_secs_f64() * 8.0, "building grows faster than its size");
}
