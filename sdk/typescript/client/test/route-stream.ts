// Run by scripts/verify_generated_route_stream.py against `corvid serve`
// on route_stream.cor, after `corvid contract ts-client` generated
// ./api.ts beside the client's sources. It checks the three outcomes a
// frontend must tell apart: a handled application error (an `Err` chunk),
// a server failure (`failed`) and a lost connection (`interrupted`).
//
// Usage: node route-stream.js <server url> <url of a proxy that drops the
// connection after the first event>

import { Api } from "./api.js";
import { CorvidClient, CorvidError, type RouteEvent } from "./index.js";

declare const process: { argv: string[]; exitCode?: number };

const [serverUrl, droppingProxyUrl] = process.argv.slice(2);

function check(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function collect<Item>(stream: AsyncIterable<RouteEvent<Item>>): Promise<RouteEvent<Item>[]> {
  const events: RouteEvent<Item>[] = [];
  for await (const event of stream) events.push(event);
  return events;
}

async function rejection(promise: Promise<unknown>): Promise<CorvidError> {
  try {
    await promise;
  } catch (error) {
    check(error instanceof CorvidError, `expected a CorvidError, got ${error}`);
    return error;
  }
  throw new Error("expected the promise to reject");
}

const api = new Api(new CorvidClient({ baseUrl: serverUrl }));

// A handled application error is a value in the same loop as Ok, and the
// route still completes, with null.
const feed = await collect(api.route("GET /boards/{board}/feed", 7));
check(feed.map((e) => e.kind).join() === "chunk,chunk,completed", `feed: ${JSON.stringify(feed)}`);
const [first, second, end] = feed;
check(first.kind === "chunk" && "ok" in first.value && first.value.ok.text === "board 7", `Ok chunk: ${JSON.stringify(first)}`);
check(
  second.kind === "chunk" && "err" in second.value && second.value.err.variant === "Paused"
    && second.value.err.fields?.[0] === "rate limited",
  `Err chunk: ${JSON.stringify(second)}`,
);
check(end.kind === "completed" && end.value === null, `completed: ${JSON.stringify(end)}`);
const completed: null = await api.route("GET /boards/{board}/feed", 7).result();
check(completed === null, "result() of a completed route is null");

// A server failure is `failed`, with a public sentence that never holds
// the refused URL or the path in it.
const broken = await collect(api.route("GET /boards/{board}/broken", 7));
check(broken.map((e) => e.kind).join() === "chunk,failed", `broken: ${JSON.stringify(broken)}`);
const failed = broken[1];
check(failed.kind === "failed" && failed.error.error === "handler_failed", `failed: ${JSON.stringify(failed)}`);
for (const leak of ["127.0.0.1", "/Users/ops", "http"]) {
  check(!failed.error.detail.includes(leak), `failed.detail leaked ${leak}: ${failed.error.detail}`);
}
const serverFailure = await rejection(api.route("GET /boards/{board}/broken", 7).result());
check((serverFailure.body as { error?: string }).error === "handler_failed", `result() on failed: ${serverFailure.message}`);

// A connection lost before `completed` or `failed` is `interrupted`.
const dropping = new Api(new CorvidClient({ baseUrl: droppingProxyUrl }));
const lost = await collect(dropping.route("GET /boards/{board}/feed", 7));
check(lost.map((e) => e.kind).join() === "chunk,interrupted", `lost: ${JSON.stringify(lost)}`);
const interrupted = await rejection(dropping.route("GET /boards/{board}/feed", 7).result());
check(interrupted.message.includes("connection was lost"), `result() on interrupted: ${interrupted.message}`);

// A path value the server would read differently is refused before any
// request is sent.
let refused = false;
try {
  api.route("GET /boards/{board}/feed", 2 ** 60);
} catch {
  refused = true;
}
check(refused, "an inexact Int path value must be refused");

// The routes table is checked at compile time; these lines must not
// compile, and tsc fails the run if one does.
async function compileTimeChecks() {
  // @ts-expect-error board is an Int path parameter.
  api.route("GET /boards/{board}/feed", "7");
  // @ts-expect-error the route needs its path parameter.
  api.route("GET /boards/{board}/feed");
  // @ts-expect-error no such route is declared.
  api.route("GET /boards/{board}/missing", 7);
  // @ts-expect-error a route stream completes with null, not a value.
  const value: string = await api.route("GET /boards/{board}/feed", 7).result();
  return value;
}
void compileTimeChecks;

console.log("Generated client read Ok and Err chunks, completed with null, a sanitized failure and an interrupted stream.");
