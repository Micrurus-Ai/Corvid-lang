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
