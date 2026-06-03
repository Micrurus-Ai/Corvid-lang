# Corvid Developer Production Guide

This is the canonical path for shipping a Corvid backend in production. It assumes you want a real service — durable migrations, approval-gated dangerous work, signed deployment artifacts, observability that survives an audit, and a release pipeline you can hand to someone else next quarter.

If you are evaluating Corvid for the first time, read [`docs/README.md`](README.md) and the [invention catalog](reference/inventions.md) first. This guide is for the engineer who has decided to ship.

> **Reading order.** Each section here is the *shipping summary*. Deep mechanics live in the per-topic guides under [`docs/guides/`](guides/) — backend, connectors, observability, persistence, jobs, auth, performance. This guide tells you what good looks like and links you to the rest.

---

## Backend Tutorial

The backend tier renders a production-shape `axum` 0.7 binary from a Corvid `server` declaration. Every example under [`examples/backend/`](../examples/backend/) is a working app you can copy. The canonical happy path:

1. Create or open an app under `examples/backend/<your_app>/`.
2. Put service source in `src/<your_app>.cor`. Define effects *before* tools and routes — the effect row is what the typechecker uses to gate `dangerous`, `approve`, and `replay` requirements.
3. Use `server Name:` with route declarations for HTTP entrypoints. Each route's effect row drives the middleware (body-limit 413, isolation timeout 504, dangerous→auth-permission check) automatically.
4. Migrations live under `migrations/`. Each `.sql` is checksum-pinned and applied in lex order.
5. Run the production-shaped checks:

```bash
corvid check examples/backend/refund_api/src/refund_api.cor
corvid build examples/backend/refund_api/src/refund_api.cor --target=server
corvid migrate status --dir examples/backend/refund_api/migrations
corvid deploy package examples/backend/refund_api --out target/refund_api-package
```

`corvid check` validates effect rows, approval coverage, migration manifest integrity, connector usage, and source-level claim coverage in one pass. `corvid build --target=server` emits the deployable binary plus a `manifest.json` capturing the source SHA, descriptor hash, effect ledger, and embedded migration checksums. `corvid deploy package` wraps that binary with the env schema, healthz/readyz config, durable-job schedule manifest, and a DSSE attestation.

**What you get out of the box.** Generated `/healthz` and `/readyz`, tracing-header propagation, graceful shutdown via tokio oneshot, per-route effect enforcement, env-schema validation at startup, and a deployment artifact you can hand to Kubernetes, Compose, or systemd unchanged. See [`docs/guides/backend.md`](guides/backend.md) for the route-by-route mechanics.

**Common pitfalls.**

- Forgetting to declare `effect retrieval:` before using `Grounded<T>` returns — the typechecker rejects the build with a pointer at the missing effect row.
- Storing migration `.sql` files outside the `migrations/` directory — they are silently ignored. `corvid migrate status` lists exactly which files it discovered.
- Using `panic!`/`unwrap` in tool implementations — those abort the whole isolation timeout and surface as 504 with no audit trail. Return `Result` and let the runtime surface the error through the approval/observability path.

---

## Personal Executive Agent Tutorial

The Personal Executive Agent under [`examples/backend/personal_executive_agent/`](../examples/backend/personal_executive_agent/) is the reference app for high-value personal-agent workflows:

- daily brief generation,
- meeting prep,
- inbox triage,
- follow-up drafting,
- calendar scheduling,
- task updates,
- approval-gated external writes.

It exists because the obvious version (an LLM with a few API calls) is the one that paged on-call at 2am. The Corvid version enforces every external write through an approval boundary, replays every decision against a trace, and ships with a hardening eval that fails the build if dangerous work loses its `approve` gate.

Run the production-shaped checks:

```bash
corvid check examples/backend/personal_executive_agent/src/main.cor
corvid audit examples/backend/personal_executive_agent/src/main.cor --json
corvid deploy compose examples/backend/personal_executive_agent --out target/pea-compose
corvid deploy k8s examples/backend/personal_executive_agent --out target/pea-k8s
corvid eval examples/backend/personal_executive_agent/evals/hardening_eval.cor
```

External write tools are marked `dangerous` and must sit behind approval routes. Durable schedules are declared in source (not in a runbook) and are included in signed claim coverage. `corvid audit --json` emits a machine-readable evidence ledger: every `dangerous` tool, its approval predicate, the `approve` site in source, and the trace event the runtime emits when the predicate fires.

**Why use this app as a template.** It's not a tutorial; it's a production app with seeds, mocks, evals, deploy manifests, security model, adversarial-fixture coverage, and a `CLAIM.md` of runnable claims. Fork the directory, rename the agents, swap your connectors. Every file you touch has a test that fails if you remove the production discipline.

---

## Connector Guide

Connector manifests are the contract between your Corvid app and the world outside it. Every shipped manifest must declare:

- **Provider scope.** Which OAuth scopes / API tokens the connector requires. Minimal scopes only; the `corvid connectors check` linter flags overscoped manifests.
- **Data classes.** What sensitive classes the connector touches (`pii`, `financial`, `health`, ...). The runtime checks these against the agent's `data:` effect dimension at every call.
- **Approval requirement for writes.** Each `write_*` operation declares the approval predicate. `corvid connectors check` refuses to ship a write operation without one.
- **Replay policy.** What the connector returns in replay mode — captured response, stub, or refused. The default is "refused" so you can't accidentally ship a connector with no replay path.
- **Rate limits.** Per-tenant token bucket; the runtime enforces these without your code seeing the failures unless you opt in.
- **Sensitive redaction rules.** What fields get redacted before they reach traces or logs. The runtime fail-closes if a redaction rule is missing for a declared `pii`/`financial`/`health` field.

Use mock or replay mode during development:

```bash
corvid connectors list                 # show every manifest
corvid connectors check                # validate every manifest, exits non-zero on the first invalid one
corvid connectors run gmail send-email --mock '{"to":"a@b","subject":"hi"}'
```

Move to real provider mode only after scope minimization, write approval, webhook signature verification, and rate-limit behavior have explicit tests — or explicit non-scope notes filed in `docs/phases/`. See [`docs/guides/connectors.md`](guides/connectors.md) for manifest schema and webhook contract.

---

## Approval Guide

Use approvals for every external write, money movement, irreversible message, data deletion, or privileged tenant action. The rule is a *positive* one: if the action affects something outside the calling process and a reasonable human would want to see it before it lands, it goes through `approve`.

Production approval surfaces need:

- **A typed approval request.** Declared with `request ActionName:` in source, not constructed ad-hoc. The type makes the predicate auditable.
- **A `dangerous` tool.** The tool implementation is what the runtime *would* run, but only after `approve` resolves.
- **An `approve` boundary.** The agent function calls `approve action with predicate`. The typechecker rejects a `dangerous` call that isn't behind one.
- **An audit record.** Every approval emits an `ApprovalRequest` trace event with the proposed action JSON, the predicate evaluation, the actor, and the decision.
- **A denial path.** Your agent must handle `ApprovalDenied` gracefully — refusing to write isn't an error, it's a state.
- **An expiry path.** Approvals have a TTL. Expired approvals do not become silently accepted.
- **A replay key.** Trace events carry a replay key so an auditor can re-derive the decision from the recorded inputs without running the agent live.

The approval route should return the proposed action and the evidence supporting it — not execute the action and ask permission afterward. The reference apps under [`examples/backend/`](../examples/backend/) demonstrate the `request` / `dangerous` / `approve` / audit pattern end-to-end.

**The "trace id" contract.** Every approval lands in a single trace identified by its `trace id`. The auditor needs only the trace id to find the request, the predicate, the decision, the actor, and any subsequent state changes. If a `dangerous` action lands in production without a trace id you can hand to the auditor, the build was misconfigured — `corvid audit` would have caught it.

---

## Observability Guide

Production services should emit:

- **request id** — propagated from the inbound header (`x-request-id`) or generated; flows into every downstream trace event.
- **trace id** — the run-level identifier the auditor reaches for; one per agent invocation.
- **route or job name** — the source-level symbol that fired, so the trace is greppable by the same name the developer sees.
- **effect names** — the resolved effect row, so an auditor can answer "which dangerous tools fired" without parsing the agent body.
- **approval status** — pending / accepted / denied / expired / replay-accepted, with the predicate that decided.
- **cost and token counters when LLMs are used** — both budget-bound (declared) and actual.
- **replay key** — the deterministic key that lets you re-derive this run.
- **connector mode** — `live` / `mock` / `replay` per connector touched. Mismatches between dev and prod mode are the most common cause of "it worked on staging."
- **migration state** — applied migration head at run start. A mismatched head fails the readiness probe.

Use traces for replay and claim audit. The replay engine round-trips every trace through the agent and verifies that the rendered output matches byte-for-byte (after timestamp / line-ending normalization). A diverged replay is the strongest signal that production has drifted from the source.

**Do not** log plaintext API keys, connector tokens, approval secrets, or raw `pii`/`financial`/`health`-class data without redaction. The runtime's redaction layer fails closed: a missing redaction rule for a declared sensitive class refuses to start the service. If you see a startup error mentioning `data_class_redaction_unspecified`, that's the safety net. Don't bypass it — declare the redaction rule.

See [`docs/guides/observability.md`](guides/observability.md) for trace event schema, OpenTelemetry export wiring, and the conformance suite under [`docs/operations/observability-conformance.md`](operations/observability-conformance.md).

---

## Production Checklist

Before shipping a Corvid backend, every one of these must be green:

- [ ] `corvid check` passes against the source.
- [ ] Migrations run and `corvid migrate status` reports drift-free.
- [ ] `corvid upgrade check` is clean — no pending stdlib migration is required.
- [ ] Every `dangerous` tool has at least one `approve` predicate gating it (`corvid audit --json` is the machine-checked evidence).
- [ ] `corvid deploy package` produced a deployment artifact for your target runtime.
- [ ] Compose / PaaS / Kubernetes / systemd manifest exists at the path the deploy package references.
- [ ] Env schema is complete; the service refuses to start when a declared var is missing.
- [ ] `/healthz` and `/readyz` are wired and tested with the migration head.
- [ ] Connector mode is explicit per environment (`live` / `mock` / `replay`), not implied by an env var fallback.
- [ ] Traces are enabled, or explicitly disabled in source with a documented reason.
- [ ] Release artifacts are signed (DSSE) and the verifying key is published.
- [ ] Launch claims under your app's `CLAIM.md` have runnable evidence — every line has a command an auditor can run.

The reference apps under `examples/backend/` ship with this checklist completed. Use them as the template, not as inspiration.

---

## No-Prototype Rule

A Corvid app is not production-shaped until it has *all* of: source checks, migrations, deployment artifacts, operational docs, approval boundaries for every dangerous tool, and a claim / audit path. Demo-only mocks are allowed only when they are clearly isolated from production connector mode — the runtime refuses to run a `live`-mode connector when a `mock`-mode dependency is in the dependency graph.

The rule exists because the alternative — "ship now, harden later" — is how the obvious version of every agent app pages on-call at 2am. Corvid's design takes the position that *harden later* is not a real plan, and the toolchain reflects it: the production checks are not optional, and the build fails when you skip one. If you find a `# TODO: approval gate before launch` in a Corvid agent body, the typechecker has already rejected it.

This is the cost of having a language that knows what `dangerous` means. It's also the reason the resulting service survives the audit.

---

## Further reading

- [`docs/guides/backend.md`](guides/backend.md) — route shapes, middleware pipeline, body-limit and isolation-timeout mechanics.
- [`docs/guides/auth.md`](guides/auth.md) — auth context, permission tokens, dangerous-route gating.
- [`docs/guides/connectors.md`](guides/connectors.md) — manifest schema, webhook signatures, replay policies.
- [`docs/guides/observability.md`](guides/observability.md) — trace event schema, OTLP export, redaction layer.
- [`docs/guides/persistence.md`](guides/persistence.md) — typed stores, migrations, store policies.
- [`docs/guides/jobs.md`](guides/jobs.md) — durable schedules, job replay, jitter policy.
- [`docs/maintainer-runbooks.md`](maintainer-runbooks.md) — on-call procedures, release checklist, key rotation, rollback.
- [`docs/operations/production-checklist.md`](operations/production-checklist.md) — pre-merge checklist (operator-facing companion to this one).
- [`docs/operations/receipts-and-signing.md`](operations/receipts-and-signing.md) — DSSE attestation chain and key publication.
- [`docs/reference/inventions.md`](reference/inventions.md) — what each Corvid invention promises and how to prove it.
