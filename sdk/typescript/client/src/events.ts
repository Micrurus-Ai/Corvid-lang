// The typed agent event protocol (slice 51l), mirroring the
// `corvid-ai.json` event union (slice 51c). A streamed agent emits
// these over SSE; the client parses them into a typed async iterable so
// a frontend writes `for await (const event of agent.stream(...))`
// instead of hand-parsing `text/event-stream`.

/** One event in an agent invocation's lifetime. */
export type AgentEvent<T> =
  | { kind: "started"; runId: string }
  | { kind: "chunk"; value: T }
  | { kind: "tool_started"; tool: string }
  | { kind: "tool_completed"; tool: string }
  | { kind: "approval_required"; approval: ApprovalRequest }
  | { kind: "completed"; value: T }
  | { kind: "failed"; error: unknown };

/** A mid-invocation approval the agent is blocked on. */
export interface ApprovalRequest {
  id: string;
  label: string;
  summary?: string;
}

/**
 * A typed stream of agent events plus a convenience `result()` that
 * resolves to the terminal `completed` value (or rejects on `failed`).
 */
export interface AgentStream<T> extends AsyncIterable<AgentEvent<T>> {
  /** Await the final value, ignoring intermediate events. */
  result(): Promise<T>;
}
