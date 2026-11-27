use super::*;
use crate::llm::mock::MockAdapter;
use crate::llm::LlmRequest;
use serde_json::json;
use std::sync::Arc;
#[tokio::test]
async fn call_llm_uses_default_model_when_request_blank() {
    let r = super::tests::rt();
    let resp = r
        .call_llm(LlmRequest {
            prompt: "greet".into(),
            model: String::new(),
            rendered: "say hi".into(),
            args: vec![],
            output_schema: None,
        sampling: Default::default(),
            messages: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(resp.value, json!("hi"));
}

#[tokio::test]
async fn call_llm_fails_over_to_compatible_model_and_traces_provider_events() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("failover.jsonl");
    let r = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "failover-run"))
        .llm(Arc::new(MockAdapter::new("primary")))
        .llm(Arc::new(
            MockAdapter::new("fallback").reply("greet", json!("from fallback")),
        ))
        .default_model("primary")
        .model(
            RegisteredModel::new("primary")
                .provider("openai")
                .capability("standard")
                .output_format("strict_json")
                .privacy_tier("hosted")
                .jurisdiction("US")
                .structured_output(true)
                .cost_per_token_in(0.000002),
        )
        .model(
            RegisteredModel::new("fallback")
                .provider("anthropic")
                .capability("expert")
                .output_format("strict_json")
                .privacy_tier("hosted")
                .jurisdiction("US")
                .structured_output(true)
                .cost_per_token_in(0.000001),
        )
        .build();

    let resp = r
        .call_llm(LlmRequest {
            prompt: "greet".into(),
            model: String::new(),
            rendered: "say hi".into(),
            args: vec![],
            output_schema: None,
        sampling: Default::default(),
            messages: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(resp.value, json!("from fallback"));

    let health = r.provider_health();
    let primary = health
        .iter()
        .find(|entry| entry.adapter == "primary")
        .expect("primary health");
    assert_eq!(primary.consecutive_failures, 1);
    assert!(primary.degraded);

    let events = corvid_trace_schema::read_events_from_path(&trace_path).unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::HostEvent { name, payload, .. }
            if name == "llm.provider_degraded" && payload["model"] == "primary"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::HostEvent { name, payload, .. }
            if name == "llm.provider_failover"
                && payload["from_model"] == "primary"
                && payload["to_model"] == "fallback"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::LlmResult { model, result, .. }
            if model.as_deref() == Some("fallback") && result == &json!("from fallback")
    )));
}

#[tokio::test]
async fn call_llm_records_normalized_usage_by_provider() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("usage.jsonl");
    let r = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "usage-run"))
        .llm(Arc::new(MockAdapter::new("gpt").reply_with_usage(
            "greet",
            json!("hi"),
            crate::llm::TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 4,
                total_tokens: 0,
            },
        )))
        .default_model("gpt")
        .model(
            RegisteredModel::new("gpt")
                .provider("openai")
                .privacy_tier("hosted")
                .cost_per_token_in(0.01)
                .cost_per_token_out(0.02),
        )
        .build();

    let resp = r
        .call_llm(LlmRequest {
            prompt: "greet".into(),
            model: String::new(),
            rendered: "say hi".into(),
            args: vec![],
            output_schema: None,
        sampling: Default::default(),
            messages: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(resp.value, json!("hi"));

    let records = r.llm_usage_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider.as_deref(), Some("openai"));
    assert_eq!(records[0].adapter.as_deref(), Some("gpt"));
    assert_eq!(records[0].total_tokens, 14);
    assert!((records[0].cost_usd - 0.18).abs() < 1e-12);

    let totals = r.llm_usage_totals_by_provider();
    assert_eq!(totals["openai"].calls, 1);
    assert_eq!(totals["openai"].total_tokens, 14);
    assert!((totals["openai"].cost_usd - 0.18).abs() < 1e-12);

    let events = corvid_trace_schema::read_events_from_path(&trace_path).unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        TraceEvent::HostEvent { name, payload, .. }
            if name == "llm.usage"
                && payload["provider"] == "openai"
                && payload["total_tokens"] == 14
                && payload["currency"] == "USD"
    )));

    // Lineage from real runs (L1c): the run's derived lineage carries the
    // same usage the ledger holds, on the prompt's span.
    let lineage = crate::lineage_from_trace(&events);
    let prompt = lineage
        .iter()
        .find(|span| span.kind == crate::LineageKind::Prompt)
        .expect("the model call is a prompt span");
    assert_eq!(prompt.name, "greet");
    assert_eq!((prompt.tokens_in, prompt.tokens_out), (10, 4));
    assert!((prompt.cost_usd - records[0].cost_usd).abs() < 1e-12, "lineage cost matches the ledger");
    assert_eq!(prompt.model_id, "gpt");
}

/// Lineage from real runs (L1d): a streamed call records its usage when
/// the stream ends, through the same path as a whole call, so the ledger,
/// the trace and derived lineage count it. Before, a streamed call's
/// tokens and cost were dropped.
#[tokio::test]
async fn a_streamed_call_records_its_usage_when_the_stream_ends() {
    use futures::StreamExt;
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("stream-usage.jsonl");
    let r = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "stream-usage-run"))
        .llm(Arc::new(MockAdapter::new("gpt").reply_with_usage(
            "draft",
            json!("streamed text"),
            crate::llm::TokenUsage { prompt_tokens: 12, completion_tokens: 30, total_tokens: 0 },
        )))
        .default_model("gpt")
        .model(RegisteredModel::new("gpt").provider("openai").cost_per_token_in(0.01).cost_per_token_out(0.02))
        .build();

    let mut stream = r
        .call_llm_stream(LlmRequest {
            prompt: "draft".into(),
            model: String::new(),
            rendered: "write".into(),
            args: vec![],
            output_schema: None,
            sampling: Default::default(),
            messages: Vec::new(),
        })
        .await
        .expect("the stream starts");
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        text.push_str(&chunk.expect("a chunk").delta);
    }
    assert_eq!(text, "streamed text");

    let records = r.llm_usage_records();
    assert_eq!(records.len(), 1, "a streamed call is in the usage ledger");
    assert_eq!((records[0].prompt_tokens, records[0].completion_tokens, records[0].total_tokens), (12, 30, 42));
    assert!((records[0].cost_usd - 0.72).abs() < 1e-12, "priced like a whole call: {}", records[0].cost_usd);
    assert_eq!(records[0].adapter.as_deref(), Some("gpt"));

    drop(r);
    let events = corvid_trace_schema::read_events_from_path(&trace_path).unwrap();
    let usage_at = events
        .iter()
        .position(|e| matches!(e, TraceEvent::HostEvent { name, .. } if name == "llm.usage"))
        .expect("the trace records the streamed call's usage");
    let result_at = events.iter().position(|e| matches!(e, TraceEvent::LlmResult { .. })).expect("result");
    assert!(usage_at < result_at, "usage comes just before the result, as for a whole call");
    let lineage = crate::lineage_from_trace(&events);
    let prompt = lineage.iter().find(|s| s.kind == crate::LineageKind::Prompt).expect("prompt span");
    assert_eq!((prompt.tokens_in, prompt.tokens_out), (12, 30));
    assert!((prompt.cost_usd - 0.72).abs() < 1e-12);
}

#[tokio::test]
async fn observation_summary_aggregates_usage_and_provider_health() {
    let dir = tempfile::tempdir().unwrap();
    let trace_path = dir.path().join("observe.jsonl");
    let r = Runtime::builder()
        .tracer(Tracer::open_path(&trace_path, "observe-run"))
        .llm(Arc::new(MockAdapter::new("gpt").reply_with_usage(
            "summarize",
            json!("ok"),
            crate::llm::TokenUsage {
                prompt_tokens: 8,
                completion_tokens: 4,
                total_tokens: 12,
            },
        )))
        .model(
            RegisteredModel::new("gpt")
                .provider("openai")
                .privacy_tier("hosted")
                .cost_per_token_in(0.001)
                .cost_per_token_out(0.002),
        )
        .build();

    r.call_llm(LlmRequest {
        prompt: "summarize".into(),
        model: "gpt".into(),
        rendered: "Summarize.".into(),
        args: vec![],
        output_schema: None,
        sampling: Default::default(),
            messages: Vec::new(),
    })
    .await
    .unwrap();

    let summary = r.emit_observation_summary();
    assert_eq!(summary.llm_calls, 1);
    assert_eq!(summary.local_llm_calls, 0);
    assert_eq!(summary.total_tokens, 12);
    assert_eq!(summary.cost_usd, 0.016);
    assert_eq!(summary.provider_count, 1);
    assert_eq!(summary.degraded_provider_count, 0);

    let events = corvid_trace_schema::read_events_from_path(&trace_path).unwrap();
    let event = events
        .iter()
        .find_map(|event| match event {
            TraceEvent::HostEvent { name, payload, .. } if name == "std.observe.summary" => {
                Some(payload)
            }
            _ => None,
        })
        .expect("std.observe summary event");
    assert_eq!(event["llm_calls"], json!(1));
    assert_eq!(event["total_tokens"], json!(12));
    assert_eq!(event["provider_count"], json!(1));
    assert_eq!(event["degraded_provider_count"], json!(0));
}
