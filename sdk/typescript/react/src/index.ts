// `@corvid/react` — optional React hooks over a Corvid application
// (slice 51n). Generic implementations specialized by the types
// `corvid contract ts-client` generates. Build on the shipped
// `@corvid/client` transport; nothing here is per-app.

export {
  useCorvidAgent,
  useCorvidStream,
  type AgentState,
  type AgentHandle,
  type StreamState,
  type StreamHandle,
} from "./useCorvidAgent.js";
export {
  useCorvidApprovals,
  type ApprovalsHandle,
} from "./useCorvidApprovals.js";
export {
  useCorvidPaginated,
  type PaginatedHandle,
} from "./useCorvidPaginated.js";

// Optional prototype/admin components (slice 51p).
export * from "./components/index.js";
