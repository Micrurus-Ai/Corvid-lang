// The generic Corvid transport (slice 51l).
//
// One `CorvidClient` instance is shared by all generated methods. It
// owns every cross-cutting concern the moat depends on — session
// cookies, typed errors, the SSE event protocol, and cursor pagination
// — so the generated per-app code stays a thin, typed veneer over this
// audited surface. Nothing here is regenerated per app.

import { CorvidError } from "./errors.js";
import { connectionForPage, type LiveConnection, type LiveObserver } from "./live-connection.js";
import {
  type AgentEvent,
  type AgentStream,
  type ApprovalRequest,
  type RouteEvent,
  type RouteFailure,
  type RouteStream,
} from "./events.js";
import { type Page, paginate } from "./pagination.js";
import { fillPath, urlEncodedRecord } from "./path.js";
import { readSseEvents, type SseEvent } from "./sse.js";

/**
 * How a generated route call lays out its arguments: the path parameters
 * in path order, then the query record when `query` is true, then the
 * body in the form `body` names.
 */
export interface RouteShape {
  readonly params: readonly string[];
  readonly query: boolean;
  readonly body: "json" | "form" | null;
}

/**
 * An action call a reviewer must approve first. Poll `poll` for the
 * decision; approving runs the action.
 */
export interface ApprovalPending {
  readonly kind: "approval_pending";
  readonly approvalId: string;
  readonly poll: string;
}

export interface CorvidClientOptions {
  /** Base URL of the Corvid backend, e.g. `https://api.example.com`. */
  baseUrl: string;
  /** Extra headers to send on every request. */
  headers?: Record<string, string>;
  /** Override the global fetch (for tests / non-browser runtimes). */
  fetch?: typeof fetch;
}

export class CorvidClient {
  private live?: LiveConnection;
  private readonly baseUrl: string;
  private readonly headers: Record<string, string>;
  private readonly doFetch: typeof fetch;

  constructor(options: CorvidClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    this.headers = options.headers ?? {};
    this.doFetch = options.fetch ?? fetch.bind(globalThis);
  }

  /** Subscribe with a concrete route path and its ordered table key fields.
   * The returned function releases this subscription and is safe to call twice. */
  subscribe<Row extends object>(
    path: string, keyFields: readonly (keyof Row)[], observer: LiveObserver<Row>,
  ): () => void {
    this.live ??= connectionForPage(this.baseUrl);
    return this.live.subscribe(path, keyFields, observer, CorvidError);
  }

  /** Invoke a public agent/prompt and await its single JSON result. */
  async invoke<T>(name: string, input: unknown): Promise<T> {
    const res = await this.doFetch(`${this.baseUrl}/agents/${encodeURIComponent(name)}`, {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/json", ...this.headers },
      body: JSON.stringify(input ?? {}),
    });
    return this.parse<T>(res);
  }

  /**
   * Call a declared `action` at `POST /actions/<name>` with its input:
   * the record itself when the action takes one, otherwise an object
   * with one field per parameter. An action that declares `@approval`
   * answers `202` while a reviewer decides, which resolves to an
   * `ApprovalPending`; every other non-2xx answer throws `CorvidError`.
   */
  async action<T>(name: string, input: unknown): Promise<T | ApprovalPending> {
    const res = await this.doFetch(`${this.baseUrl}/actions/${encodeURIComponent(name)}`, {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/json", ...this.headers },
      body: JSON.stringify(input ?? {}),
    });
    if (res.status === 202) {
      const queued = (await safeJson(res)) as { approval_id?: string; poll?: string } | null;
      return { kind: "approval_pending", approvalId: queued?.approval_id ?? "", poll: queued?.poll ?? "" };
    }
    return this.parse<T>(res);
  }

  /** A typed low-level request against a declared route. */
  async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const res = await this.doFetch(`${this.baseUrl}${path}`, {
      method,
      credentials: "include",
      headers: body === undefined ? this.headers : { "content-type": "application/json", ...this.headers },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return this.parse<T>(res);
  }

  /** Invoke a streaming agent and consume its typed SSE event stream. */
  stream<T>(name: string, input: unknown): AgentStream<T> {
    const url = `${this.baseUrl}/agents/${encodeURIComponent(name)}/stream`;
    const doFetch = this.doFetch;
    const headers = this.headers;

    async function* iterate(): AsyncGenerator<AgentEvent<T>> {
      const res = await doFetch(url, {
        method: "POST",
        credentials: "include",
        headers: { "content-type": "application/json", accept: "text/event-stream", ...headers },
        body: JSON.stringify(input ?? {}),
      });
      if (!res.ok || !res.body) {
        throw new CorvidError(res.status, await safeJson(res), "stream failed to open");
      }
      for await (const raw of readSseEvents(res.body)) {
        const event = agentEvent<T>(raw);
        if (event) yield event;
      }
    }

    const gen = iterate();
    return {
      [Symbol.asyncIterator]: () => gen,
      async result(): Promise<T> {
        for await (const event of gen) {
          if (event.kind === "completed") return event.value;
          if (event.kind === "failed") throw new CorvidError(200, event.error, "agent failed");
        }
        throw new CorvidError(200, null, "stream ended without a terminal event");
      },
    };
  }

  /**
   * Call a declared route that returns `Stream<Item>`. `route` is the
   * method and contract path, as in `"GET /boards/{board}/feed"`, and
   * `args` follow `shape`. Generated `Api.route` calls this. A path value
   * that cannot be sent exactly throws here, before any request.
   */
  routeStream<Item>(route: string, shape: RouteShape, args: readonly unknown[]): RouteStream<Item> {
    const space = route.indexOf(" ");
    const method = route.slice(0, space);
    const values: Record<string, unknown> = {};
    let next = 0;
    for (const name of shape.params) values[name] = args[next++];
    let path = fillPath(route.slice(space + 1), values);
    if (shape.query) {
      const query = urlEncodedRecord(args[next++]);
      if (query) path += "?" + query;
    }
    const headers: Record<string, string> = { accept: "text/event-stream", ...this.headers };
    let body: string | undefined;
    if (shape.body === "json") {
      body = JSON.stringify(args[next++]);
      headers["content-type"] = "application/json";
    } else if (shape.body === "form") {
      body = urlEncodedRecord(args[next++]);
      headers["content-type"] = "application/x-www-form-urlencoded";
    }
    const url = `${this.baseUrl}${path}`;
    const doFetch = this.doFetch;

    async function* iterate(): AsyncGenerator<RouteEvent<Item>> {
      let res: Response;
      try {
        res = await doFetch(url, { method, credentials: "include", headers, body });
      } catch {
        yield { kind: "interrupted" };
        return;
      }
      if (!res.ok || !res.body) {
        throw new CorvidError(res.status, await safeJson(res), "stream failed to open");
      }
      try {
        for await (const { event, data } of readSseEvents(res.body)) {
          if (event === "chunk") {
            yield { kind: "chunk", value: data as Item };
          } else if (event === "completed") {
            yield { kind: "completed", value: null };
            return;
          } else if (event === "failed") {
            yield { kind: "failed", error: routeFailure(data) };
            return;
          }
        }
      } catch {
        // The connection failed mid-read; reported as interrupted below.
      }
      yield { kind: "interrupted" };
    }

    const gen = iterate();
    return {
      [Symbol.asyncIterator]: () => gen,
      async result(): Promise<null> {
        for await (const event of gen) {
          if (event.kind === "completed") return null;
          if (event.kind === "failed") throw new CorvidError(200, event.error, event.error.detail);
          if (event.kind === "interrupted") {
            throw new CorvidError(0, null, "the connection was lost before the stream completed");
          }
        }
        throw new CorvidError(0, null, "the stream ended without a terminal event");
      },
    };
  }

  /** Walk a cursor-paginated route to exhaustion. */
  paginate<Item>(path: string): AsyncGenerator<Item> {
    return paginate<Item>((cursor) => {
      const q = cursor ? `${path}${path.includes("?") ? "&" : "?"}cursor=${encodeURIComponent(cursor)}` : path;
      return this.request<Page<Item>>("GET", q);
    });
  }

  // ---- auth / session (slice 51h) ----

  /** Redirect the browser to begin sign-in with a provider. */
  login(provider: string): void {
    globalThis.location.href = `${this.baseUrl}/auth/${encodeURIComponent(provider)}/login`;
  }

  /** Fetch the current authenticated actor (or throw 401). */
  session(): Promise<unknown> {
    return this.request("GET", "/auth/session");
  }

  /** Revoke the session and clear the cookie. */
  async logout(): Promise<void> {
    await this.request<void>("POST", "/auth/logout");
  }

  private async parse<T>(res: Response): Promise<T> {
    if (!res.ok) {
      throw new CorvidError(res.status, await safeJson(res));
    }
    return (await res.json()) as T;
  }
}

async function safeJson(res: Response): Promise<unknown> {
  try {
    return await res.json();
  } catch {
    return null;
  }
}

/** An agent invocation's event, or `null` for one it does not define. */
function agentEvent<T>({ event, data: payload }: SseEvent): AgentEvent<T> | null {
  switch (event) {
    case "started":
      return { kind: "started", runId: (payload as { run_id?: string })?.run_id ?? "" };
    case "chunk":
      return { kind: "chunk", value: payload as T };
    case "tool_started":
      return { kind: "tool_started", tool: (payload as { tool?: string })?.tool ?? "" };
    case "tool_completed":
      return { kind: "tool_completed", tool: (payload as { tool?: string })?.tool ?? "" };
    case "approval_required":
      return { kind: "approval_required", approval: payload as ApprovalRequest };
    case "completed":
      return { kind: "completed", value: payload as T };
    case "failed":
      return { kind: "failed", error: payload };
    default:
      return null;
  }
}

/** A `failed` event's data; a malformed one still yields a safe sentence. */
function routeFailure(data: unknown): RouteFailure {
  const failure = (data ?? {}) as Partial<RouteFailure>;
  return {
    error: typeof failure.error === "string" ? failure.error : "handler_failed",
    detail: typeof failure.detail === "string" ? failure.detail : "the server stopped the stream",
  };
}
