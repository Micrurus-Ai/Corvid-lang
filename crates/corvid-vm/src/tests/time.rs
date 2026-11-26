//! B06 `Duration` and `Instant` on the interpreter, run from Corvid
//! source. Text and range rules are tested in `corvid-time`; these
//! tests pin the language surface.

use super::*;

async fn text_of(src: &str) -> String {
    let ir = ir_of(src);
    match run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run") {
        Value::String(s) => s.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

async fn error_of(src: &str) -> String {
    let ir = ir_of(src);
    run_agent(&ir, "main", vec![], &empty_runtime()).await.expect_err("must trap").to_string()
}

/// Shared helpers every test program below starts with.
const PRELUDE: &str = "\
agent at(text: String) -> Instant:
    return match text.parse_instant():
        Ok(i) -> i
        Err(e) -> Instant.from_epoch_milliseconds(0)

agent span(text: String) -> Duration:
    zero = Instant.from_epoch_milliseconds(0)
    return match text.parse_duration():
        Ok(d) -> d
        Err(e) -> zero - zero

";

#[tokio::test]
async fn instants_and_durations_compute_exactly() {
    let src = format!(
        "{PRELUDE}agent main() -> String:
    start = at(\"2026-07-11T08:30:00Z\")
    step = span(\"PT1H30M\")
    finish = start + step * 3
    total = finish - start
    return finish.to_string() + \" \" + total.to_string() + \" \" + (total / 4).to_string() + \" \" + (-step).to_string() + \" \" + (start - step).to_string()
"
    );
    assert_eq!(
        text_of(&src).await,
        "2026-07-11T13:00:00Z PT4H30M PT1H7M30S -PT1H30M 2026-07-11T07:00:00Z"
    );
}

#[tokio::test]
async fn readings_truncate_toward_zero() {
    let src = format!(
        "{PRELUDE}agent main() -> String:
    d = span(\"PT1H59M59.999S\")
    back = -d
    return d.whole_hours().to_string() + \" \" + d.whole_minutes().to_string() + \" \" + d.whole_seconds().to_string() + \" \" + d.whole_milliseconds().to_string() + \" \" + back.whole_hours().to_string() + \" \" + back.abs().to_string()
"
    );
    assert_eq!(text_of(&src).await, "1 119 7199 7199999 -1 PT1H59M59.999S");
}

#[tokio::test]
async fn instants_order_by_the_timeline_not_the_offset() {
    let src = format!(
        "{PRELUDE}agent main() -> List<Bool>:
    berlin = at(\"2026-03-29T03:30:00+02:00\")
    utc = at(\"2026-03-29T01:30:00Z\")
    later = at(\"2026-03-29T01:30:00.000000001Z\")
    return [berlin == utc, utc < later, later > berlin, later - utc == span(\"PT0.000000001S\")]
"
    );
    let ir = ir_of(&src);
    let Value::List(items) = run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run") else {
        panic!("expected a list")
    };
    assert_eq!(items.iter_cloned(), vec![Value::Bool(true); 4]);
}

#[tokio::test]
async fn epoch_milliseconds_convert_both_ways() {
    let src = "\
agent main() -> String:
    stamp = Instant.from_epoch_milliseconds(1783758600012)
    before = Instant.from_epoch_milliseconds(-1)
    return stamp.to_string() + \" \" + stamp.epoch_milliseconds().to_string() + \" \" + before.to_string()
";
    assert_eq!(text_of(src).await, "2026-07-11T08:30:00.012Z 1783758600012 1969-12-31T23:59:59.999Z");
}

#[tokio::test]
async fn parse_errors_are_results_not_traps() {
    let src = "\
agent main() -> String:
    day = match \"P1D\".parse_duration():
        Ok(d) -> d.to_string()
        Err(e) -> e
    local = match \"2026-07-11T08:30:00\".parse_instant():
        Ok(i) -> i.to_string()
        Err(e) -> \"no offset\"
    return day + \"|\" + local
";
    assert_eq!(
        text_of(src).await,
        "`P1D` is not a duration: a day is not a fixed length of time; write the span in hours, such as PT24H for one day of 24 hours|no offset"
    );
}

#[tokio::test]
async fn leaving_the_instant_range_traps() {
    let src = format!(
        "{PRELUDE}agent main() -> Instant:
    last = at(\"9999-12-31T23:59:59Z\")
    return last + span(\"PT1S\")
"
    );
    assert!(error_of(&src).await.contains("outside the range"));
    let src = "\
agent main() -> Instant:
    return Instant.from_epoch_milliseconds(9223372036854775807)
";
    assert!(error_of(src).await.contains("outside the range"));
}

#[tokio::test]
async fn dividing_a_duration_by_zero_traps() {
    let src = format!(
        "{PRELUDE}agent main(parts: Int) -> Duration:
    return span(\"PT1H\") / parts
"
    );
    let ir = ir_of(&src);
    let error = run_agent(&ir, "main", vec![Value::Int(0)], &empty_runtime()).await.expect_err("must trap");
    assert!(error.to_string().contains("division by zero"), "got {error}");
}
