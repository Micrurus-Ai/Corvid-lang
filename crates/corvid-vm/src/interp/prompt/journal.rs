use corvid_runtime::llm::LlmResponse;
use corvid_runtime::queue::{JobCheckpoint, JobCheckpointKind};
use corvid_runtime::runtime::DurableJobContext;
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Replay saved results in invocation order. A partial parallel-call batch
/// leaves its assistant message in each committed result, so the remaining
/// calls can finish before the next request to the provider.
pub(super) struct ConversationJournal {
    job: DurableJobContext,
    label: String,
    saved: VecDeque<JobCheckpoint>,
    next: u64,
    steps: Option<u64>,
    calls: Option<u64>,
}

impl ConversationJournal {
    pub(super) fn open(job: &DurableJobContext, fingerprint: String) -> Result<Self, String> {
        let label = job.conversation_label(fingerprint);
        let snapshot = job
            .queue
            .resume_state(&job.job_id)
            .map_err(|e| e.to_string())?;
        let steps = snapshot.remaining_steps();
        let calls = snapshot.remaining_tool_calls();
        Ok(Self {
            job: job.clone(),
            steps,
            calls,
            next: snapshot.next_sequence,
            saved: snapshot
                .checkpoints
                .into_iter()
                .filter(|c| c.label == label)
                .collect(),
            label,
        })
    }

    pub(super) fn response(&mut self) -> Result<Option<LlmResponse>, String> {
        let Some(step) = self.saved.front() else {
            self.check_allowance(false)?;
            return Ok(None);
        };
        let value = step
            .payload
            .get("response")
            .ok_or("missing checkpointed model response")?
            .clone();
        let usage =
            serde_json::from_value(step.payload["usage"].clone()).map_err(|e| e.to_string())?;
        if step.kind == JobCheckpointKind::AgentStep {
            if !value.is_string() || self.saved.len() != 1 {
                return Err("invalid final conversation checkpoint".into());
            }
            self.saved.pop_front();
        }
        Ok(Some(LlmResponse::new(value, usage)))
    }

    pub(super) fn tool_result(
        &mut self,
        response: &LlmResponse,
        id: &str,
    ) -> Result<Option<Value>, String> {
        let Some(step) = self.saved.front() else {
            self.check_allowance(true)?;
            return Ok(None);
        };
        if step.kind != JobCheckpointKind::ToolResult
            || step.payload["response"] != response.value
            || step.payload["result"]["tool_call_id"] != id
        {
            return Err("saved tool result does not match its model request".into());
        }
        Ok(Some(
            self.saved.pop_front().unwrap().payload["result"].clone(),
        ))
    }

    /// Check the whole outstanding batch before applying any of its functions.
    pub(super) fn check_batch(&self, response: &LlmResponse, count: usize) -> Result<(), String> {
        let committed = self
            .saved
            .iter()
            .take(count)
            .take_while(|step| {
                step.kind == JobCheckpointKind::ToolResult
                    && step.payload["response"] == response.value
            })
            .count();
        let needed = (count - committed) as u64;
        if needed == 0 {
            return Ok(());
        }
        if self.calls.is_some_and(|calls| calls < needed) {
            return Err("function batch exceeds remaining job tool-call allowance".into());
        }
        if self.steps.is_some_and(|steps| steps <= needed) {
            return Err("job needs a step for each function and one for the final answer".into());
        }
        Ok(())
    }

    pub(super) fn final_answer_only(&self) -> bool {
        self.calls == Some(0) || self.steps == Some(1)
    }

    fn check_allowance(&self, tool: bool) -> Result<(), String> {
        if self.steps == Some(0) {
            return Err("durable job step allowance exhausted".into());
        }
        if tool && self.steps == Some(1) {
            return Err("the final job step is reserved for the model answer".into());
        }
        if tool && self.calls == Some(0) {
            return Err("durable job tool-call allowance exhausted".into());
        }
        Ok(())
    }

    pub(super) fn commit(
        &mut self,
        response: &LlmResponse,
        result: Option<&Value>,
    ) -> Result<(), String> {
        let kind = if result.is_some() {
            JobCheckpointKind::ToolResult
        } else {
            JobCheckpointKind::AgentStep
        };
        self.job
            .queue
            .record_step_checkpoint(
                &self.job.job_id,
                kind,
                &self.label,
                json!({"response": response.value, "usage":response.usage, "result":result}),
                None,
                self.next,
            )
            .map_err(|e| e.to_string())?;
        self.next += 1;
        self.steps = self.steps.map(|n| n.saturating_sub(1));
        if result.is_some() {
            self.calls = self.calls.map(|n| n.saturating_sub(1));
        }
        Ok(())
    }
}
