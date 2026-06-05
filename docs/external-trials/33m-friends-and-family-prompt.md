# Corvid v1.0 Friends-and-Family Round — Build Prompt

> **Slice:** `33M` (repositioned, Path A) — final 4 weeks of Phase 43.
> **Target reviewers:** 5-10 hand-picked AI engineers.
> **Output:** one report per reviewer, triaged through
> [`docs/external-trials/phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md),
> closing as `code` / `docs` / `test` / `non-scope`.
>
> **Status (rev 2026-06-05):** prior trial (`anonymous-2026-06-04`)
> closed; five bugs they surfaced have all shipped fixes (
> [`1455b6c`](https://github.com/Micrurus-Ai/Corvid-lang/commit/1455b6c),
> [`e8efa23`](https://github.com/Micrurus-Ai/Corvid-lang/commit/e8efa23),
> [`7b92e90`](https://github.com/Micrurus-Ai/Corvid-lang/commit/7b92e90)).
> Build path below has been verified against HEAD; report any
> mismatch as a `[Blocked]` issue.

---

## What we're asking you to do

You've been hand-picked (not a public beta) to build a **small
production-shape AI backend app in Corvid v1.0** and report back
on whether the language holds up under your own hands.

"Small production-shape" means at minimum these six surfaces —
not because they're impressive on their own, but because they're
the surfaces production AI apps ALWAYS need and Corvid claims to
make first-class:

1. **At least one HTTP route** that handles a real request body
   (typed, JSON-deserialized).
2. **Persistence** through `std.db` — at least 2 tables and one
   migration applied through `corvid migrate up`.
3. **At least one approval-gated tool** that crosses an
   `approve` boundary before a dangerous side effect.
4. **At least one `effect` declaration** with declared
   dimensions (cost, trust, data) and a typed `uses` clause on
   the agent that consumes the effect.
5. **At least one durable job** with retry policy and replay
   determinism — and please try killing the worker mid-step at
   least once to see what happens.
6. **A deploy package** produced via `corvid deploy package
   <app>/ --out <dir>`, with the resulting `Dockerfile` +
   `oci-labels.json` + `sbom.spdx.json` +
   `build-attestation.dsse.json` artifacts inspected.

If you want a template, the 5 shipped reference apps are at
[`examples/backend/`](https://github.com/Micrurus-Ai/Corvid-lang/tree/main/examples/backend).
They're production-maturity-bar-closed — copy whichever one is
closest to your use case, then carve it down to a 100-300-line
app of your own. Don't try to ship a reference-app-scale 600+
-line surface; we explicitly want the **small** end of
production-shape.

## What we want you to stress-test

We don't need you to verify the language works as advertised —
the proof matrix at
[`docs/reference/inventions.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/inventions.md#proof-matrix)
already cross-references every shipped invention to a test file.
What we DO want is the friction nobody on the maintainer side
can see anymore:

- **The first 30 minutes.** Where did you get stuck? Which
  error message confused you? Which doc page did you wish
  existed? Which CLI command did you guess at and miss?
- **The diagnostic experience.** When you wrote something
  that didn't typecheck, did the compiler's error point you at
  the actual cause? Did it suggest a fix that worked? Did the
  diagnostic refer to a concept the docs explained anywhere?
- **The moat under your own use case.** Did
  approve-before-dangerous catch a mistake YOU were about to
  make? Did `Grounded<T>` make a citation requirement visible
  that your previous stack hid? Did a compile-time budget bound
  surface a cost-overrun risk you hadn't planned for? Or did
  the moat feel like ceremony that didn't pay off for your
  app's shape?
- **The production-readiness ceiling.** If you wanted to put
  this app into a real environment tomorrow, what's the first
  thing that would stop you? Authentication that doesn't fit
  your IdP? A connector your business depends on that doesn't
  exist? A deploy target that isn't supported? An ops
  introspection surface that's missing?
- **The honest moments.** Were there places where Corvid
  over-claimed? Where a docs page said something Corvid almost
  does? Where a reference app demonstrated a pattern that
  worked in the demo but felt fragile under your variations?

## What you do NOT need to do

- You do NOT need to make your app public or share its source.
- You do NOT need to use a real LLM provider — every adapter
  ships with a mock mode by default
  (`CORVID_PROVIDER_LIVE=1` plus a per-provider key flips real
  mode). The mock-mode build path is the same one CI uses.
- You do NOT need to ship signed binaries or deploy to a real
  PaaS — running `corvid deploy package` and inspecting the
  artifacts is enough.
- You do NOT need to file every concern as a separate report.
  One report, in your own words, is what we're after.

## Setup links

| What you need | Where to get it |
|---|---|
| Install `corvid` (stable) | <https://github.com/Micrurus-Ai/Corvid-lang/tree/main/install> — `curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh \| sh` (Unix) or the `install.ps1` in the same directory (Windows). Linux+macOS targets: `{x86_64,aarch64}-{unknown-linux-gnu,apple-darwin}`. Windows targets: `{x86_64,aarch64}-pc-windows-msvc`. Auto-mirrored to [`Micrurus-Ai/corvid-installer`](https://github.com/Micrurus-Ai/corvid-installer) for ergonomic shortcuts; Corvid-lang/install/ is the single source of truth. |
| Install `corvid` (nightly, if you want HEAD) | Same scripts, `CORVID_VERSION=nightly` env var. Nightly builds from main; resolves the most recent `nightly-*` tag via GitHub Releases API (no `jq` required). |
| Pin against the exact build | `corvid --version` prints `corvid <semver> (<short-sha>, <commit-date>)`. The outreach message names the SHA you should match. |
| Environment sanity check | `corvid doctor` (verifies provider keys, local models, replay storage, approvals, wasm/native toolchains, registry lock, platform prerequisites) |
| Runnable demos for every shipped invention | `corvid tour --list` then `corvid tour --topic <name>` |
| Repository | <https://github.com/Micrurus-Ai/Corvid-lang> |
| Website + docs | <https://corvid-lang.org> / <https://corvid-lang.org/docs> |
| Reference apps (copy-and-carve template) | <https://github.com/Micrurus-Ai/Corvid-lang/tree/main/examples/backend> (5 apps, each with `CLAIM.md`, deploy manifests, traces, evals) |
| Per-app signed-cdylib claim manifests | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/examples/backend/personal_executive_agent/CLAIM.md> (PEA — same shape under `personal_knowledge_agent/`, `finance_operations_agent/`, `customer_support_agent/`, `code_maintenance_agent/`) |
| 22-row Proof Matrix for the moat inventions | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/inventions.md#proof-matrix> |
| Core semantics reference | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/core-semantics.md> |
| Grammar (EBNF) | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/reference/grammar.md> |
| Tutorial / cookbook | <https://corvid-lang.org/docs/book> + <https://corvid-lang.org/docs/recipes> |
| Backend tutorial | <https://corvid-lang.org/docs/book/14-backend-tutorial> |
| PEA tutorial | <https://corvid-lang.org/docs/book/15-personal-executive-agent-tutorial> |
| Roadmap | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md> |
| Launch claim audit (every public claim with runnable evidence) | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/meta/launch-claim-audit.md> |
| Companion trial doc (inspect path) | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-trial-one.md> |
| Feedback triage process | <https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md> |
| Where to file the report | New GitHub issue at <https://github.com/Micrurus-Ai/Corvid-lang/issues/new> with label `friends-and-family-trial` |

## Build path (suggested, not required)

> **Version pin.** The commands below match `corvid` at the SHA
> named in the outreach message. Verify with `corvid --version`
> — it prints `corvid <semver> (<short-sha>, <commit-date>)`. If
> your SHA doesn't match, reach out before debugging.

```sh
# 1. Install + sanity check.
curl -fsSL https://raw.githubusercontent.com/Micrurus-Ai/Corvid-lang/main/install/install.sh | sh
corvid --version   # corvid 0.0.x (<sha>, <date>) — match the outreach SHA
corvid doctor      # provider keys, local models, replay storage,
                   # approvals, wasm/native toolchains, registry
                   # lock, platform prerequisites

# 2. Skim the inventions matrix so you know what's in the box.
open https://corvid-lang.org/docs/reference/inventions

# 3. Clone the repo for the reference apps (you'll copy from
#    them but build your own standalone app).
git clone https://github.com/Micrurus-Ai/Corvid-lang.git
cd Corvid-lang

# 4. Run one reference app cold so you've seen the shape.
corvid serve examples/backend/personal_executive_agent/src/main.cor \
  --listen 127.0.0.1:8000 &
curl http://127.0.0.1:8000/schema
curl -X POST http://127.0.0.1:8000/actions/follow-up/send \
  -H 'Content-Type: application/json' \
  -d '{"to":"...", "body":"..."}'    # 202 + approval id
curl http://127.0.0.1:8000/__approvals  # lists pending
kill %1                               # stop the reference-app serve

# 5. Carve a smaller standalone app of your own.
#    `corvid new` produces a hello-world scaffold (no
#    --template backend flag yet). Copy from the reference app's
#    main.cor + migrations/ as your starting point.
cd ..
corvid new my_app
cd my_app
# Edit src/main.cor — define types, effects, tools, agents,
# and a `server` block with at least one POST route per the
# 6 build-surfaces ask above. Add migrations/0001_*.sql files
# for the persistence surface. Add evals/<name>.cor for at
# least one eval case.

# 6. Build (debug, interpreter-runnable).
corvid check src/main.cor                     # full pipeline: lex/parse/resolve/typecheck/imports
corvid serve src/main.cor --listen 127.0.0.1:8001 &
# Stress test against 127.0.0.1:8001 — POST requests, the
# /__approvals/* admin endpoints, etc.

# 7. Build + sign as a cdylib for the deploy path.
#    `--sign` takes the key PATH directly (no `--key` flag).
#    Either supply the path or `--sign` will fall back to
#    CORVID_SIGNING_KEY if set.
#
#    NOTE: cdylib targets REQUIRE at least one `pub extern "c"`
#    agent declared in main.cor — that's the exported ABI
#    surface the signed cdylib enumerates. A hello-world
#    `corvid new` scaffold won't have one; add (or copy from a
#    reference app) something like:
#        pub extern "c" agent handle_request(...) -> Result<...>:
#            ...
#    The error message names the requirement but not yet a doc
#    page (filed as `33Q-pub-extern-doc-page`).
echo "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f" > dev.key
corvid build src/main.cor --target=cdylib --sign dev.key
ls target/release/   # main.so / main.dylib / main.dll depending on OS

# 8. Verify the signed cdylib's claim manifest.
corvid claim --explain target/release/main.so --source src/main.cor

# 9. Render the Phase 43 deploy package.
#    <APP> arg must be a NAMED directory (`.` is rejected by
#    the impl's filename check). Use `$(pwd)` if you're in the
#    app dir. CORVID_DEPLOY_SIGNING_KEY is REQUIRED — same
#    32-byte hex seed as the dev signing key works.
export CORVID_DEPLOY_SIGNING_KEY="000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
corvid deploy package "$(pwd)" --out deploy/ --cdylib target/release/main.so
ls deploy/
# Dockerfile  oci-labels.json  env.schema.json  health.json
# migrate.sh  startup-checks.md  build-attestation.dsse.json
# sbom.spdx.json  VERIFY.md
#
# If you want to actually `docker build` the rendered image,
# run from the APP ROOT (not `cd deploy/`). The Dockerfile's
# COPY paths are relative to the app root; `deploy/` only
# contains generated artifacts:
#     docker build -f deploy/Dockerfile -t my_app:dev .
#
# The renderer emits COPY lines only for paths that exist at
# render time (closed under 33Q4). A bare `corvid new` scaffold
# produces a Dockerfile with just `COPY src` + `COPY corvid.toml`;
# add migrations/evals/traces/tools.py and re-run `corvid deploy
# package` to pick them up.

# 10. Stress-test other surfaces.
corvid audit src/main.cor       # operator-summary; takes a FILE, not a dir
corvid migrate up               # applies migrations to target/corvid.sqlite
corvid jobs enqueue --kind email-send --payload '{}'
corvid jobs run --workers 4 --max-runtime-ms 30000
# (`corvid claim audit --explain-failures` is a repo-internal
#  command that reads `docs/meta/launch-claim-audit.md`; runs
#  in the Corvid-lang clone, not in a standalone app dir. The
#  app-dir equivalent is `corvid claim --explain
#  target/release/main.so --source src/main.cor` shown in
#  step 8.)
```

**Things to know that the surface doesn't always advertise:**

- **`corvid run` on a multi-agent file** currently asks you to
  disambiguate with `--agent <name>`, but the parser may not yet
  accept that flag (filed as code follow-up
  `35V2-P33-corvid-run-agent-flag`). For the trial, prefer
  `corvid serve` (HTTP) or `corvid build --target=cdylib`
  (signed deploy) over `corvid run` for multi-agent apps.
- **`corvid new` produces a hello-world scaffold**, not a
  backend-shaped one. If you want a backend starting point,
  copy `examples/backend/personal_executive_agent/src/main.cor`
  + `migrations/` + `evals/` and carve it down to a small app
  of your own. The `std/` directory is vendored automatically
  into `src/std/` so `import "./std/effects"` works without
  setup (this was a bug as recently as 2026-06-04; fixed at
  [`7b92e90`](https://github.com/Micrurus-Ai/Corvid-lang/commit/7b92e90)).
- **`cargo install --path crates/corvid-cli`** from a source
  clone gives you a runnable CLI but no shipped `std/` and no
  prebuilt `libcorvid_runtime.a`, so the native-deploy path
  won't work. The install script is the canonical install path
  — it ships the binary + std/ + staticlib together.
- **Architecture coverage.** Stable release archives ship for
  `{x86_64,aarch64}-{unknown-linux-gnu,apple-darwin}` and
  `{x86_64,aarch64}-pc-windows-msvc`. If you're on Raspberry
  Pi, AWS Graviton, Surface Pro X, or a Snapdragon X laptop,
  the install script will fetch the right archive natively
  (no source-build fallback needed).
- **Windows SmartScreen warning.** The Windows binaries ship
  unsigned until Phase 33P7 Authenticode lands. If you see an
  "unrecognized publisher" SmartScreen prompt, that's expected
  for pre-v1.0 builds — please flag it in your report as a
  user-experience signal, but it's already on ROADMAP.
- **Pre-v1.0 versioning is intentional.** `corvid --version`
  reports `0.0.x` (with the SHA + date suffix). The bump to
  `0.1.0` / `1.0.0` happens at the actual v1.0 cut; until then
  every release is honest about being pre-launch. If the
  low version number reads oddly given the surface maturity,
  that's the right signal — please flag it in your report.
- **`corvid serve` and approval-gated tools.** Surface 3
  (approval-gated dangerous tool over HTTP) now works two ways
  on `corvid serve`:
  - **Drop a `tools.py` next to your source** (the
    `corvid new` scaffold writes one). Decorate your async
    handler with `@tool("<name>")` from `corvid_runtime` and
    `corvid serve` autoloads it via embedded Python. Fastest
    path for trying things out.
  - **OR `corvid serve --with-tools-cdylib <path>`** with a
    Rust cdylib built from a crate that uses the `#[tool]`
    proc-macro (`crate-type = ["cdylib"]`). Production-shape
    path; no Python in the request path. Explicit flag wins
    precedence over implicit autoload if both define the
    same tool.
  - Both shipped under 33Q1 ([`2d3e24f`](https://github.com/Micrurus-Ai/Corvid-lang/commit/2d3e24f),
    [`ff49112`](https://github.com/Micrurus-Ai/Corvid-lang/commit/ff49112)).
- **Approval-budget integrity.** A 500 from a handler under
  `/__approvals/<id>/approve` no longer consumes the approval
  (closed under 33Q2). The 500 body carries
  `approval_status: "pending"` + a `retry` envelope, and
  `GET /__approvals/<id>` surfaces a `last_handler_error`
  field so you can see why the grant didn't take effect. Retry
  /approve to try again, or POST /__approvals/<id>/deny to
  terminate a permanently-broken approval.
- **`@trust(...)` and `--sign` work together** (closed under
  33Q3). The signed cdylib's `claim --explain` enumerates
  `trust.constraint_enforcement` as one of the enforced
  guarantees when your agent declares `@trust(<level>)` or
  `@trust(autonomous_if_confident(<threshold>))`. The
  typechecker rejects bodies that violate the declared
  ceiling at compile time.

## Report template

Open a GitHub issue at
<https://github.com/Micrurus-Ai/Corvid-lang/issues/new> with the
label `friends-and-family-trial` and the title
`[Trial] <your-handle> — <one-line summary>`. The body follows
the
[`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md)
intake shape:

```markdown
## Identity
- Handle / name (whatever you want public; "anonymous-YYYY-MM-DD" is fine):
- Commit SHA tested (from `corvid --version`):
- OS + architecture + shell:
- Time spent (rough — 1h / half-day / full-day):

## What I built
- Use case (one sentence):
- Surfaces exercised (which of the 6 above):
- Approximate line count of `main.cor`:
- Real provider keys used? (yes / no — and which providers if yes)

## What worked
- First-30-minute experience:
- Moat moments where Corvid caught something:
- CLI / docs / error-messages that surprised me positively:

## What didn't
- First failure (which command, what message):
- Doc pages I wished existed:
- Error messages that confused me (please paste the exact
  message + what I thought it meant):
- Where the moat felt like ceremony that didn't pay off:

## Production-readiness ceiling
- First thing that would stop me from shipping this for real:
- A connector / auth / deploy / ops surface I needed that
  doesn't exist:
- An honest moment where Corvid over-claimed:

## Suggested disposition (your call, we'll triage)
- This is a CODE issue: [ ] specific to my use case / [ ] general
- This is a DOCS issue: [ ] missing page / [ ] wrong page /
  [ ] page exists but wasn't discoverable
- This is a TEST issue: [ ] regression caught me / [ ] no
  regression but the surface deserves a test
- This is NON-SCOPE: [ ] explicitly out-of-v1.0 / [ ] my use
  case is unusual / [ ] I disagree with the trade-off but
  understand why it was made
```

## What happens with your report

Per
[`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md),
every report disposes as one of:

- `code` — fix lands with a linked commit before v1.0 cut.
- `docs` — clarification lands with a linked commit.
- `test` — regression coverage lands with a linked test.
- `non-scope` — explicitly named in the launch claim audit or
  the v1.0 ROADMAP as out-of-scope for this release, with a
  pointer to the post-v1.0 phase that owns it.

The disposition rate is published in the v1.0 launch materials
per the closing criterion at
[`ROADMAP.md L51`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md#L51):
"their feedback closes as code / docs / tests / explicit
non-scope before the public cut."

You'll see your handle in the launch announcement's contributor
list unless you ask to be anonymized.

## Time commitment

We're asking for **roughly half a day to a day** of your time,
in two parts:

1. **Build** (2-6 hours, depending on app complexity and your
   familiarity with Rust-shaped syntax).
2. **Write the report** (30-90 minutes — please don't polish
   it; raw thoughts are more valuable than polished prose).

If you find yourself spending more than that and still want to
ship the app, please file the partial report at the "Build"
stage with the partial-coverage caveat — that's useful signal.

## Questions before you start?

Reply to the original outreach (DM / email / Signal / however
we reached you) and we'll answer before you commit. If
something blocks you mid-build, file a partial-report issue
with `[Blocked]` in the title and we'll prioritize unblocking
you over the rest of the round.

Thank you for doing this. This is the round that catches the
things nobody on the maintainer side can see anymore.
