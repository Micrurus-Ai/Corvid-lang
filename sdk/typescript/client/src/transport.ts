// The generic Corvid transport (slice 51l).
//
// One `CorvidClient` instance is shared by all generated methods. It
// owns every cross-cutting concern the moat depends on — session
// cookies, typed errors, the SSE event protocol, and cursor pagination
// — so the generated per-app code stays a thin, typed veneer over this
// audited surface. Nothing here is regenerated per app.

import { CorvidError } from "./errors.js";
import { type AgentEvent, type AgentStream, type ApprovalRequest } from "./events.js";
import { type Page, paginate } from "./pagination.js";

export interface CorvidClientOptions {
  /** Base URL of the Corvid backend, e.g. `https://api.example.com`. */
  baseUrl: string;
  /** Extra headers to send on every request. */
  headers?: Record<string, string>;
  /** Override the global fetch (for tests / non-browser runtimes). */
  fetch?: typeof fetch;
}

export class CorvidClient {
  private readonly baseUrl: string;
  private readonly headers: Record<string, string>;
  private readonly doFetch: typeof fetch;

  constructor(options: CorvidClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    this.headers = options.headers ?? {};
    this.doFetch = options.fetch ?? fetch.bind(globalThis);
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
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let sep: number;
        while ((sep = buffer.indexOf("\n\n")) !== -1) {
          const raw = buffer.slice(0, sep);
          buffer = buffer.slice(sep + 2);
          const event = parseSseEvent<T>(raw);
          if (event) yield event;
        }
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

function parseSseEvent<T>(raw: string): AgentEvent<T> | null {
  let event = "message";
  const dataLines: string[] = [];
  for (const line of raw.split("\n")) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    else if (line.startsWith("data:")) dataLines.push(line.slice(5).trim());
  }
  if (dataLines.length === 0) return null;
  let payload: unknown = undefined;
  try {
    payload = JSON.parse(dataLines.join("\n"));
  } catch {
    payload = dataLines.join("\n");
  }
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
