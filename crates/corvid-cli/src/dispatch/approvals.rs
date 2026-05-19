use super::*;

pub(crate) fn cmd_approvals(command: ApprovalsCommand) -> Result<u8> {
    use approvals_cmd::*;
    match command {
        ApprovalsCommand::Queue {
            approvals_state,
            tenant,
            status,
        } => {
            let out = run_approvals_queue(ApprovalsQueueArgs {
                approvals_state,
                tenant_id: tenant,
                status,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::to_value(approvals_queue_summary(
                    &out
                ))?)?
            );
            Ok(0)
        }
        ApprovalsCommand::Inspect {
            approvals_state,
            tenant,
            approval_id,
        } => {
            let out = run_approvals_inspect(ApprovalsInspectArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&approvals_inspect_summary(&out))?
            );
            Ok(0)
        }
        ApprovalsCommand::Approve {
            approvals_state,
            tenant,
            actor,
            role,
            reason,
            approval_id,
        } => {
            let summary = run_approvals_approve(ApprovalsTransitionArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
                actor_id: actor,
                role,
                reason,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&approval_summary_value(&summary))?
            );
            Ok(0)
        }
        ApprovalsCommand::Deny {
            approvals_state,
            tenant,
            actor,
            role,
            reason,
            approval_id,
        } => {
            let summary = run_approvals_deny(ApprovalsTransitionArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
                actor_id: actor,
                role,
                reason,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&approval_summary_value(&summary))?
            );
            Ok(0)
        }
        ApprovalsCommand::Expire {
            approvals_state,
            tenant,
            actor,
            reason,
            approval_id,
        } => {
            let summary = run_approvals_expire(ApprovalsExpireArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
                actor_id: actor,
                reason,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&approval_summary_value(&summary))?
            );
            Ok(0)
        }
        ApprovalsCommand::Comment {
            approvals_state,
            tenant,
            actor,
            comment,
            approval_id,
        } => {
            let event = run_approvals_comment(ApprovalsCommentArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
                actor_id: actor,
                comment,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&audit_event_value(&event))?
            );
            Ok(0)
        }
        ApprovalsCommand::Delegate {
            approvals_state,
            tenant,
            actor,
            role,
            delegate_to,
            reason,
            approval_id,
        } => {
            let summary = run_approvals_delegate(ApprovalsDelegateArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
                actor_id: actor,
                role,
                delegate_to,
                reason,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&approval_summary_value(&summary))?
            );
            Ok(0)
        }
        ApprovalsCommand::Batch {
            approvals_state,
            tenant,
            actor,
            role,
            reason,
            ids,
            require_data_class,
        } => {
            let out = run_approvals_batch(ApprovalsBatchArgs {
                approvals_state,
                tenant_id: tenant,
                actor_id: actor,
                role,
                approval_ids: ids,
                reason,
                require_data_class,
            })?;
            let approved = out
                .approved
                .iter()
                .map(approval_summary_value)
                .collect::<Vec<_>>();
            let failed = out
                .failed
                .iter()
                .map(|f| serde_json::json!({"approval_id": f.approval_id, "reason": f.reason}))
                .collect::<Vec<_>>();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "approved": approved,
                    "failed": failed,
                }))?
            );
            Ok(if out.failed.is_empty() { 0 } else { 1 })
        }
        ApprovalsCommand::Export {
            approvals_state,
            tenant,
            since_ms,
            out,
        } => {
            let result = run_approvals_export(ApprovalsExportArgs {
                approvals_state,
                tenant_id: tenant,
                since_ms,
            })?;
            let entries: Vec<serde_json::Value> = result
                .approvals
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "approval": approval_summary_value(&e.approval),
                        "audit_events": e.audit_events.iter().map(audit_event_value).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let payload = serde_json::json!({
                "tenant_id": result.tenant_id,
                "approvals": entries,
            });
            let serialized = serde_json::to_string_pretty(&payload)?;
            if let Some(path) = out {
                std::fs::write(&path, &serialized)
                    .with_context(|| format!("writing export to `{}`", path.display()))?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "wrote_to": path,
                        "approval_count": result.approvals.len(),
                    }))?
                );
            } else {
                println!("{serialized}");
            }
            Ok(0)
        }
        ApprovalsCommand::Explain {
            approvals_state,
            tenant,
            approval_id,
        } => {
            let report = run_approvals_explain(ApprovalsExplainArgs {
                approvals_state,
                tenant_id: tenant,
                approval_id,
            })?;
            let payload = serde_json::json!({
                "approval_id": report.approval_id,
                "tenant_id": report.tenant_id,
                "lifecycle_position": report.lifecycle_position,
                "headline": report.headline,
                "reviewer_facts": {
                    "status": report.reviewer_facts.status,
                    "action": report.reviewer_facts.action,
                    "target_kind": report.reviewer_facts.target_kind,
                    "target_id": report.reviewer_facts.target_id,
                    "required_role": report.reviewer_facts.required_role,
                    "risk_level": report.reviewer_facts.risk_level,
                    "data_class": report.reviewer_facts.data_class,
                    "irreversible": report.reviewer_facts.irreversible,
                    "max_cost_usd": report.reviewer_facts.max_cost_usd,
                    "expires_at_ms": report.reviewer_facts.expires_at_ms,
                    "created_at_ms": report.reviewer_facts.created_at_ms,
                    "trace_id": report.reviewer_facts.trace_id,
                },
                "transitions": report.transitions.iter().map(|t| serde_json::json!({
                    "audit_event_id": t.audit_event_id,
                    "event_kind": t.event_kind,
                    "status_before": t.status_before,
                    "status_after": t.status_after,
                    "actor_id": t.actor_id,
                    "reason": t.reason,
                    "created_at_ms": t.created_at_ms,
                })).collect::<Vec<_>>(),
                "suggested_next_steps": report.suggested_next_steps,
                "sources": report.sources,
            });
            println!("{}", serde_json::to_string_pretty(&payload)?);
            Ok(0)
        }
    }
}
