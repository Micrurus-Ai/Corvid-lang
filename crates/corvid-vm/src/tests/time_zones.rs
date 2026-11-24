//! B06 `TimeZone` and `ZonedTime` on the interpreter, run from Corvid
//! source. The B06 acceptance test lives here: one calendar day across
//! Berlin's spring change is 23 hours, and 24 hours later is 10:00.

use super::*;

async fn text_of(src: &str) -> String {
    let ir = ir_of(src);
    match run_agent(&ir, "main", vec![], &empty_runtime()).await.expect("run") {
        Value::String(s) => s.to_string(),
        other => panic!("expected a String, got {other:?}"),
    }
}

const PRELUDE: &str = "\
agent local(text: String, place: TimeZone) -> ZonedTime:
    return match text.parse_local_time(place):
        Ok(z) -> z
        Err(e) -> Instant.from_epoch_milliseconds(0).in_zone(\"UTC\")

agent span(text: String) -> Duration:
    zero = Instant.from_epoch_milliseconds(0)
    return match text.parse_duration():
        Ok(d) -> d
        Err(e) -> zero - zero

";

#[tokio::test]
async fn a_calendar_day_across_berlins_spring_change() {
    let src = format!(
        "{PRELUDE}agent main() -> String:
    saturday = local(\"2026-03-28T09:00\", \"Europe/Berlin\")
    sunday = saturday.add_days(1)
    exact = saturday + span(\"PT24H\")
    return sunday.to_string() + \" \" + (sunday - saturday).to_string() + \" \" + exact.to_string()
"
    );
    assert_eq!(
        text_of(&src).await,
        "2026-03-29T09:00:00+02:00[Europe/Berlin] PT23H 2026-03-29T10:00:00+02:00[Europe/Berlin]"
    );
}

#[tokio::test]
async fn skipped_and_repeated_local_times_are_errors_naming_both_offsets() {
    let src = "\
agent main() -> String:
    skipped = match \"2026-03-29T02:30\".parse_local_time(\"Europe/Berlin\"):
        Ok(z) -> z.to_string()
        Err(e) -> e
    repeated = match \"2026-10-25T02:30\".parse_local_time(\"Europe/Berlin\"):
        Ok(z) -> z.to_string()
        Err(e) -> e
    return skipped + \"|\" + repeated
";
    assert_eq!(
        text_of(src).await,
        "2026-03-29T02:30 does not exist in Europe/Berlin: clocks move from +01:00 to +02:00 there, skipping it|2026-10-25T02:30 happens twice in Europe/Berlin: once at +02:00 and again at +01:00"
    );
}

#[tokio::test]
async fn local_fields_and_other_zones() {
    let src = format!(
        "{PRELUDE}agent main() -> String:
    berlin = local(\"2026-07-11T12:00\", \"Europe/Berlin\")
    tokyo = berlin.with_zone(\"Asia/Tokyo\")
    fields = berlin.year().to_string() + \"-\" + berlin.month().to_string() + \"-\" + berlin.day().to_string() + \" \" + berlin.weekday().to_string() + \" \" + berlin.offset_seconds().to_string()
    same_instant = tokyo.instant() == berlin.instant()
    return fields + \" \" + tokyo.to_string() + \" \" + tokyo.time_zone().to_string() + \" \" + same_instant.to_string() + \" \" + (tokyo == berlin).to_string()
"
    );
    assert_eq!(
        text_of(&src).await,
        "2026-7-11 6 7200 2026-07-11T19:00:00+09:00[Asia/Tokyo] Asia/Tokyo true false"
    );
}

#[tokio::test]
async fn months_clamp_and_zoned_text_round_trips() {
    let src = format!(
        "{PRELUDE}agent main() -> String:
    end_of_january = local(\"2028-01-31T08:00\", \"America/New_York\")
    february = end_of_january.add_months(1)
    round = match february.to_string().parse_zoned_time():
        Ok(z) -> z == february
        Err(e) -> false
    wrong = match \"2026-07-11T12:00:00+01:00[Europe/Berlin]\".parse_zoned_time():
        Ok(z) -> \"accepted\"
        Err(e) -> \"refused\"
    return february.to_string() + \" \" + round.to_string() + \" \" + wrong
"
    );
    assert_eq!(text_of(&src).await, "2028-02-29T08:00:00-05:00[America/New_York] true refused");
}

#[tokio::test]
async fn zone_names_from_run_time_go_through_a_result() {
    let src = "\
agent main(name: String) -> String:
    return match name.parse_time_zone():
        Ok(z) -> z.to_string()
        Err(e) -> e
";
    let ir = ir_of(src);
    let run = |name: &'static str| {
        let ir = ir.clone();
        async move { run_agent(&ir, "main", vec![Value::String(Arc::from(name))], &empty_runtime()).await.expect("run") }
    };
    assert_eq!(run("Australia/Sydney").await, Value::String(Arc::from("Australia/Sydney")));
    assert_eq!(
        run("Europe/Berln").await,
        Value::String(Arc::from("`Europe/Berln` is not a time zone in the IANA database; did you mean `Europe/Berlin`?"))
    );
}
