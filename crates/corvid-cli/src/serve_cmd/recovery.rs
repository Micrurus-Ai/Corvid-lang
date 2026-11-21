//! Durable checkpoints for a common HTTP action shape: a route tail-calls
//! an agent, the agent reviews its parameters, then returns one call. The
//! checkpoint contains all live bindings, so recovery can enter the suffix
//! directly. Never replay a handler prefix to reconstruct missing state.
use super::*;
use corvid_ir::{IrCallKind, IrExprKind};
use corvid_runtime::approval_queue::ApprovalQueueRecord;
use sha2::{Digest, Sha256};

pub(super) struct Plan {
    pub label: String,
    pub key: String,
    agent: usize,
}

pub(super) fn plan(state: &ServeState, handler: &str) -> Option<Plan> {
    let ir = &state.ir;
    let route = ir
        .servers
        .iter()
        .flat_map(|s| &s.routes)
        .find(|r| r.handler_agent == handler)?;
    let [IrStmt::Return {
        value: Some(call), ..
    }] = route.body.stmts.as_slice()
    else {
        return None;
    };
    let IrExprKind::Call {
        kind: IrCallKind::Agent { def_id },
        args,
        ..
    } = &call.kind
    else {
        return None;
    };
    if !args
        .iter()
        .all(|arg| matches!(arg.kind, IrExprKind::Local { .. }))
    {
        return None;
    }
    let agent = ir.agents.iter().position(|a| a.id == *def_id)?;
    let a = &ir.agents[agent];
    let [IrStmt::Approve { label, args, .. }, IrStmt::Return {
        value: Some(result),
        ..
    }] = a.body.stmts.as_slice()
    else {
        return None;
    };
    let IrExprKind::Call {
        kind: IrCallKind::Tool { .. } | IrCallKind::StructConstructor { .. },
        args: call_args,
        ..
    } = &result.kind
    else {
        return None;
    };
    if !call_args
        .iter()
        .all(|arg| matches!(arg.kind, IrExprKind::Local { .. } | IrExprKind::Literal(_)))
    {
        return None;
    }
    // Independent flat values round-trip through the typed HTTP decoder.
    // Streams, nested mutable graphs, provenance and host handles need
    // their own checkpoint representation.
    fn scalar(ty: &Type) -> bool {
        matches!(ty, Type::Int | Type::Bool | Type::String)
    }
    if args.len() != a.params.len() || !args.iter().zip(&a.params).all(|(arg, param)| {
        matches!(arg.kind, IrExprKind::Local { local_id, .. } if local_id == param.local_id)
            && (scalar(&param.ty) || matches!(&param.ty, Type::Struct(id)
                if ir.types.iter().any(|t| t.id == *id && t.variants.is_empty() && t.fields.iter().all(|f| scalar(&f.ty)))))
    }) { return None; }
    let mode = state.runtime.connector_mode();
    let contract = format!(
        "v1:{route:?}:{a:?}:{:?}:{:?}:{:?}:{mode:?}",
        ir.types, ir.tools, ir.connectors
    );
    Some(Plan {
        agent,
        key: hex::encode(Sha256::digest(contract)),
        label: label.clone(),
    })
}

pub(super) fn restore(
    state: &ServeState,
    record: &ApprovalQueueRecord,
) -> Result<Option<Invocation>, String> {
    let Some(json) = state
        .approval_queue
        .checkpoint(&record.id)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    #[derive(serde::Deserialize)]
    struct Stored {
        version: u32,
        key: String,
        args: Vec<serde_json::Value>,
    }
    let stored: Stored = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let route = state
        .ir
        .servers
        .iter()
        .flat_map(|s| &s.routes)
        .find(|r| format!("{} {}", r.method.as_str(), r.path) == record.target_id);
    let Some(plan) = route.and_then(|r| plan(state, &r.handler_agent)) else {
        return Ok(None);
    };
    if stored.version != 1 || stored.key != plan.key || record.action != plan.label {
        return Ok(None);
    }
    let mut ir = (*state.ir).clone();
    let agent = &mut ir.agents[plan.agent];
    if stored.args.len() != agent.params.len() {
        return Err("checkpoint arity mismatch".into());
    }
    let types = ir.types.iter().map(|t| (t.id, t)).collect();
    let args = stored
        .args
        .into_iter()
        .zip(&agent.params)
        .map(|(json, param)| {
            json_to_value(json, &param.ty, &types)
                .map_err(|e| format!("checkpoint argument: {e:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    agent.body.stmts.remove(0);
    let name = agent.name.clone();
    let runtime = state.runtime.clone();
    let (_, pauses) = tokio::sync::mpsc::unbounded_channel();
    Ok(Some(Invocation {
        execution: Box::pin(
            async move { run_ir_with_runtime(&ir, Some(&name), args, &runtime).await },
        ),
        pauses,
    }))
}
