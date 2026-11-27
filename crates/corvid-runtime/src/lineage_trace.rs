//! Lineage derived from a run's trace (lineage from real runs, L1).
//!
//! The trace a run writes (`<run_id>.jsonl`) is the one record of the
//! run: replay reads it. Lineage is a projection of that record, computed
//! here and nowhere else, so `corvid observe`, `corvid trace lineage`,
//! `corvid eval promote` and the OTLP export all see the same tree, and
//! that tree cannot drift from what replay uses.
//!
//! Each run becomes one tree, whose trace id is the run id:
//! - the root is the run (`Agent`, named after the agent it started);
//! - a call and its result become one span: `Db` for a table operation
//!   (`table:<Table>.<op>`, named `<Table>.<op>`), `Tool` for any other
//!   tool, `Prompt` for a model call, `Approval` for an approval. A call
//!   whose result is a recorded failure is `Failed`; a typed conflict is
//!   an answer, so it is `Ok`;
//! - an HTTP exchange is a child of the tool span it belongs to, and a
//!   retried attempt is a `Retry` span;
//! - a run that ended in an error has a failed root and an `Error` child.
//!
//! A call is paired with the next result of the same name in the same
//! run. A call whose result never arrives (a trace cut off mid-call) is
//! kept, with status `Failed`, never dropped.

use crate::lineage::{LineageEvent, LineageKind, LineageStatus};
use corvid_trace_schema::TraceEvent;
use std::collections::{BTreeMap, HashMap, VecDeque};

/// The prefix of a recorded table operation's tool name.
const TABLE_TOOL_PREFIX: &str = "table:";

/// The lineage of every run in `events`, one tree per run, in the order
/// the runs start. Events of different runs may be interleaved.
pub fn lineage_from_trace(events: &[TraceEvent]) -> Vec<LineageEvent> {
    let mut runs: Vec<(String, Vec<&TraceEvent>)> = Vec::new();
    for event in events {
        let Some(run_id) = run_id_of(event) else { continue };
        match runs.iter_mut().find(|(id, _)| id == run_id) {
            Some((_, run)) => run.push(event),
            None => runs.push((run_id.to_string(), vec![event])),
        }
    }
    runs.into_iter().flat_map(|(run_id, events)| RunLineage::project(&run_id, &events)).collect()
}

fn run_id_of(event: &TraceEvent) -> Option<&str> {
    match event {
        TraceEvent::SchemaHeader { .. } => None,
        other => Some(other.run_id()),
    }
}

/// One run's tree, built in trace order. Spans are kept in the order they
/// start; an open span waits in a queue under its pairing key until its
/// result arrives.
struct RunLineage {
    spans: Vec<LineageEvent>,
    open: HashMap<String, VecDeque<usize>>,
    children: BTreeMap<String, u64>,
    last_ms: u64,
}

impl RunLineage {
    fn project(run_id: &str, events: &[&TraceEvent]) -> Vec<LineageEvent> {
        let first_ms = events.first().map(|e| e.ts_ms()).unwrap_or(0);
        let root_name = events
            .iter()
            .find_map(|e| match e {
                TraceEvent::RunStarted { agent, .. } => Some(agent.clone()),
                _ => None,
            })
            .unwrap_or_else(|| run_id.to_string());
        let mut root = LineageEvent::root(run_id, LineageKind::Agent, root_name, first_ms);
        root.replay_key = run_id.to_string();
        let mut run = Self { spans: vec![root], open: HashMap::new(), children: BTreeMap::new(), last_ms: first_ms };
        let mut completed = false;
        for event in events {
            run.last_ms = run.last_ms.max(event.ts_ms());
            match event {
                TraceEvent::RunStarted { ts_ms, .. } => {
                    run.spans[0].started_ms = *ts_ms;
                    run.spans[0].ended_ms = *ts_ms;
                }
                TraceEvent::ToolCall { ts_ms, tool, .. } => {
                    let (kind, name) = match tool.strip_prefix(TABLE_TOOL_PREFIX) {
                        Some(operation) => (LineageKind::Db, operation),
                        None => (LineageKind::Tool, tool.as_str()),
                    };
                    run.open_span(0, kind, name, *ts_ms, format!("tool:{tool}"));
                }
                TraceEvent::ToolResult { ts_ms, tool, result, .. } => {
                    // A call that failed records its failure as its result,
                    // in the form replay serves it back; the same reader
                    // decides here, so the two never disagree.
                    let status = if crate::replay::replayed_failure(result).is_some() {
                        LineageStatus::Failed
                    } else {
                        LineageStatus::Ok
                    };
                    run.close_span(&format!("tool:{tool}"), status, *ts_ms);
                }
                TraceEvent::LlmCall { ts_ms, prompt, model, .. } => {
                    let at = run.open_span(0, LineageKind::Prompt, prompt, *ts_ms, format!("prompt:{prompt}"));
                    if let Some(model) = model {
                        run.spans[at].model_id = model.clone();
                    }
                }
                TraceEvent::LlmResult { ts_ms, prompt, model, .. } => {
                    if let Some(at) = run.close_span(&format!("prompt:{prompt}"), LineageStatus::Ok, *ts_ms) {
                        if let Some(model) = model {
                            run.spans[at].model_id = model.clone();
                        }
                    }
                }
                TraceEvent::ApprovalRequest { ts_ms, label, .. } => {
                    run.open_span(0, LineageKind::Approval, label, *ts_ms, format!("approval:{label}"));
                }
                // An approval is decided once; whichever of the two
                // events comes first closes it, and the other finds
                // nothing open.
                TraceEvent::ApprovalDecision { ts_ms, site, accepted, .. } => {
                    run.close_span(&format!("approval:{site}"), approval_status(*accepted), *ts_ms);
                }
                TraceEvent::ApprovalResponse { ts_ms, label, approved, .. } => {
                    run.close_span(&format!("approval:{label}"), approval_status(*approved), *ts_ms);
                }
                TraceEvent::HttpExchange { ts_ms, operation, attempt, outcome, elapsed_ms, .. } => {
                    let parent = run.latest_open(&format!("tool:{operation}")).unwrap_or(0);
                    let kind = if *attempt > 1 { LineageKind::Retry } else { LineageKind::Tool };
                    let name = format!("{operation} attempt {attempt}");
                    let status = match outcome.as_str() {
                        "response" | "redirected" => LineageStatus::Ok,
                        _ => LineageStatus::Failed,
                    };
                    let at = run.open_span(parent, kind, &name, ts_ms.saturating_sub(*elapsed_ms), String::new());
                    run.finish(at, status, *ts_ms);
                }
                TraceEvent::RunCompleted { ts_ms, ok, error, .. } => {
                    completed = true;
                    let status = if *ok { LineageStatus::Ok } else { LineageStatus::Failed };
                    if let Some(error) = error.as_deref().filter(|_| !ok) {
                        let at = run.open_span(0, LineageKind::Error, error, *ts_ms, String::new());
                        run.finish(at, LineageStatus::Failed, *ts_ms);
                    }
                    run.fail_open_spans(*ts_ms);
                    run.finish(0, status, *ts_ms);
                }
                _ => {}
            }
        }
        if !completed {
            // The trace ends before the run did: the run and every call
            // still open failed to finish within the record.
            let last = run.last_ms;
            run.fail_open_spans(last);
            run.finish(0, LineageStatus::Failed, last);
        }
        run.spans
    }

    /// Start a span under `parent`, waiting under `key` for its result
    /// (an empty key never waits). Its id is stable: the parent's id, the
    /// kind, the name and the child's position under its parent.
    fn open_span(&mut self, parent: usize, kind: LineageKind, name: &str, started_ms: u64, key: String) -> usize {
        let parent_span = self.spans[parent].span_id.clone();
        let ordinal = self.children.entry(parent_span).or_insert(0);
        let span = LineageEvent::child(&self.spans[parent], kind, name, *ordinal, started_ms);
        *ordinal += 1;
        self.spans.push(span);
        let at = self.spans.len() - 1;
        if !key.is_empty() {
            self.open.entry(key).or_default().push_back(at);
        }
        at
    }

    /// Finish the earliest open span waiting under `key`.
    fn close_span(&mut self, key: &str, status: LineageStatus, ended_ms: u64) -> Option<usize> {
        let at = self.open.get_mut(key)?.pop_front()?;
        self.finish(at, status, ended_ms);
        Some(at)
    }

    fn latest_open(&self, key: &str) -> Option<usize> {
        self.open.get(key)?.back().copied()
    }

    fn finish(&mut self, at: usize, status: LineageStatus, ended_ms: u64) {
        let ended_ms = ended_ms.max(self.spans[at].started_ms);
        let span = std::mem::replace(&mut self.spans[at], LineageEvent::root("", LineageKind::Agent, "", 0));
        self.spans[at] = span.finish(status, ended_ms);
    }

    fn fail_open_spans(&mut self, ended_ms: u64) {
        let open: Vec<usize> = self.open.drain().flat_map(|(_, spans)| spans).collect();
        for at in open {
            self.finish(at, LineageStatus::Failed, ended_ms);
        }
    }
}

fn approval_status(approved: bool) -> LineageStatus {
    if approved { LineageStatus::Ok } else { LineageStatus::Denied }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lineage::validate_lineage;
    use serde_json::json;

    const RUN: &str = "run-1";

    fn started(ts_ms: u64) -> TraceEvent {
        TraceEvent::RunStarted { ts_ms, run_id: RUN.into(), agent: "main".into(), args: vec![] }
    }

    fn completed(ts_ms: u64, error: Option<&str>) -> TraceEvent {
        TraceEvent::RunCompleted {
            ts_ms,
            run_id: RUN.into(),
            ok: error.is_none(),
            result: None,
            error: error.map(str::to_string),
        }
    }

    fn tool_call(ts_ms: u64, tool: &str) -> TraceEvent {
        TraceEvent::ToolCall { ts_ms, run_id: RUN.into(), tool: tool.into(), args: vec![] }
    }

    fn tool_result(ts_ms: u64, tool: &str) -> TraceEvent {
        TraceEvent::ToolResult { ts_ms, run_id: RUN.into(), tool: tool.into(), result: json!(null) }
    }

    fn shape(lineage: &[LineageEvent]) -> Vec<(LineageKind, &str, LineageStatus, u64, u64)> {
        lineage.iter().map(|e| (e.kind, e.name.as_str(), e.status, e.started_ms, e.ended_ms)).collect()
    }

    #[test]
    fn a_run_is_the_root_and_table_operations_are_database_spans() {
        let lineage = lineage_from_trace(&[
            started(10),
            tool_call(11, "table:Shelf.insert"),
            tool_result(15, "table:Shelf.insert"),
            tool_call(16, "send_email"),
            tool_result(20, "send_email"),
            completed(21, None),
        ]);
        assert_eq!(
            shape(&lineage),
            vec![
                (LineageKind::Agent, "main", LineageStatus::Ok, 10, 21),
                (LineageKind::Db, "Shelf.insert", LineageStatus::Ok, 11, 15),
                (LineageKind::Tool, "send_email", LineageStatus::Ok, 16, 20),
            ]
        );
        assert!(validate_lineage(&lineage).complete, "{:?}", validate_lineage(&lineage).violations);
        assert_eq!(lineage[0].trace_id, RUN);
        assert_eq!(lineage[0].replay_key, RUN, "the root names the run to replay");
        assert!(lineage[1..].iter().all(|span| span.parent_span_id == lineage[0].span_id));
        assert_eq!(lineage[1].latency_ms, 4);
    }

    #[test]
    fn a_call_pairs_with_the_next_result_of_the_same_name() {
        let lineage = lineage_from_trace(&[
            started(0),
            tool_call(1, "lookup"),
            tool_call(2, "table:Shelf.get"),
            tool_call(3, "lookup"),
            tool_result(4, "lookup"),
            tool_result(5, "table:Shelf.get"),
            tool_result(6, "lookup"),
            completed(7, None),
        ]);
        let spans: Vec<_> = shape(&lineage)[1..].iter().map(|(_, name, _, start, end)| (*name, *start, *end)).collect();
        assert_eq!(spans, vec![("lookup", 1, 4), ("Shelf.get", 2, 5), ("lookup", 3, 6)]);
        let ids: std::collections::BTreeSet<_> = lineage.iter().map(|e| &e.span_id).collect();
        assert_eq!(ids.len(), lineage.len(), "two calls of one tool get distinct span ids");
    }

    #[test]
    fn prompts_carry_their_model_and_approvals_their_decision() {
        let lineage = lineage_from_trace(&[
            started(0),
            TraceEvent::LlmCall {
                ts_ms: 1,
                run_id: RUN.into(),
                prompt: "draft".into(),
                model: Some("claude-sonnet-5-5".into()),
                model_version: None,
                rendered: None,
                args: vec![],
                sampling: None,
            },
            TraceEvent::LlmResult {
                ts_ms: 9,
                run_id: RUN.into(),
                prompt: "draft".into(),
                model: Some("claude-sonnet-5-5".into()),
                model_version: None,
                result: json!("text"),
                chunk_boundaries: None,
            },
            TraceEvent::ApprovalRequest { ts_ms: 10, run_id: RUN.into(), label: "SendEmail".into(), args: vec![] },
            TraceEvent::ApprovalResponse { ts_ms: 12, run_id: RUN.into(), label: "SendEmail".into(), approved: false },
            TraceEvent::ApprovalDecision {
                ts_ms: 12,
                run_id: RUN.into(),
                site: "SendEmail".into(),
                args: vec![],
                accepted: false,
                decider: "stdin".into(),
                rationale: None,
            },
            completed(13, None),
        ]);
        assert_eq!(
            shape(&lineage)[1..],
            [
                (LineageKind::Prompt, "draft", LineageStatus::Ok, 1, 9),
                (LineageKind::Approval, "SendEmail", LineageStatus::Denied, 10, 12),
            ]
        );
        assert_eq!(lineage[1].model_id, "claude-sonnet-5-5");
    }

    #[test]
    fn http_exchanges_sit_under_their_tool_and_retries_are_marked() {
        let exchange = |ts_ms, attempt, outcome: &str| TraceEvent::HttpExchange {
            ts_ms,
            run_id: RUN.into(),
            connector: Some("billing".into()),
            operation: "charge".into(),
            attempt,
            hop: 0,
            method: "POST".into(),
            url: "https://billing.example/charge".into(),
            request_headers: vec![],
            request_body_sha256: None,
            request_body_bytes: 0,
            status: None,
            response_headers: vec![],
            response_body_bytes: 0,
            elapsed_ms: 3,
            outcome: outcome.into(),
            detail: None,
        };
        let lineage = lineage_from_trace(&[
            started(0),
            tool_call(1, "charge"),
            exchange(5, 1, "timeout"),
            exchange(9, 2, "response"),
            tool_result(10, "charge"),
            completed(11, None),
        ]);
        assert_eq!(
            shape(&lineage)[1..],
            [
                (LineageKind::Tool, "charge", LineageStatus::Ok, 1, 10),
                (LineageKind::Tool, "charge attempt 1", LineageStatus::Failed, 2, 5),
                (LineageKind::Retry, "charge attempt 2", LineageStatus::Ok, 6, 9),
            ]
        );
        assert_eq!(lineage[2].parent_span_id, lineage[1].span_id);
        assert_eq!(lineage[3].parent_span_id, lineage[1].span_id);
        assert!(validate_lineage(&lineage).complete);
    }

    #[test]
    fn a_failed_run_has_an_error_span_and_an_unfinished_call_fails() {
        let lineage = lineage_from_trace(&[
            started(0),
            tool_call(1, "table:Shelf.get"),
            completed(4, Some("the database is unreachable")),
        ]);
        assert_eq!(
            shape(&lineage),
            vec![
                (LineageKind::Agent, "main", LineageStatus::Failed, 0, 4),
                (LineageKind::Db, "Shelf.get", LineageStatus::Failed, 1, 4),
                (LineageKind::Error, "the database is unreachable", LineageStatus::Failed, 4, 4),
            ]
        );
    }

    #[test]
    fn a_call_that_failed_is_a_failed_span_and_a_typed_conflict_is_not() {
        let failure = crate::replay::recorded_failure(&crate::errors::RuntimeError::Other("the database is locked".into()));
        let lineage = lineage_from_trace(&[
            started(0),
            tool_call(1, "table:Shelf.save"),
            TraceEvent::ToolResult { ts_ms: 2, run_id: RUN.into(), tool: "table:Shelf.save".into(), result: failure },
            tool_call(3, "table:Shelf.insert"),
            TraceEvent::ToolResult {
                ts_ms: 4,
                run_id: RUN.into(),
                tool: "table:Shelf.insert".into(),
                result: json!({"conflict": "Taken", "row": [1, "Reading", 1]}),
            },
            completed(5, None),
        ]);
        let statuses: Vec<_> = lineage[1..].iter().map(|span| span.status).collect();
        assert_eq!(statuses, vec![LineageStatus::Failed, LineageStatus::Ok], "a conflict is an answer, not a failure");
    }

    #[test]
    fn a_trace_cut_off_mid_run_keeps_what_it_holds_and_fails_the_rest() {
        let lineage = lineage_from_trace(&[started(0), tool_call(2, "lookup"), tool_result(3, "lookup"), tool_call(5, "lookup")]);
        assert_eq!(
            shape(&lineage),
            vec![
                (LineageKind::Agent, "main", LineageStatus::Failed, 0, 5),
                (LineageKind::Tool, "lookup", LineageStatus::Ok, 2, 3),
                (LineageKind::Tool, "lookup", LineageStatus::Failed, 5, 5),
            ]
        );
    }

    #[test]
    fn the_same_trace_always_gives_the_same_span_ids() {
        let trace = [started(0), tool_call(1, "table:Shelf.get"), tool_result(2, "table:Shelf.get"), completed(3, None)];
        assert_eq!(lineage_from_trace(&trace), lineage_from_trace(&trace));
    }

    #[test]
    fn interleaved_runs_become_separate_trees() {
        let other = |event: TraceEvent| match event {
            TraceEvent::RunStarted { ts_ms, agent, args, .. } => TraceEvent::RunStarted { ts_ms, run_id: "run-2".into(), agent, args },
            TraceEvent::RunCompleted { ts_ms, ok, result, error, .. } => {
                TraceEvent::RunCompleted { ts_ms, run_id: "run-2".into(), ok, result, error }
            }
            other => other,
        };
        let lineage = lineage_from_trace(&[started(0), other(started(1)), completed(2, None), other(completed(3, None))]);
        let roots: Vec<_> = lineage.iter().filter(|e| e.parent_span_id.is_empty()).map(|e| e.trace_id.as_str()).collect();
        assert_eq!(roots, vec!["run-1", "run-2"]);
    }
}
