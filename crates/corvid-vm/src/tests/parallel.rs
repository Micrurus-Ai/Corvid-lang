//! `parallel:` block tests (slice 46e).
//!
//! The load-bearing proof: a concurrent run records an ARM-ORDERED
//! trace (buffers flush at the join in arm order, so the trace is
//! indistinguishable from sequential arm-order execution), and a
//! Substitute-mode replay of that trace reproduces the identical
//! result while consuming events with the ordinary sequential
//! cursor — zero schema changes, zero new matching rules.

use super::*;
use corvid_runtime::tracing::Tracer;
use corvid_runtime::MockAdapter;

const PARALLEL_SRC: &str = "\
public effect llm_call:
    cost: $0.01
    reversible: true

prompt ask_weather(city: String) -> String uses llm_call:
    \"Weather in {city}?\"

prompt ask_news(city: String) -> String uses llm_call:
    \"News in {city}?\"

agent main(city: String) -> String:
    parallel:
        weather = ask_weather(city)
        news = ask_news(city)
    return weather + \"|\" + news
";

fn mock() -> MockAdapter {
    MockAdapter::new("mock-1")
        .reply("ask_weather", serde_json::json!("sunny"))
        .reply("ask_news", serde_json::json!("quiet"))
}

#[tokio::test]
async fn parallel_arms_join_and_bind() {
    let ir = ir_of(PARALLEL_SRC);
    let rt = Runtime::builder()
        .llm(Arc::new(mock()))
        .default_model("mock-1")
        .build();
    let out = run_agent(&ir, "main", vec![Value::String(Arc::from("Nairobi"))], &rt)
        .await
        .expect("parallel run");
    let Value::String(s) = out else {
        panic!("expected String, got {out:?}");
    };
    assert_eq!(&*s, "sunny|quiet");
}

#[tokio::test]
async fn parallel_trace_is_arm_ordered_and_replays_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace_path = dir.path().join("parallel.jsonl");

    // 1. RECORD: concurrent arms, arm-ordered flush.
    let ir = ir_of(PARALLEL_SRC);
    {
        let tracer = Tracer::open_path(&trace_path, "r-parallel");
        let rt = Runtime::builder()
            .llm(Arc::new(mock()))
            .default_model("mock-1")
            .tracer(tracer)
            .build();
        let out = run_agent(&ir, "main", vec![Value::String(Arc::from("Nairobi"))], &rt)
            .await
            .expect("recorded run");
        let Value::String(s) = out else {
            panic!("expected String");
        };
        assert_eq!(&*s, "sunny|quiet");
    }

    // 2. The trace's llm events appear IN ARM ORDER regardless of
    //    completion order: ask_weather strictly before ask_news.
    let text = std::fs::read_to_string(&trace_path).expect("trace readable");
    let weather_pos = text.find("ask_weather").expect("weather event recorded");
    let news_pos = text.find("ask_news").expect("news event recorded");
    assert!(
        weather_pos < news_pos,
        "arm buffers must flush in arm order"
    );

    // 3. REPLAY: Substitute mode reproduces the identical result
    //    through the ordinary sequential cursor.
    let replay_rt = Runtime::builder()
        .llm(Arc::new(mock()))
        .default_model("mock-1")
        .replay_from(&trace_path)
        .build();
    let out = run_agent(
        &ir,
        "main",
        vec![Value::String(Arc::from("Nairobi"))],
        &replay_rt,
    )
    .await
    .expect("replayed run");
    let Value::String(s) = out else {
        panic!("expected String");
    };
    assert_eq!(&*s, "sunny|quiet", "replay must reproduce the join");
}
