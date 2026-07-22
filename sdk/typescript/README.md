# Corvid TypeScript SDK

A Corvid backend describes its public surface as a machine-readable
[Application Contract](../../docs/reference/inventions.md). The TypeScript
SDK turns that contract into a typed browser client with **no hand-written
glue**.

## Two pieces

- **`@corvid/client`** (`client/`) — the generic transport, shipped once
  and reused by every app. It owns the cross-cutting behavior:
  - session-cookie auth (`credentials: "include"`) and the
    `/auth/{provider}/login|logout|session` helpers,
  - the typed agent **event protocol** over SSE
    (`started`/`chunk`/`tool_started`/`tool_completed`/`approval_required`/`completed`/`failed`),
  - typed errors (`CorvidError`, carrying the `@status` code and the
    parsed error-enum body),
  - cursor **pagination** (`Page<Item>`, `paginate(...)`).
- **Generated `types.ts` + `api.ts`** — produced per app by
  `corvid contract ts-client`. `types.ts` is one `interface` per record
  and a discriminated union per sum/error type (with a `…Meta` map for
  `@status`/`@ui`); `api.ts` is a thin, typed `Api` class whose methods
  delegate to `@corvid/client`. Nothing cross-cutting is regenerated —
  upgrades to streaming, approvals, grounding, or pagination happen in
  the one shipped package.

## Usage

```bash
corvid contract ts-client src/main.cor --out sdk/generated
```

```ts
import { CorvidClient } from "@corvid/client";
import { Api } from "./sdk/generated/api";

const api = new Api(new CorvidClient({ baseUrl: "https://api.example.com" }));

const answer = await api.classify("hello");            // typed result
for await (const event of api.chat("hi")) {            // typed SSE events
  if (event.kind === "chunk") render(event.value);
}
api.loginWith_google();                                // auth helper
```

Define the backend once in Corvid; the frontend gets types, methods,
streaming, auth, errors, and pagination for free.

## React hooks (optional)

`@corvid/react` (`react/`) layers idiomatic hooks over the same client
and the generated `Api`. The hooks are generic — the generated method
signatures specialize them at the call site:

```tsx
import { useCorvidAgent, useCorvidStream, useCorvidPaginated } from "@corvid/react";
import { Api } from "./sdk/generated/api";

const api = new Api(client);

const classify = useCorvidAgent((q: string) => api.classify(q)); // classify.data: Answer | null
const chat = useCorvidStream((m: string) => api.chat(m));        // chat.chunks: string[]
const feed = useCorvidPaginated((c) => api.browse(c ?? ""));     // feed.items: Item[]
```

- `useCorvidAgent` / `useCorvidStream` — invoke an agent, tracking
  `data`/`error`/`loading`, or consume a streaming agent's typed event
  log with accumulated `chunks` and a terminal `result`.
- `useCorvidApprovals` — surface `approval_required` events from a
  stream and resolve them (`approve`/`deny`) through the client.
- `useCorvidPaginated` — cursor pagination with `items`, `loadMore`,
  and `hasMore`.

### Prototype components (optional)

`@corvid/react` also ships headless-ish scaffolding components for
standing up an admin panel or demo fast — **not** polished product UI.
They accept `className` for restyling and specialize with the generated
types:

```tsx
import { CorvidAgentForm, CorvidSignIn, CorvidStream } from "@corvid/react";

<CorvidSignIn client={client} providers={["google", "github"]} />
<CorvidAgentForm
  fields={[{ name: "question" }]}
  call={(v) => api.classify(v.question)}
  renderResult={(a) => <p>{a.text}</p>}   // a: Answer, inferred
/>
```

`CorvidAgentForm`, `CorvidStream`, `CorvidApprovalQueue`,
`CorvidGroundedAnswer`, `CorvidReviewQueue`, `CorvidSignIn`. For real
product UI, use the hooks directly.
