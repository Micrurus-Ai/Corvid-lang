# Reference apps

## What ships

The `examples/` directory contains canonical Corvid programs that
exercise every shipped invention end-to-end:

- `refund_bot.cor` — approval-gated refund agent with budgets.
- `rag_qa_bot.cor` — grounded retrieval Q&A.
- `support_escalation_bot.cor` — multi-step support agent with loop
  bounds and human approval.
- `code_review_agent.cor` — code-review agent with effect rows on
  filesystem and version-control reads.
- `model_provider_routing.cor` — typed model substrate routing.
- `local_model_chat.cor` — local model via the model substrate.

## How to run

```sh
git clone https://github.com/Micrurus-Ai/Corvid-lang
cd Corvid-lang/examples
corvid run refund_bot.cor
```

Each example has tests, evals, traces, and benchmark notes.

## What each one teaches

| Example | Inventions exercised |
|---|---|
| refund_bot | approve, budget, audit log |
| rag_qa_bot | Grounded<T>, citation, retrieval effect |
| support_escalation_bot | loop bounds, await_approval, durable jobs |
| code_review_agent | filesystem effect, multi-prompt composition |
| model_provider_routing | model substrate, swap_model eval |
| local_model_chat | local model substrate, no-network compile |

## Drawing editor storage

`examples/drawing_editor/drawing-store.js` exports these browser functions for the 96 × 64 pixel sketchbook.

| Function | Result |
| --- | --- |
| `openDrawing(onBlocked?)` | Resolves to the version 2 IndexedDB connection scoped to the example's directory. The optional callback reports an older connection blocking migration. Existing version 1 cells become pending uploads in the upgrade transaction. The connection closes on a subsequent version change. |
| `restoreCells(db)` | Resolves to the stored `[x, y]` coordinate pairs. |
| `saveCells(db, cells)` | Takes a worker stroke as alternating x and y coordinates. Resolves after the marks and pending keys commit together. |
| `synchronizeDrawing(db, resource)` | Reads the resource, uploads the union of remote and pending marks when needed, then commits downloaded marks and clears the corresponding pending keys together. Rejects on network, format, transaction or unresolved revision errors. Concurrently queued distinct marks stay pending. |

The editor calls `GET ./sketch.json` and expects either 404 or a 200 JSON document such as `{"version":1,"cells":[[10,20]]}` with a strong ETag. Coordinates are integer pairs inside the canvas. A host that supports sharing must atomically enforce `If-None-Match: *` for creation and `If-Match` for replacement, change the ETag on modification, and return 200, 201 or 204 after committing. A 412 triggers a fresh read and merge, up to three attempts per sync. The format supports adding painted cells. Replaying an upload after a lost acknowledgement preserves the union. An unknown document format stops the upload and keeps pending edits.

## The benchmark archives

Phase 33N ships side-by-side benchmark runners against Python and
TypeScript implementations of `refund_bot`, `rag_qa_bot`, and
`support_escalation_bot`. Results live in
[`benches/moat/RESULTS.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/benches/moat/RESULTS.md).
