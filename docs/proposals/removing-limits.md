# Removing the limits

On 2026-11-26 the owner asked for every limit that is not needed to be removed. An inventory of `docs/backend-ai-roadmap.md` and `docs/proposals/` found about 120, in six kinds. This page sorts them into what goes, in what order and by whom, and what stays on purpose. The same day, the laws in `AGENTS.md` lost their pace limits and gained "Narrowing is temporary", so a limit kept for now is stated plainly and scheduled.

## What stays: safety refusals (28)

These remove failure modes, so under "Built for the hardest work on earth" they are the design, not limits:
- a `Secret` cannot be printed, stored, sent to a model or returned from a route;
- no hidden rounding: `Decimal` and `Float` never mix, and division names its rounding;
- no `30 days` duration, since a day is 23, 24 or 25 hours;
- strict Ed25519 verification;
- connectors must declare a timeout and a redirect policy;
- migrations never drop data or invent values;
- `atomic:` refuses what it cannot undo;
- linear-time regular expressions;
- nothing is exposed to an assistant unless the source says so.

Each still names what to do instead. Where a safety refusal blocks a real need, the answer is a safe form of the need, not removing the refusal. The `Secret` table column, for example, waits for an encrypted column rather than a plain one.

## What goes, in order

Ordered by what blocks a real backend and AI application most. Items marked *grammar* were proposed to frontend development more than one working day ago, so under the amended law they are built now and revised if frontend development objects.

| # | Limit removed | Owner | Kind |
| --- | --- | --- | --- |
| 1 | ~~Path parameters take only `Int`; route query fields of most types reach the handler as text.~~ Done 2026-11-26: one typed reader for both, through the public JSON decoder. | Windows session | type hole, narrowing |
| 2 | `try ... else`: a failing table, tool or prompt call stops the request instead of degrading. [Proposal](recoverable-failures.md) | Windows session | grammar |
| 3 | Shared actions (B10): actions are named by strings, with no derived endpoint, form, tool or CLI. The defining feature, and it blocks B11, B13 and RT-5. [Proposal](shared-actions.md) | Windows session | grammar |
| 4 | ~~JSON key renaming for camelCase APIs.~~ Done 2026-11-26: `@json(key:)` and `@json(case:)`, one key rule read by every public JSON surface on both tiers. [Proposal](json-field-keys.md) | Mac session | grammar |
| 5 | ~~Variant payload fields take refinements, as record fields do.~~ Done 2026-11-26. | Mac session | grammar |
| 6 | ~~The duration literal, `30 seconds`.~~ Done 2026-11-26, with the calendar-unit compile error. | Mac session | grammar |
| 7 | A connector call with the signed-in user's own credential (per-call bearer token). | Windows session | needs a proposal |
| 8 | Raw request bodies and a reusable webhook verifier, so payment and SaaS webhooks verify over HTTP (B16). | Windows session | needs a proposal |
| 9 | Joins and aggregates in table queries and `live`. | Windows session | needs a proposal |
| 10 | ~~The TS client reads served streams, and serve's stream `error` event sends a typed failure, not Rust debug text.~~ Done 2026-11-26 ([stream failures S2](stream-failures.md)): `chunk`, `completed` and a sanitized `failed`, read by the generated client's `api.route(...)`. | Mac session | frontend |
| 11 | `corvid serve` refuses `live` until RT-3 lands. Unblocked on 2026-11-26 (session watching, removal revisions). | Codex | frontend |
| 12 | Native gaps: streams, B06 types at tool and prompt boundaries (proposed, [native scalar boundaries](native-scalar-boundaries.md)), tables, `llm.usage` events in native runs. `find_regex` and `+=` on decimals and times landed on 2026-11-26, and a native prompt's `String` argument now renders as its JSON form, as on the interpreter (B1a); prompts exchange the public JSON form with a model on both tiers (B1), and native prompts take and return lists, records, `Option`, variants and text-held scalars through the public plan (B1b); and tools of the program's own run through the runtime call in native code, exchanging the public form on both tiers (B2); next is B3, native maps with text-held keys and values. | Mac session | native |
| 13 | Scale: incremental live matching, uploads held in memory, `verify_password` on the agent's thread. | Windows session | scale |

Items 1 to 6 come before the rest of the roadmap order, because each is a hole or a block a first real application hits. The roadmap records that amendment.

## Smaller fixes found by the inventory

Fixed with this page:
- Stale open checkboxes ticked: literals compared with a `TimeZone`, `Currency` or `Locale`, and `Some(10)` against an `Option<Decimal>`. Both were done by the comparison-literal rule on 2026-11-26.
- `decimal-and-time.md` said `Duration` and `Instant` were 64-bit, about ±292 years. Both are 128-bit nanoseconds, years 1 to 9999, as `corvid-time` holds them and `typed-persistence.md` says.
- `decimal-and-time.md` deferred byte encodings "until B04's byte type", which has shipped; they now wait only on an example.

## Left as planned work, with reasons

These are real work, not artificial limits, and stay on the roadmap with their reasons:
- MySQL, deferred because it cannot make a failed migration change nothing;
- private certificate authorities for Postgres;
- `Money` with its currency;
- plain dates;
- Ed25519 signing and encryption, which need key storage designed first;
- run-time regex patterns, which need a validated pattern type;
- cookie jars, proxies and mutual TLS in the HTTP client.

Each waits for an example that needs it, which "No ceiling" allows as long as the roadmap names it.
