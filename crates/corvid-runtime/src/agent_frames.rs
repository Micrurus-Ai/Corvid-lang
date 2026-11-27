//! Which agent a traced step ran in (lineage from real runs, L1b).
//!
//! A run's trace records an agent call only when the agent did something
//! the trace records. In Corvid every function is an `agent`, so writing
//! every call would bury a run's steps under helpers that touch nothing.
//!
//! - **Entering an agent** pushes a frame that writes nothing.
//! - **The first traced event** written while frames are pending first
//!   writes an `AgentStarted` for each pending frame, outermost first. The
//!   check happens in [`crate::tracing::Tracer::emit`], so no list of
//!   "traced steps" has to be kept in step with the runtime.
//! - **Leaving an agent** writes `AgentFinished` only if its start was
//!   written.
//!
//! A pure helper therefore costs one push and one pop and never appears.
//! Frames live in a tokio task-local, set around each poll, so concurrent
//! work polled in one task keeps its own frame. Before a `parallel:`
//! block hands its arms buffered tracers, pending starts are written (see
//! [`crate::tracing::Tracer::buffered`]), so an arm's buffer never holds an
//! ancestor's start. Native code keeps no frames yet: its steps sit
//! directly under the run.

use crate::tracing::{now_ms, Tracer};
use corvid_trace_schema::TraceEvent;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Frame {
    agent: String,
    parent: Option<Arc<Frame>>,
    started: AtomicBool,
}

tokio::task_local! {
    static CURRENT: Arc<Frame>;
}

/// Run an agent's `body` as a frame. With tracing off this only awaits
/// `body`.
pub async fn within_agent<F, T, E>(tracer: &Tracer, agent: &str, body: F) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
{
    if !tracer.is_enabled() {
        return body.await;
    }
    let frame = Arc::new(Frame {
        agent: agent.to_string(),
        parent: CURRENT.try_with(Arc::clone).ok(),
        started: AtomicBool::new(false),
    });
    let outcome = CURRENT.scope(Arc::clone(&frame), body).await;
    if frame.started.load(Ordering::Acquire) {
        tracer.emit(TraceEvent::AgentFinished {
            ts_ms: now_ms(),
            run_id: tracer.run_id().to_string(),
            agent: frame.agent.clone(),
            ok: outcome.is_ok(),
        });
    }
    outcome
}

/// Write the `AgentStarted` of every pending frame around the current
/// task, outermost first, through `write`. Called by the tracer before
/// any other event, and before arms get buffered tracers.
pub(crate) fn write_pending_starts(run_id: &str, mut write: impl FnMut(TraceEvent)) {
    let Ok(innermost) = CURRENT.try_with(Arc::clone) else { return };
    let mut pending = Vec::new();
    let mut frame = Some(innermost);
    while let Some(current) = frame {
        if current.started.load(Ordering::Acquire) {
            break;
        }
        frame = current.parent.clone();
        pending.push(current);
    }
    for frame in pending.into_iter().rev() {
        if !frame.started.swap(true, Ordering::AcqRel) {
            write(TraceEvent::AgentStarted { ts_ms: now_ms(), run_id: run_id.to_string(), agent: frame.agent.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace_of(path: &std::path::Path) -> Vec<String> {
        corvid_trace_schema::read_events_from_path(path)
            .unwrap()
            .into_iter()
            .filter_map(|event| match event {
                TraceEvent::AgentStarted { agent, .. } => Some(format!("start {agent}")),
                TraceEvent::AgentFinished { agent, ok, .. } => Some(format!("end {agent} {ok}")),
                TraceEvent::HostEvent { name, .. } => Some(format!("step {name}")),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn only_agents_with_traced_steps_appear_nested_outermost_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frames.jsonl");
        let tracer = Tracer::open_path(&path, "run-frames");
        let step = |name: &str| TraceEvent::HostEvent {
            ts_ms: 0,
            run_id: "run-frames".into(),
            name: name.into(),
            payload: serde_json::Value::Null,
        };
        let outcome: Result<(), ()> = within_agent(&tracer, "outer", async {
            // A helper that records nothing leaves no trace.
            within_agent(&tracer, "pure_helper", async { Ok::<_, ()>(()) }).await?;
            within_agent(&tracer, "inner", async {
                tracer.emit(step("first"));
                tracer.emit(step("second"));
                Err::<(), ()>(())
            })
            .await
            .or(Ok(()))?;
            tracer.emit(step("after"));
            Ok(())
        })
        .await;
        assert!(outcome.is_ok());
        drop(tracer);
        assert_eq!(
            trace_of(&path),
            vec!["start outer", "start inner", "step first", "step second", "end inner false", "step after", "end outer true"]
        );
    }

    #[tokio::test]
    async fn with_tracing_off_a_frame_writes_nothing() {
        let tracer = Tracer::null();
        let value: Result<i32, ()> = within_agent(&tracer, "agent", async { Ok(7) }).await;
        assert_eq!(value, Ok(7));
    }
}
