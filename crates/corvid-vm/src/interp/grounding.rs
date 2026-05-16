use crate::value::Value;
use corvid_ir::{IrPrompt, IrTool};

/// True when a tool's declared effect row produces grounded values.
/// Reads the IR's pre-computed `produces_grounded` bit, which the
/// IR lowering sets from `corvid_types::effects::effect_row_is_grounded`
/// — the single source of truth (literal `retrieval` effect or any
/// effect whose `data` dimension resolves to `grounded`).
pub(super) fn tool_has_retrieval_effect(tool: &IrTool) -> bool {
    tool.produces_grounded
}

pub(super) fn maybe_ground_tool_result(tool: &IrTool, callee_name: &str, value: Value) -> Value {
    if !tool.produces_grounded {
        return value;
    }
    if matches!(value, Value::Grounded(_)) {
        return value;
    }

    let chain = crate::ProvenanceChain::with_retrieval(callee_name, corvid_runtime::now_ms());
    Value::Grounded(crate::value::GroundedValue::new(value, chain))
}

/// Wrap a prompt's non-stream result in `Value::Grounded` when the
/// prompt's effect row carries `data: grounded`. Mirror of
/// `maybe_ground_tool_result` for prompts — without this, the
/// typechecker promises `Grounded<T>` for a `data: grounded` prompt
/// (Design X, slice 2b) but the runtime delivers a plain `T`, and
/// the slice-7b `UnwrapGrounded` IR node fails the strip at runtime.
pub(super) fn maybe_ground_prompt_result(
    prompt: &IrPrompt,
    callee_name: &str,
    value: Value,
) -> Value {
    if !prompt.produces_grounded {
        return value;
    }
    if matches!(value, Value::Grounded(_)) {
        return value;
    }

    let chain = crate::ProvenanceChain::with_retrieval(callee_name, corvid_runtime::now_ms());
    Value::Grounded(crate::value::GroundedValue::new(value, chain))
}
