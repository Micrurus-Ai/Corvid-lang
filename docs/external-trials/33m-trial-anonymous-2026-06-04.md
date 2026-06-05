# [Trial] anonymous-2026-06-04 — friends-and-family build, mostly blocked at the CLI/runtime boundary

> First trial report against the 33M friends-and-family round
> prompt at
> [`33m-friends-and-family-prompt.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-prompt.md).
> Reviewer's report verbatim below, plus the maintainer-side
> triage disposition per
> [`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md).
> Path-A timing posture preserved — the actual outreach window
> hasn't opened yet, so this is a single self-administered
> trial run that stress-tested the PROMPT itself, surfacing real
> friction before the prompt goes to hand-picked reviewers.

## Intake

- **Reviewer handle:** anonymous-2026-06-04
- **Version tested:** `corvid 0.0.1` (binary install at
  `~/.cargo/bin/corvid`; `CORVID_HOME=~/.corvid`). NOTE: this
  binary is **older than current `main` at HEAD** —
  verification below confirms several reviewer claims about
  missing surfaces are stale-binary issues that HEAD has fixed
  (`corvid serve`, `--cdylib`, `--explain-failures`, SBOM,
  hardcoded SBW staticlib path).
- **OS + shell:** Windows 11 Pro, PowerShell + git-bash.
- **Time spent:** ~half-day.

## What the reviewer built

- Use case: a tiny "preferences agent" — fetch a grounded
  preference, summarize it, and a dangerous `reset` that
  wipes a user's prefs behind an approval gate.
- Surfaces exercised (of the 6 the prompt names): 2 (migrations,
  partial), 3 (approve gate ✅), 4 (effects/budget ✅), 5
  (jobs queue mechanics), 6 (deploy package, partial). Surface
  1 (HTTP route) was reported as not reachable.
- `main.cor` ≈ 35 lines.
- Real provider keys: no (mock/interpreter only).

## Headline: the build-prompt's own "suggested build path" does not run as written

Reviewer found most copy-paste commands in the
`33m-friends-and-family-prompt.md` "Build path (suggested,
not required)" section do not match the actual CLI surface.
Maintainer verification at HEAD splits the claims into
**real prompt bugs**, **stale-binary mistakes**, and **real
code bugs**.

| Reviewer claim | Maintainer verification |
|---|---|
| `corvid new my_app --template backend` — no `--template` flag | **Confirmed prompt bug.** `corvid new <NAME>` only; scaffold is a hello-world `echo` agent. |
| `corvid serve` doesn't exist | **Stale binary.** `corvid serve` shipped in `9c2faf6` (slice E0-serve-2); this session added the HTTP approval queue (`2788490` E0-serve-5 + `3bb77e9` serve-6). HEAD ships it. |
| `corvid build --sign --key dev.key` — no `--key` flag | **Confirmed prompt bug.** `--sign <KEY_PATH>` takes the key path directly; `--key-id <ID>` is the keyid label. |
| `corvid deploy package . --cdylib ...` — `.` rejected; `--cdylib` doesn't exist | **Mixed.** `.` rejection is real (impl's `file_name().context()` check). `--cdylib <PATH>` flag DOES exist at HEAD. |
| `corvid jobs run --kill-after 2s some_job` — flag doesn't exist | **Confirmed prompt bug.** Real flag is `--max-runtime-ms`; `run` does not take a positional job name. |
| `corvid audit my_app` — directory rejected | **Confirmed prompt bug.** `audit <FILE>` takes the root Corvid source file, not a directory. |
| `corvid claim audit --explain-failures` — flag doesn't exist | **Stale binary.** `--explain-failures` shipped in `f3a8d0d` (43T claim-audit-explain-failures); HEAD ships it. |

## What worked (and worked well)

(Reviewer report kept verbatim — these are the moat moments the
trial confirmed:)

- **Approve-before-dangerous (Surface 3).** Calling a `dangerous`
  tool without `approve` fails to compile with a precise,
  copy-pasteable fix: `[E0101] dangerous tool 'delete_all_prefs'
  called without a prior 'approve'` … `Help: add 'approve
  DeleteAllPrefs(arg1)'`. Good moat moment — and it taught me
  the approve label is the PascalCase of the **tool** name,
  not an arbitrary effect label.
- **Compile-time budgets (Surface 4).** Tightening
  `@budget($0.01)` below the composed declared cost is
  rejected statically: `[E0250] effect constraint violated …
  static worst-case cost exceeds the declared budget`. This is
  the real thing and it's nice.
- **Migrations (Surface 2, half).** `corvid migrate up`
  applied two ordered `.sql` files, tracked sha256 checksums +
  drift, and **actually executed the SQL** — I verified real
  `prefs` and `audit_log` tables in `target/corvid.sqlite`.
- **Jobs queue mechanics (Surface 5, half).** `jobs enqueue` /
  `jobs run --workers N` / idempotency-key dedupe (duplicate
  enqueue returned the same `job_2`) / `dlq` / `checkpoint` /
  `drain` all exist and behave.

## What didn't (production-readiness ceiling)

(Reviewer report kept verbatim; maintainer notes inline:)

1. **You cannot actually run any non-trivial agent from the
   CLI.**
   - Native tier fails looking for `corvid_runtime.lib` at a
     hardcoded maintainer build path baked into the binary:
     `C:\Users\SBW\...\Documents\GitHub\corvid\target\release\corvid_runtime.lib`.
     **Maintainer note:** at HEAD this is fixed — `3f77ec1`
     retired the `C_RUNTIME_LIB_PATH` bake; `discover_staticlib`
     dynamically searches via `WalkExeAncestors`. But — a
     `cargo install --path crates/corvid-cli` from HEAD STILL
     doesn't produce a runnable native binary because cargo
     only emits the `staticlib` crate-type output when
     corvid-runtime is the build TARGET, not a dep (same
     constraint that bit the `effect-system-gates` workflow at
     `fcf4ce4`). **Filed as separate slice:**
     `35V2-P33-install-staticlib-fallback` — improve the
     diagnostic when staticlib is missing post-install, and
     fall back to interpreter for `--target=auto` with a clear
     notice.
   - Interpreter tier (`--target interp`) runs only **zero-arg**
     agents. Any agent that takes parameters: *"corvid run
     cannot supply them yet — use a runner binary that calls
     run_with_runtime with arguments."* So no realistic handler
     is invocable. **Filed as separate slice:**
     `35V2-P33-corvid-run-with-args` — add positional arg
     passthrough to `corvid run` for interpreter targets.
   - Multi-agent file: the runtime says *"pick one with
     `--agent`"*, but `--agent` is then rejected as an unknown
     argument. The flag the error tells you to use does not
     exist. **Filed as separate slice:**
     `35V2-P33-corvid-run-agent-flag` — either add the
     `--agent <NAME>` flag the diagnostic suggests, or change
     the diagnostic to name the actual disambiguation
     mechanism.

2. **Surface 1 (HTTP route) reported as unreachable.**
   **Stale binary.** `corvid serve` ships at HEAD; the prompt
   reviewer was on a binary predating slice E0-serve-2 closure.
   At HEAD: `corvid serve <app>/src/main.cor --listen
   127.0.0.1:8000` runs the in-process interpreter dispatcher
   for `server` block routes; the HTTP approval queue
   (E0-serve-5/6 this session) gives POST routes the async
   approval model with `/__approvals/*` admin endpoints. The
   prompt's "stress-test surface 1" instruction IS realistic
   against HEAD.

3. **Surface 6 deploy package ships a broken Dockerfile.**
   **Confirmed real CODE bug.** `render_dockerfile` at
   `crates/corvid-cli/src/deploy_cmd.rs:192-211` is hard-coded
   for the Corvid monorepo layout:
   - `RUN cargo build -p corvid-cli --release` (build context
     must be the whole compiler)
   - `COPY examples/backend/{app_name} examples/backend/{app_name}`
     and `COPY std std` (paths that don't exist in a standalone
     app)
   - `CMD ["run", "examples/backend/{app_name}/src/main.cor"]`
     hardcoded monorepo path

   Fixed in this triage commit — `render_dockerfile` rewritten
   to produce a standalone-app-compatible Dockerfile that
   pulls a published `corvid` binary into a distroless runtime
   layer + COPYs the user's app sources from the local working
   directory.

   `sbom.spdx.json` missing: **stale binary.** 43M (`a06f1fe`)
   shipped the SBOM emit; HEAD generates it at line 88 of
   `deploy_cmd.rs`.

4. **`std.db` / `std.http` / `std.jobs` are metadata-envelope
   libraries, not runtime bindings.** **Confirmed and known.**
   This is the Phase 37/38/41 audit posture documented at
   `docs/phases/phase-{37,38,41}-audit-2026-05-17.md`: the
   stdlib modules ARE envelope-shaped (descriptor structs,
   typed records) and the runtime bindings live in
   `corvid-runtime`. The launch-claim-audit's Section 4 rows
   point at the runtime tests not the stdlib because that's
   where the executing surface lives. The prompt's framing
   "Persistence through `std.db`" reads as ambiguous —
   reviewer correctly flagged this. **Filed as DOCS slice:**
   `35V2-P33-prompt-stdlib-framing` — clarify in the prompt
   that `std.db` is the typed envelope surface and the
   executing path is via the runtime + `corvid migrate up` /
   `corvid jobs run`, not directly via `std.db` calls in
   source.

## Honest moment / over-claim

(Reviewer:) The README's own **Status** section (line 488)
already discloses a 2026-04-29 internal audit: four
"phase-done" bullets in Phases 38–41 were "structurally
absent" — including the **multi-worker job runner +
crash-recovery** and **real auth/approvals**. The build prompt
asks reviewers to stress-test exactly those surfaces ("kill
the worker mid-step", approval gating) **without mentioning**
they're audit-correction tracks, and `jobs run`'s own help
admits it "runs the shipped **no-op executor**." So the
crash-mid-step + replay-determinism test the prompt requests
is not performable: there is no real job body to interrupt.

**Maintainer note (HEAD):** Phase 38 + 39 audit-correction
tracks closed via `35V2-P38-*` slices including
`t38l_d3_checkpoints_survive_unclean_shutdown` (the actual
crash-mid-step test) and the multi-worker pool / DST cron /
crash recovery slices 38K/38L/38M (closed 2026-05-XX). The
README's Status section IS out of date at the audit point the
reviewer cited; at HEAD those phase-done items are mechanically
ticked. **Filed as DOCS slice:**
`35V2-P33-readme-status-refresh` — refresh the README Status
section to reflect the closed audit-correction tracks instead
of the 2026-04-29 snapshot.

## Disposition

Per
[`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md):

**`code`** — fixed in this triage commit (`render_dockerfile`
rewritten for standalone-app layout) + 3 follow-up slices filed
(`35V2-P33-install-staticlib-fallback`,
`35V2-P33-corvid-run-with-args`,
`35V2-P33-corvid-run-agent-flag`).

**`docs`** — fixed in this triage commit
(`33m-friends-and-family-prompt.md` "Build path (suggested)"
section rewritten to use actual HEAD CLI signatures + version-
pin notice added) + 2 follow-up slices filed
(`35V2-P33-prompt-stdlib-framing`,
`35V2-P33-readme-status-refresh`).

**`test`** — 2 follow-up slices filed:
`35V2-P33-corvid-run-with-args-regression` (parameterized
agent run end-to-end) and `35V2-P33-deploy-dockerfile-builds`
(generated Dockerfile passes `docker build` from a standalone
app dir without the monorepo).

**`non-scope`** — none. Reviewer's "NON-SCOPE (but should be
labeled as such in the prompt)" framing of durable-job
crash-recovery and HTTP serving relies on the stale README
disclosure; at HEAD both surfaces are shipped and the prompt's
stress-test ask IS realistic.

## Closing

This trial was self-administered to stress-test the
33M build prompt itself before it goes to hand-picked
reviewers, and it caught five real prompt bugs + one real
code bug + a meta-finding (the prompt was written without
testing the commands against HEAD CLI). That's exactly the
shape of friction the friends-and-family round exists to
surface — and it confirms the round is worth running. The
remediation commits land 2026-06-04 alongside this report.

The hand-picked reviewer round at the actual Path-A timing
window will now run against a corrected prompt and a
standalone-app-buildable Dockerfile, so its findings will be
about the LANGUAGE not the prompt + the build wrappers.

---

## Round 2 (2026-06-05) — verbatim

The reviewer retested against `corvid 0.0.1 (5c8a0db, 2026-06-05)`
— nightly `corvid-x86_64-pc-windows-msvc.zip` from
`nightly-2026-06-05-5c8a0db`, installed side-by-side. Windows 11,
x86_64, git-bash. Time spent: roughly half a day across the
original + two retries.

> Context: The rev-2026-06-05 install path works on Windows now,
> and the five prior fixes (`corvid serve`, `new` vendoring
> `src/std/`, `deploy --cdylib`, `sbom.spdx.json`, the
> de-monorepo'd Dockerfile) are real and verified. This issue is
> the **next layer**: five things that still stop a reviewer
> from shipping the six-surface app for real. Ordered by
> severity. Each has exact repro against the SHA above.

### P1 — CODE: approval-gated dangerous route can never complete over `corvid serve`, and the approval is burned on failure

The headline moat demo (approve-before-dangerous over HTTP) gets
you the `202` and the queued approval, but approving it can
never run the side effect, and the approval is consumed anyway.

Repro (`server` block with a POST route whose handler calls a
`dangerous` tool):

```
POST /prefs/reset  {"user_id":"u-99"}      -> 202 {"approval_id": "...","status":"pending"}
GET  /__approvals                          -> 200 lists it (action: DeleteAllPrefs)
POST /__approvals/<id>/approve             -> 500 {"error":"approved_execution_failed",
                                                   "detail":"no handler registered for tool `delete_all_prefs`"}
GET  /__approvals                          -> 200 {"approvals":[]}   # consumed despite the 500
```

- `serve --help` exposes no tool-handler option (no
  `--with-tools-lib`), and adding the tool to `tools.py` does
  not help — the interpreter serve path doesn't load it.
- Two distinct asks:
  1. Give `serve` a way to register tool handlers (a
     `--with-tools-lib` parity flag, or load `tools.py`, or
     document the intended mechanism). Without it, Surface 3
     (approval-gated dangerous tool) is undemonstrable over
     HTTP — which is the surface the trial most wants
     exercised.
  2. **Do not consume the approval when approved-execution
     fails.** A 500 should leave the approval pending (or move
     it to a retryable/failed state with the original
     invocation intact), not silently drop it. Right now a
     transient handler failure permanently burns a human
     approval.

### P2 — CODE/DOCS: `@trust(...)` is incompatible with `corvid build --sign`

```
corvid build src/main.cor --target=cdylib --sign dev.key
# error: `corvid build --sign` refused ... agent `execute_reset` declares `@trust(...)`,
#        but no signed cdylib guarantee id covers that effect constraint yet
```

`claim --explain` confirms there is no `trust` guarantee id
among `enforced_guarantees`. So the trust moat and the signed-
deploy path are mutually exclusive — I had to delete `@trust`
to produce the deploy cdylib the build path asks for. Either
register a `trust.*` guarantee id so `@trust` can be signed,
or document that `@trust` must be omitted from cdylib-targeted
agents (and say why).

### P3 — CODE/DOCS: generated deploy Dockerfile won't `docker build` for a fresh app

From `corvid deploy package "$(pwd)" --out deploy/ --cdylib ...`:

- It unconditionally `COPY src`, `COPY corvid.toml`,
  **`COPY migrations`, `COPY evals`, `COPY traces`** — but a
  `corvid new` app has none of `migrations/`, `evals/`,
  `traces/`, so the build fails at the first missing path.
  (The comment hand-waves `evals`/`traces` but not
  `migrations`; shipping a Dockerfile that needs hand-editing
  to build is the trap.) It also never `COPY`s `tools.py`.
- Default `ARG CORVID_VERSION=latest` → `releases/latest/
  download/...` = **v0.1.0** (nightlies are prereleases,
  excluded from "latest"), and **v0.1.0 has no `serve`** —
  yet the image `CMD` is `serve …`. The default image's
  entrypoint is a command its own binary lacks.
- The build-path instruction `cd deploy && docker build .` is
  wrong: the COPY paths are relative to the **app root**, but
  `deploy/` only contains generated artifacts. Correct form is
  `docker build -f deploy/Dockerfile .` from the app root.
- Asks: make the optional `COPY` lines conditional (or only
  emit them for dirs that exist); copy `tools.py` when
  present; default `CORVID_VERSION` to a version that actually
  has `serve` (or fail the build if `latest` lacks the `CMD`
  subcommand); fix the documented `docker build` invocation. A
  CI gate that `docker build`s the generated Dockerfile from a
  bare `corvid new` app would have caught all of these (you
  already filed `35V2-P33-deploy-dockerfile-builds` — this is
  its acceptance test).

### P4 — DOCS: `pub extern "c"` requirement for cdylib is undocumented

```
corvid build src/main.cor --target=cdylib --sign dev.key
# error: library targets require at least one `pub extern "c"` agent
```

The build path's cdylib step never mentions this. Add `pub
extern "c"` to the build-path example (and ideally have the
error name a doc page on the exported-ABI surface).

### P5 — DOCS: `corvid claim audit --explain-failures` can't run in a standalone app

```
corvid claim audit --explain-failures
# error: read claim inventory `docs/meta/launch-claim-audit.md`: cannot find the path
```

That inventory is a repo-internal file. The build path lists
this command in the app-dir context (step 10), where it always
errors. Either drop it from the app-dir build path, or have
it no-op gracefully with a clear message when no inventory is
present.

### Minor

- `corvid --version` still reports semver `0.0.1` (now with
  `(sha, date)`). A toolchain this mature self-reporting
  `0.0.1` reads oddly to a new reviewer.

### What's working (so the signal isn't all negative)

- `serve` GET + typed-JSON-body POST routing; the `202` +
  `/__approvals` queue flow.
- `new` auto-vendoring `src/std/`; `migrate up` executing real
  SQL; `jobs enqueue/run`.
- `deploy --cdylib` emitting `sbom.spdx.json` + an attestation
  that binds `cdylib_sha256` (`chain_status: complete`);
  `claim --explain` verifying source↔descriptor SHA agreement
  and enumerating enforced guarantees. These are genuinely
  good.

### Reviewer's suggested disposition

- P1: CODE (general) ×2 — serve tool handlers; approval-not-
  burned-on-failure.
- P2: CODE (general) or DOCS — register `trust.*` guarantee,
  or document the exclusion.
- P3: CODE (general) — Dockerfile renderer + the existing
  `35V2-P33-deploy-dockerfile-builds` gate.
- P4, P5: DOCS — build-path corrections.

### Repro harness

> The two apps I used are standalone and reproduce all of the
> above: `prefs_api/src/main.cor` (server block, approval-
> gated POST) and the round-1 `prefs_agent/`. Happy to attach
> them or open a draft PR for the DOCS items (P3/P4/P5) if
> that's useful.

---

## Round 2 maintainer triage (2026-06-05)

Per the [`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md)
disposition shape; closing criterion at
[`ROADMAP.md L51`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md#L51).
Each finding closes as `code` / `docs` / `test` / `non-scope`
before the v1.0 cut.

| Finding | Class | Disposition | Owning slice |
|---------|-------|-------------|--------------|
| **P1.1** `serve` has no tool-handler registration; Surface 3 undemonstrable over HTTP | code | ROADMAP slice **33Q1 — `serve --with-tools-lib`** | New: parity with the `build --target=cdylib` linkage for the interpreter `serve` path. Needs design pass (subprocess `tools.py` loader vs explicit `--with-tools-lib` flag vs both). |
| **P1.2** Approval is consumed when approved-execution fails | code | ROADMAP slice **33Q2 — approval-not-burned-on-failure** | New: a 500 from the handler must leave the approval in `pending` (or a new `failed-retryable` state with the original invocation captured), not silently consume it. Adversarial: this is approval-budget integrity, not just UX. |
| **P2** `@trust(...)` incompatible with `corvid build --sign` | code | ROADMAP slice **33Q3 — `trust.*` guarantee registration** | New: register a `trust.*` row in `GUARANTEE_REGISTRY` (`RuntimeChecked`) so `@trust` annotations participate in the signed-cdylib claim. Surfaces the trust moat in `claim --explain`. |
| **P3.a** Dockerfile unconditionally COPYs `migrations/`, `evals/`, `traces/`; never COPYs `tools.py` | code | ROADMAP slice **33Q4 — Dockerfile renderer presence-conditional COPYs** | New: emit `COPY` lines only for paths that exist at render time; emit `COPY tools.py` when present. Acceptance: bare `corvid new my_app` → `corvid deploy package` → `docker build` succeeds. |
| **P3.b** Default `CORVID_VERSION=latest` resolves to v0.1.0 which lacks `serve` subcommand | code | ROADMAP slice **33Q5 — Dockerfile CORVID_VERSION default** | New: pin to the SHA the package was generated against (`ARG CORVID_VERSION=<sha>` with the literal SHA from `corvid --version`), so the rendered image's `CMD` matches the binary's CLI surface. Alternative: emit `CORVID_VERSION=nightly` (resolves the most recent nightly which has `serve`). Default to the SHA-pin since it's reproducible. |
| **P3.c** Build-path doc says `cd deploy && docker build .` (wrong; COPY paths are app-root-relative) | docs | This slice — prompt fix | Correct to `docker build -f deploy/Dockerfile .` from the app root. |
| **P4** `pub extern "c"` requirement for cdylib undocumented in the build path | docs | This slice — prompt fix | Add to step 7 of the build path; cdylib `--target` example shows a `pub extern "c"` agent. |
| **P5** `corvid claim audit --explain-failures` is repo-internal but listed in the app-dir build path | docs | This slice — prompt fix | Drop from step 10; that command belongs to the maintainer-side launch audit, not the app-dir trial flow. |
| **Minor** `corvid --version` reports `0.0.1` | non-scope (signal noted) | Pre-v1.0 honest versioning; the bump to `0.1.0`/`1.0.0` happens at the actual v1.0 cut. Documented as expected. | Reviewer's signal is real — added a one-liner to the prompt's "Things to know" block so future reviewers see "pre-v1.0 versioning is intentional." |
| **Repro-harness offer (prefs_api + prefs_agent)** | code | We'd take the draft PR for P3/P4/P5 if offered; the prefs_api app shape is also useful as a small standalone-app test fixture for the 33Q4 `docker build` CI gate. | — |

### Slice dispatch

- **This commit (docs slice):** records the report verbatim,
  files the triage table, and queues the next commits.
- **Next commit (33m-prompt-fixes):** P3.c + P4 + P5 + the
  Minor-versioning-note edit, all in
  [`33m-friends-and-family-prompt.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/33m-friends-and-family-prompt.md).
- **Subsequent commits (ROADMAP entries):** file 33Q1-33Q5 as
  five new sub-slices under a new `33Q-trial-round-2-fixes`
  block in ROADMAP. Each carries the reviewer-named acceptance
  criterion. Code work follows per slice discipline.

### Why this round is high-signal

Round-1 surfaced wrappers-and-onboarding bugs. Round 2 — same
reviewer, same hand-built app shape, retest against a polished
install pipeline — surfaced **language-and-runtime** bugs: an
approval can be silently burned on handler failure (an
approval-integrity bug, not just UX), the trust moat is
mutually exclusive with the signed-deploy path, the
auto-generated Dockerfile is broken for the canonical
`corvid new` shape. None of these are findable from the
maintainer side because we always test against the monorepo
where `migrations/` + `evals/` + `traces/` always exist and
the demo apps don't exercise `@trust` + signed-cdylib together.

This is the round-trip the 33M friends-and-family round
exists for. The signal density is now closer to the language
surface we want pre-launch reviewers stress-testing; the
33Q* slices will be its acceptance criteria.
