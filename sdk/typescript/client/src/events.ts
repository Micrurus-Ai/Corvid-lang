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

/**
 * The public answer to a server failure: a code naming the kind of stop
 * and a sentence safe to show a person. It never holds the server's own
 * error text.
 */
export interface RouteFailure {
  error: string;
  detail: string;
}

/**
 * One event of a route that returns `Stream<Item>`. A typed `Err` item the
 * program handles arrives as a `chunk`, like `Ok`; `failed` is a server
 * failure; `interrupted` means the connection was lost before the server
 * said `completed` or `failed`. Each is the last event when it occurs,
 * except `chunk`.
 */
export type RouteEvent<Item> =
  | { kind: "chunk"; value: Item }
  | { kind: "completed"; value: null }
  | { kind: "failed"; error: RouteFailure }
  | { kind: "interrupted" };

/**
 * A route's stream of items plus `result()`, which resolves to `null` when
 * the route completes and rejects with a `CorvidError` on `failed` or
 * `interrupted`. A route stream has no final value; an agent's does
 * (`AgentStream`).
 */
export interface RouteStream<Item> extends AsyncIterable<RouteEvent<Item>> {
  result(): Promise<null>;
}
