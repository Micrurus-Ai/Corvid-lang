# Personal Knowledge Agent — per-app side-by-side

## Headline

For a multi-tenant knowledge agent that ingests documents with
preserved provenance, answers questions with `Grounded<T>` citations,
and gates five external-write surfaces (share-to-chat, share-via-email,
publish-authoritative-answer, export-tenant-corpus,
cross-tenant-index-share) behind typed approvals, Corvid declares the
governance in the language and rejects any ungated dangerous call at
typecheck. The FastAPI + LangChain and Next.js + Vercel-AI-SDK baselines
assemble the same grounding, approval, and audit guarantees across
separate libraries with hand-written glue.

## Reproduce

Corvid governance surface lives in
[`examples/backend/personal_knowledge_agent/src/main.cor`](../../examples/backend/personal_knowledge_agent/src/main.cor).
The governance-line subtotal is countable by inspection:

```bash
m=examples/backend/personal_knowledge_agent/src/main.cor
grep -c 'trust: human_required' $m   # external-write effects (×4 lines each)
grep -c 'dangerous uses' $m          # dangerous tool declarations
grep -c '^    approve ' $m           # compiler-enforced approve gates
grep -c 'agent permission_for_' $m   # typed permission per dangerous tool
```

The serve smoke (`cargo test -p corvid-cli --test serve_smoke`) proves
the same source runs as an HTTP service; orchestration-latency figures
reference the capability benchmarks in
[`jobs_durability.md`](./jobs_durability.md) and
[`observability.md`](./observability.md) rather than restating them.

## Side-by-side (sketch)

### Corvid

```corvid
effect cross_tenant_share:
    cost: $0.03
    trust: human_required
    data: external

tool cross_tenant_index_share(req: CrossTenantIndexShareRequest)
    -> CrossTenantIndexShareReceipt dangerous uses cross_tenant_share

agent cross_tenant_index_share_approval_contract(
    source_tenant_id: String, target_tenant_id: String, trace_id: String
) -> ApprovalContractRef:
    return approval_contract_ref(
        "cross-tenant-share:" + target_tenant_id, "v1",
        "CrossTenantIndexShare", "tenant_index_share", target_tenant_id,
        source_tenant_id, "Admin", 0.25, "external", true,
        4102444800000, "approval:cross_tenant_share:" + trace_id)

agent execute_approved_cross_tenant_index_share(req: CrossTenantIndexShareRequest)
    -> CrossTenantIndexShareReceipt uses cross_tenant_share:
    approve CrossTenantIndexShare(req)
    return cross_tenant_index_share(req)
```

The compiler rejects any reachable path to `cross_tenant_index_share`
that lacks a prior `approve CrossTenantIndexShare(...)` (error `E0101`,
proven by `adversarial/ungated_cross_tenant_share.cor`). The
`Grounded<T>` answer chain is enforced by the effect system: an answer
that strips its citation provenance fails the grounding check.

### Python (FastAPI + LangChain + custom approval) — bounty-open

Idiomatic stack: FastAPI routes; LangChain for retrieval + the answer
chain; a Pydantic `ApprovalContract` model + an in-app policy clause;
SQLAlchemy `approvals` / `audit_events` tables; an explicit middleware
that records the audit row and checks the approval before the
write. The "every external-write path has an approval boundary" and
"every answer carries citation provenance" invariants are runtime
conventions, not type-system facts. Submission lands under
`runs/python/personal_knowledge_agent/`.

### TypeScript (Next.js + Vercel AI SDK + zod) — bounty-open

Next.js route handlers; Vercel AI SDK for the answer; zod schemas for
the request/receipt + a hand-rolled approval contract; Drizzle/Prisma
for the approval + audit tables; the cross-tenant isolation + grounding
checks are TypeScript code. Submission lands under
`runs/typescript/personal_knowledge_agent/`.

## Governance line count

Counted by inspection of `src/main.cor` (the constructs that exist
solely for approval / effect-trust / provenance / typed-permission):

| Implementation | Governance surface | Governance lines | Notes |
|---|---|---|---|
| Corvid | 5 human_required effects, 5 dangerous tools, 5 approve gates, 5 approval contracts, 5 typed permissions, `approval_surface_valid` | ~88 | every line is language-level + compiler-checked |
| Python (FastAPI + LangChain) | same intent across Pydantic + middleware + SQLAlchemy | bounty-open | reachability + grounding are runtime conventions |
| TypeScript (Next.js + Vercel AI SDK) | same intent across zod + handlers + ORM | bounty-open | reachability + grounding are runtime conventions |

## What Corvid wins on

- **Reachability at typecheck.** A route or job that reaches any of the
  5 dangerous tools without a matching `approve` fails to compile
  (`E0101`); the 5 `ungated_*.cor` adversarial fixtures pin this.
- **Provenance as a type.** `Grounded<T>` makes "this answer cites a
  real source" a type-level invariant; the baselines re-check it at
  runtime if at all.
- **Cross-tenant isolation** is a typed boundary (`cross_tenant_share`
  is the only effect that crosses tenants, and it is approval-gated),
  not a query-filter convention.
- **One source, three artifacts.** The same `main.cor` produces the
  HTTP service (`corvid serve`), the durable-job worker
  (`corvid jobs run`), and the eval/replay corpus.

## What Corvid does not claim

- **LangChain's retriever/connector catalog** is broader; Corvid does
  not ship every vector store or loader on day one.
- **Raw retrieval/serving throughput** is not the moat; the comparison
  is governance line count + static rejection of ungated writes +
  provenance typing.
- **The `bounty-open` cells are not yet measured.** They remain
  `bounty-open` until a real submission lands, per the directory's
  honesty rules.
