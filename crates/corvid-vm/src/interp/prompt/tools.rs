use super::{journal::ConversationJournal, Interpreter};
use crate::conv::{json_to_value, value_to_json};
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::Value;
use corvid_ast::Span;
use corvid_ir::IrPrompt;
use corvid_runtime::llm::{LlmRequest, LlmResponse, ToolTurn};
use corvid_types::Type;
use serde_json::{json, Map};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use corvid_ast::MAX_PROMPT_TOOL_CALLS;

impl<'ir> Interpreter<'ir> {
    pub(super) async fn call_with_functions(
        &mut self,
        prompt: &IrPrompt,
        values: &[Value],
        req: &LlmRequest,
        model: &str,
        span: Span,
    ) -> Result<LlmResponse, InterpError> {
        let error =
            |message: String| InterpError::new(InterpErrorKind::DispatchFailed(message), span);
        if prompt.return_ty != Type::String {
            return Err(error("a function-calling prompt must return String".into()));
        }
        let mut turn = ToolTurn::default();
        let mut functions = std::collections::HashMap::new();
        for (param, value) in prompt.params.iter().zip(values) {
            let Type::Function { params, .. } = &param.ty else {
                continue;
            };
            let Value::Closure(function) = value else {
                return Err(error(format!("expected a function for {}", param.name)));
            };
            let mut properties = Map::new();
            let mut required = Vec::new();
            for ((_, name), ty) in function.params().iter().zip(params) {
                properties.insert(
                    name.clone(),
                    crate::schema::schema_for(ty, &self.types_by_id),
                );
                required.push(name);
            }
            turn.definitions.push(json!({"type":"function", "function":{
                "name":param.name, "parameters":{"type":"object","properties":properties,
                "required":required,"additionalProperties":false}
            }}));
            functions.insert(param.name.as_str(), (function, params));
        }
        let mut journal = if let Some(job) = self.runtime.durable_job() {
            // A restored result cannot reproduce writes to a captured outer variable.
            if functions.values().any(|(f, _)| !f.env_cloned().is_empty()) {
                return Err(error(
                    "durable prompt tools must be capture-free functions".into(),
                ));
            }
            let callbacks: Vec<_> = prompt
                .params
                .iter()
                .filter_map(|p| functions.get(p.name.as_str()))
                .map(|(f, _)| format!("{:?}", f.body()))
                .collect();
            let identity = json!([
                req.prompt,
                span.start,
                span.end,
                model,
                req.rendered,
                req.args,
                req.sampling.to_trace_json(),
                turn.definitions,
                callbacks
            ]);
            let key = format!("{:x}", Sha256::digest(identity.to_string().as_bytes()));
            Some(ConversationJournal::open(job, key).map_err(error)?)
        } else {
            None
        };
        let mut ids = HashSet::new();
        loop {
            turn.final_answer_only = ids.len() == MAX_PROMPT_TOOL_CALLS
                || journal
                    .as_ref()
                    .is_some_and(ConversationJournal::final_answer_only);
            let mut request = req.as_ref().with_model(model);
            request.tool_turn = Some(&turn);
            let mut args = req.args.clone();
            args.push(json!({"tools":turn.definitions, "history":turn.history}));
            request.args = &args;
            let saved = match journal.as_mut() {
                Some(j) => j.response().map_err(error)?,
                None => None,
            };
            let response = if let Some(saved) = saved {
                saved
            } else {
                let response = self
                    .runtime
                    .call_llm_ref(request)
                    .await
                    .map_err(|e| InterpError::new(InterpErrorKind::Runtime(e), span))?;
                if response.value.is_string() {
                    if let Some(j) = journal.as_mut() {
                        j.commit(&response, None).map_err(error)?;
                    }
                }
                response
            };
            if response.value.is_string() {
                return Ok(response);
            }
            let cost = self.prompt_call_cost(prompt, model, &req.rendered, response.usage);
            self.charge_cost(cost, span)?;
            let calls = response.value["tool_calls"]
                .as_array()
                .filter(|c| !c.is_empty())
                .ok_or_else(|| error("expected a text answer or function calls".into()))?;
            if calls.len() > MAX_PROMPT_TOOL_CALLS - ids.len() {
                return Err(error(format!(
                    "function-call limit of {MAX_PROMPT_TOOL_CALLS} exceeded"
                )));
            }
            if let Some(j) = &journal {
                j.check_batch(&response, calls.len()).map_err(error)?;
            }
            turn.history.push(response.value.clone());
            let mut pending = Vec::with_capacity(calls.len());
            for call in calls {
                let id = call["id"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| error("missing function call id".into()))?;
                if !ids.insert(id.to_owned()) || call["type"] != "function" {
                    return Err(error(format!("invalid or repeated function call {id}")));
                }
                let name = call["function"]["name"].as_str().unwrap_or("");
                let (function, types) = functions
                    .get(name)
                    .ok_or_else(|| error(format!("unknown prompt function {name}")))?;
                let args: Map<String, serde_json::Value> =
                    serde_json::from_str(call["function"]["arguments"].as_str().unwrap_or(""))
                        .map_err(|e| error(format!("invalid arguments for {name}: {e}")))?;
                if args.len() != types.len() {
                    return Err(error(format!("wrong argument count for {name}")));
                }
                let mut inputs = Vec::new();
                for ((_, key), ty) in function.params().iter().zip(types.iter()) {
                    let raw = args
                        .get(key)
                        .ok_or_else(|| error(format!("missing argument {name}.{key}")))?;
                    inputs.push(
                        json_to_value(raw.clone(), ty, &self.types_by_id)
                            .map_err(|e| error(format!("invalid argument {name}.{key}: {e}")))?,
                    );
                }
                pending.push((id, *function, inputs));
            }
            for (id, function, inputs) in pending {
                if let Some(j) = journal.as_mut() {
                    if let Some(saved) = j.tool_result(&response, id).map_err(error)? {
                        turn.history.push(saved);
                        continue;
                    }
                }
                let value = self.apply_closure(function, inputs, span).await?;
                let result = json!({"role":"tool","tool_call_id":id,"content":value_to_json(&value).to_string()});
                if let Some(j) = journal.as_mut() {
                    j.commit(&response, Some(&result)).map_err(error)?;
                }
                turn.history.push(result);
            }
        }
    }
}
