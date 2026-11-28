# Dynamic fan-out (B46, launch track step 8)

**Status:** proposal, 2026-11-28. It adds statement grammar shared with the frontend, so it is posted for frontend review (item 11 of [the review brief](frontend-review-2026-11-27.md)) and built after one working day without objection.
**Owner:** backend and AI (Claude Code, Windows session).
**Roadmap:** [launch track](../backend-ai-roadmap.md#launch-track-decided-by-the-owner-2026-11-28) step 8.

## The problem

`parallel:` runs a fixed set of named calls at once. An AI app usually needs the other shape: summarize every document a person uploaded, score every lead, check every clause. The count is known only at run time. Today that is a `for` loop that runs one call at a time, or hand-built batching. Python's `asyncio.gather` and JavaScript's `Promise.all` start everything at once, with no limit, no budget and no record of order, so a list of 5,000 becomes 5,000 simultaneous model calls.

## The design

```corvid
agent summarize_all(docs: List<Document>) -> List<Summary> uses llm @budget($2.00):
    summaries = parallel for doc in docs, at most 8:
        summarize(doc)
    return summaries
```

- **`parallel for x in list, at most N:`** runs the body for every item, at most `N` at a time, and gives the results as a `List` in the input's order, whatever order they finished in. The body's last expression is each item's result.
- **The limit is written.** How many calls may run at once is a cost and load choice, so `at most N` is required, with `N` from 1 to 256. There is no default. `N` may be a name, not only a literal.
- **Failure.** Each item that fails fails the whole statement, reported as the first failure in input order, as `parallel:` does today. The other running items are cancelled cooperatively, and their effects follow `parallel:`'s existing rules: a committed write completes. To keep going past a failure, the body uses `try ... else`, so each item's fallback is written next to it.
- **Budgets.** The statement's worst-case cost is the body's worst case times the number of items, which is known only at run time. `@budget` therefore checks before starting: if the list's length times the body's worst case exceeds what is left of the budget, the statement refuses to start, with an error naming both numbers. It never starts work it may not be able to pay for, and it never stops halfway because the money ran out. A list whose length is refined (`docs: List<Document> where docs.length() <= 100`) gets a static check instead.
- **Replay.** Each item's calls are recorded under its index, so a replay serves item 3's answers to item 3 whatever order the items finish in, and reproduces the result exactly.
- **The trace and lineage** show one `fan-out` span with a child per item, in input order, with each item's cost and time. That is the record `corvid graph` (B47) draws.

## Three tiers

- **Backend:** bounded concurrency, budget checked before start, replay by index.
- **Frontend:** a long fan-out inside an action reports progress as items finish (a later slice, with frontend development's progress events), so the UI can show "12 of 40" instead of a spinner.
- **AI:** an assistant calling an action that fans out pays for, and is approved for, the whole statement up front; it cannot set the limit, because the limit is in the source.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| `docs.map_parallel(fn (d) -> summarize(d), 8)` | Builtin methods are pure so replay stays deterministic, and this calls models and agents. A statement makes the effects visible where they happen. |
| A default limit, such as 10 | How hard to hit a provider and how fast to spend is a consequential choice; the source writes it. |
| Results in finishing order | The next line almost always pairs results with inputs. Input order removes that bookkeeping and keeps replay exact. |
| Keep going past failures by default | A silent partial result is the failure mode this replaces. `try ... else` in the body says, per item, what a failure becomes. |
| `for doc in docs, 8 at a time:` | Reads well, but hides that the items run concurrently; `parallel` is the word Corvid already uses for that. |

## Question for frontend development

1. `parallel for <name> in <expr>, at most <expr>:` begins a statement whose block's last expression is each item's value. Does your side parse statements inside agent and action bodies, and is this form safe for it?

## Slices

- [ ] FO-1. Grammar and checking: the statement, the required limit (1 to 256), the result list's type, refusal inside `atomic:`, and the budget rule.
- [ ] FO-2. The interpreter: bounded concurrency, input-ordered results, cooperative cancellation, the before-start budget check, per-item recording and replay.
- [ ] FO-3. The trace and lineage span, and the tour demo.
- [ ] FO-4. Native code, refused by name until then.

## Non-scope

- Streaming results as they finish, as a `Stream`, is a later slice.
- Fanning out across machines.
