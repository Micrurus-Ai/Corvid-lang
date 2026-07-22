// React hook for mid-invocation approvals (slice 51n).
//
// A Corvid agent that reaches an approval-requiring action emits an
// `approval_required` event mid-stream (slice 51h / the corvid-ai event
// protocol). This hook collects those from a live stream handle and
// exposes approve/deny actions that POST to the backend's approval
// endpoints via the shared client.

import { useCallback, useEffect, useState } from "react";
import { CorvidClient, type AgentEvent, type ApprovalRequest } from "@corvid/client";
import { type StreamState } from "./useCorvidAgent.js";

export interface ApprovalsHandle {
  /** Approvals surfaced by the stream and not yet resolved. */
  pending: ApprovalRequest[];
  /** Approve an approval by id. */
  approve: (id: string) => Promise<void>;
  /** Deny an approval by id. */
  deny: (id: string) => Promise<void>;
}

/**
 * Track the `approval_required` events in a stream's event log and
 * resolve them through the client. Pair with `useCorvidStream`:
 *
 * ```ts
 * const chat = useCorvidStream((m: string) => api.chat(m));
 * const approvals = useCorvidApprovals(client, chat.events);
 * ```
 */
export function useCorvidApprovals(
  client: CorvidClient,
  events: AgentEvent<unknown>[],
): ApprovalsHandle {
  const [resolved, setResolved] = useState<Set<string>>(() => new Set());

  // Derive pending approvals from the event log minus the resolved set.
  const [pending, setPending] = useState<ApprovalRequest[]>([]);
  useEffect(() => {
    const seen = new Map<string, ApprovalRequest>();
    for (const e of events) {
      if (e.kind === "approval_required") seen.set(e.approval.id, e.approval);
    }
    setPending([...seen.values()].filter((a) => !resolved.has(a.id)));
  }, [events, resolved]);

  const resolve = useCallback(
    async (id: string, decision: "approve" | "deny"): Promise<void> => {
      await client.request<void>("POST", `/approvals/${encodeURIComponent(id)}/${decision}`);
      setResolved((prev) => {
        const next = new Set(prev);
        next.add(id);
        return next;
      });
    },
    [client],
  );

  return {
    pending,
    approve: (id) => resolve(id, "approve"),
    deny: (id) => resolve(id, "deny"),
  };
}

// Re-export the stream state shape hook consumers commonly pair with.
export type { StreamState };
