// Approval queue (slice 51p). Renders the pending `approval_required`
// events from a stream's event log with approve/deny buttons.

import { type ReactNode } from "react";
import { CorvidClient, type AgentEvent } from "@corvid/client";
import { useCorvidApprovals } from "../useCorvidApprovals.js";

export interface CorvidApprovalQueueProps {
  client: CorvidClient;
  /** The event log from a `useCorvidStream` handle. */
  events: AgentEvent<unknown>[];
  className?: string;
}

/** A live queue of mid-invocation approvals awaiting a decision. */
export function CorvidApprovalQueue(props: CorvidApprovalQueueProps): ReactNode {
  const { client, events, className } = props;
  const approvals = useCorvidApprovals(client, events);

  if (approvals.pending.length === 0) {
    return <div className={className} data-empty="true" />;
  }
  return (
    <ul className={className} style={{ listStyle: "none", padding: 0 }}>
      {approvals.pending.map((a) => (
        <li key={a.id} className="corvid-approval" style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <span style={{ flex: 1 }}>
            <strong>{a.label}</strong>
            {a.summary ? ` — ${a.summary}` : ""}
          </span>
          <button onClick={() => void approvals.approve(a.id)}>Approve</button>
          <button onClick={() => void approvals.deny(a.id)}>Deny</button>
        </li>
      ))}
    </ul>
  );
}
