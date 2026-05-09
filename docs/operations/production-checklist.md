# Production checklist

## Before you deploy

### Code

- [ ] `corvid check` clean.
- [ ] `corvid test` green.
- [ ] `corvid audit` reviewed; every reported finding either fixed or
      explicitly marked OutOfScope with a written reason.
- [ ] Every dangerous tool has an `approve` token in every reachable
      path.
- [ ] Every retrieval-backed value flows as `Grounded<T>`.
- [ ] Every agent has `@budget`, `@max_steps`, `@max_wall_time`.
- [ ] Every agent has `@replayable` if it should survive process
      restart.

### Build

- [ ] `corvid build --target=<your-target> --sign`.
- [ ] `corvid receipt verify <binary>` passes.
- [ ] `MANIFEST.toml` committed alongside the binary if using
      reproducible builds.

### Configuration

- [ ] `corvid doctor` green on the host.
- [ ] LLM provider key configured (or local model substrate
      configured).
- [ ] Replay storage configured with the right retention.
- [ ] Approval policy in `corvid.toml` matches the operator's
      expectations.
- [ ] OTel exporter pointed at your collector.
- [ ] CORS / rate-limit / body-limit configured for the server target.

### Persistence

- [ ] `corvid migrate status` shows no drift.
- [ ] `corvid migrate up` ran on the production database.
- [ ] Audit log table exists and is exercised by at least one test.
- [ ] Encrypted token storage key configured.

### Auth

- [ ] JWT verifier configured with a real JWKS URL.
- [ ] OAuth flows tested for every connector you use.
- [ ] Approval product surface accessible to operators.

### Observability

- [ ] OTel spans visible in your dashboard for a smoke-test request.
- [ ] Lineage graph queryable for a recent trace.
- [ ] Operator runbook reviewed for the agents you ship.
- [ ] On-call playbook updated with `corvid jobs explain` and
      `corvid replay` workflows.

### Backups & disaster recovery

- [ ] Database backup tested (full restore + migration).
- [ ] Replay store backup tested (replay a year-old trace).
- [ ] Signing-key rotation procedure documented.

### Compliance

- [ ] `docs/security/model.md` reviewed by your security team.
- [ ] Contract list (`corvid contract list --format=json`) reviewed
      against your compliance requirements.
- [ ] Retention policies set for traces, audit log, encrypted tokens.

## After you deploy

- [ ] First `corvid jobs run` worker pool stabilized at expected
      throughput.
- [ ] First batch of `corvid eval --swap-model` runs against a
      production-shaped traffic sample.
- [ ] First `await_approval` round-trip exercised with a real operator.
- [ ] First incident playbook walk-through.
