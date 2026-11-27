use super::*;
use crate::llm::LlmRequestRef;


/// One structured LLM call, as the interpreter makes it: the rendered
/// prompt and the reply's public-form schema as structured output, no
/// text instructions. Returns the reply value the adapter gave (B1b), or
/// the runtime's error, which says whether it was an outage.
pub(super) fn call_llm_structured(
    state: &BridgeState,
    prompt_name: &str,
    model: &str,
    rendered: &str,
    args: &[serde_json::Value],
    schema: Option<&serde_json::Value>,
) -> Result<serde_json::Value, crate::RuntimeError> {
    let runtime = state.corvid_runtime();
    let req = LlmRequestRef {
        tool_turn: None,
        prompt: prompt_name,
        model,
        rendered,
        args,
        output_schema: schema,
        sampling: Default::default(),
        messages: &[],
    };
    let resp = state
        .tokio_handle()
        .block_on(async move { runtime.call_llm_ref_with_trace_rendered(req, Some(rendered)).await });
    match resp {
        Ok(r) => Ok(r.value),
        Err(e) => {
            panic_if_replay_runtime_error(
                &format!("corvid prompt `{prompt_name}` (model `{model}`) replay failed"),
                &e,
            );
            Err(e)
        }
    }
}

