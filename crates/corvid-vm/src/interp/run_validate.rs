use super::Interpreter;
use crate::env::Env;
use crate::errors::{InterpError, InterpErrorKind};
use crate::step::{StepController, StepEvent, StepMode};
use crate::value::{value_confidence, StructValue, Value};
use corvid_ast::Span;
use corvid_ir::IrFile;
use corvid_resolve::{DefId, LocalId};
use corvid_runtime::{Runtime, RuntimeError, TraceEvent};
use std::sync::Arc;
/// Public entry point: run `agent_name` with `args` against `runtime`.
///
/// The runtime owns tool/LLM/approval dispatch and tracing. Pass a
/// minimal runtime built via `Runtime::builder().build()` for tests
/// that don't exercise external calls.
pub async fn run_agent(
    ir: &IrFile,
    agent_name: &str,
    args: Vec<Value>,
    runtime: &Runtime,
) -> Result<Value, InterpError> {
    run_agent_with_env(ir, agent_name, args, runtime)
        .await
        .map(|(value, _env)| value)
}

pub async fn run_agent_with_env(
    ir: &IrFile,
    agent_name: &str,
    args: Vec<Value>,
    runtime: &Runtime,
) -> Result<(Value, Env), InterpError> {
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == agent_name)
        .ok_or_else(|| {
            InterpError::new(
                InterpErrorKind::DispatchFailed(format!("no agent named `{agent_name}`")),
                Span::new(0, 0),
            )
        })?;

    let json_args = recorded_run_args(ir, agent, &args)?;
    runtime
        .prepare_run(agent_name, &json_args)
        .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), Span::new(0, 0)))?;

    runtime.tracer().emit(TraceEvent::RunStarted {
        ts_ms: corvid_runtime::now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        agent: agent_name.to_string(),
        args: json_args,
    });

    let mut interp = Interpreter::new(ir, runtime);
    let bind_result = interp.bind_params(agent, args);
    let outcome = match bind_result {
        Ok(()) => interp
            .run_body(agent)
            .await
            .map(|value| (value, interp.env.clone())),
        Err(e) => Err(e),
    };

    let result_json = match &outcome {
        Ok((value, _env)) => Some(recorded_run_result(ir, agent, value)?),
        Err(_) => None,
    };
    let error_text = outcome.as_ref().err().map(|error| error.to_string());
    if should_validate_run_completion(&outcome) {
        runtime
            .complete_run(outcome.is_ok(), result_json.as_ref(), error_text.as_deref())
            .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), Span::new(0, 0)))?;
    }
    runtime.tracer().emit(TraceEvent::RunCompleted {
        ts_ms: corvid_runtime::now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        ok: outcome.is_ok(),
        result: result_json,
        error: error_text,
    });
    outcome
}

/// Run an agent with step-through control. The `hook` receives events at
/// tool/prompt/approval/agent-call boundaries (and optionally at every
/// statement) and decides whether to continue, override, or abort.
pub async fn run_agent_stepping(
    ir: &IrFile,
    agent_name: &str,
    args: Vec<Value>,
    runtime: &Runtime,
    hook: Arc<dyn crate::step::StepHook>,
    mode: StepMode,
) -> Result<(Value, Env), InterpError> {
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == agent_name)
        .ok_or_else(|| {
            InterpError::new(
                InterpErrorKind::DispatchFailed(format!("no agent named `{agent_name}`")),
                Span::new(0, 0),
            )
        })?;

    let json_args = recorded_run_args(ir, agent, &args)?;
    runtime
        .prepare_run(agent_name, &json_args)
        .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), Span::new(0, 0)))?;

    runtime.tracer().emit(TraceEvent::RunStarted {
        ts_ms: corvid_runtime::now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        agent: agent_name.to_string(),
        args: json_args,
    });

    let mut interp = Interpreter::new(ir, runtime);
    interp.stepper = Some(StepController::new(hook, mode));
    let bind_result = interp.bind_params(agent, args);
    let outcome = match bind_result {
        Ok(()) => interp
            .run_body(agent)
            .await
            .map(|value| (value, interp.env.clone())),
        Err(e) => Err(e),
    };

    let _ = interp
        .maybe_yield(StepEvent::Completed {
            agent_name: agent_name.to_string(),
            ok: outcome.is_ok(),
            result: outcome.as_ref().ok().map(|(v, _)| v.clone()),
            result_confidence: outcome.as_ref().ok().map(|(v, _)| value_confidence(v)),
            error: outcome.as_ref().err().map(|e| e.to_string()),
        })
        .await;

    let result_json = match &outcome {
        Ok((value, _)) => Some(recorded_run_result(ir, agent, value)?),
        Err(_) => None,
    };
    let error_text = outcome.as_ref().err().map(|error| error.to_string());
    if should_validate_run_completion(&outcome) {
        runtime
            .complete_run(outcome.is_ok(), result_json.as_ref(), error_text.as_deref())
            .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), Span::new(0, 0)))?;
    }
    runtime.tracer().emit(TraceEvent::RunCompleted {
        ts_ms: corvid_runtime::now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        ok: outcome.is_ok(),
        result: result_json,
        error: error_text,
    });
    outcome
}

/// Pre-bind specific locals and run an agent. Used by tests that want
/// to inject pre-built struct parameters bypassing the parameter list.
pub async fn bind_and_run_agent(
    ir: &IrFile,
    agent_name: &str,
    params_with_values: Vec<(LocalId, Value)>,
    fallback_args: Vec<Value>,
    runtime: &Runtime,
) -> Result<Value, InterpError> {
    if params_with_values.is_empty() {
        return run_agent(ir, agent_name, fallback_args, runtime).await;
    }
    let agent = ir
        .agents
        .iter()
        .find(|a| a.name == agent_name)
        .ok_or_else(|| {
            InterpError::new(
                InterpErrorKind::DispatchFailed(format!("no agent named `{agent_name}`")),
                Span::new(0, 0),
            )
        })?;
    let mut interp = Interpreter::new(ir, runtime);
    for (id, v) in params_with_values {
        if let Some(param) = agent.params.iter().find(|param| param.local_id == id) {
            Interpreter::check_grounded_argument(param, &v)?;
        }
        interp.env.bind(id, v);
    }
    interp.run_body(agent).await
}

/// Build a struct `Value` from field name → value pairs. Convenience used
/// by tests to construct struct arguments to inject into agent runs.
pub fn build_struct(
    type_id: DefId,
    type_name: &str,
    fields: impl IntoIterator<Item = (String, Value)>,
) -> Value {
    Value::Struct(StructValue::new(type_id, type_name.to_string(), fields))
}

fn should_validate_run_completion(outcome: &Result<(Value, Env), InterpError>) -> bool {
    !matches!(
        outcome,
        Err(InterpError {
            kind: InterpErrorKind::Runtime(
                RuntimeError::ReplayDivergence(_)
                    | RuntimeError::ReplayTraceLoad { .. }
            ),
            ..
        })
    )
}

/// A run's arguments as its trace records them (see
/// `tool_json::recorded_value_json`), what native code records.
fn recorded_run_args(ir: &IrFile, agent: &corvid_ir::IrAgent, args: &[Value]) -> Result<Vec<serde_json::Value>, InterpError> {
    let types: std::collections::HashMap<_, _> = ir.types.iter().map(|ty| (ty.id, ty)).collect();
    Ok(agent
        .params
        .iter()
        .zip(args)
        .map(|(param, value)| super::tool_json::recorded_value_json(value, &param.ty, &types))
        .collect())
}

/// A run's result as its trace records it, by the agent's declared
/// return type.
fn recorded_run_result(ir: &IrFile, agent: &corvid_ir::IrAgent, value: &Value) -> Result<serde_json::Value, InterpError> {
    let types: std::collections::HashMap<_, _> = ir.types.iter().map(|ty| (ty.id, ty)).collect();
    Ok(super::tool_json::recorded_value_json(value, &agent.return_ty, &types))
}
