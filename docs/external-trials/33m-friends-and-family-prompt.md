# Corvid v1.0 Friends-and-Family Round — Build Prompt

> **Slice:** `33M` (repositioned, Path A) — final 4 weeks of Phase 43.
> **Target reviewers:** 5-10 hand-picked AI engineers.
> **Output:** one report per reviewer, triaged through
> [`docs/external-trials/phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md),
> closing as `code` / `docs` / `test` / `non-scope`.

This is the copy-pasteable prompt the maintainer sends each
hand-picked AI engineer who agrees to participate in the
friends-and-family round. It is paired with
[`docs/external-trials/phase-42-trial-one.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-trial-one.md)
which covers the "inspect a shipped reference app" path; **this
prompt covers the harder path**, building a small production-shape
app from scratch.

---

## What we're asking you to do

We're inviting you (hand-picked, not a public beta) to build a
**small production-shape AI backend app in Corvid v1.0** and
report back on whether the language holds up under your own
hands.

"Small production-shape" means the app should have, at minimum,
all six of these surfaces — not because they're impressive on
their own, but because they're the surfaces production AI apps
ALWAYS need and Corvid claims to make first-class:

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
- You do NOT need to use a real LLM provider for the build —
  every adapter ships with a mock mode by default
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
| Install `corvid` | <https://github.com/Micrurus-Ai/Corvid-lang/tree/main/install> (Unix `.sh` + PowerShell `.ps1`) or `cargo install --path crates/corvid-cli` |
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

```sh
# 1. Install + sanity check.
curl -fsSL https://corvid-lang.org/install.sh | sh
corvid doctor

# 2. Skim the inventions matrix so you know what's in the box.
open https://corvid-lang.org/docs/reference/inventions

# 3. Run one reference app cold so you've seen the shape.
corvid serve examples/backend/personal_executive_agent/src/main.cor \
  --listen 127.0.0.1:8000 &
curl http://127.0.0.1:8000/schema
curl -X POST http://127.0.0.1:8000/actions/follow-up/send \
  -d '{"to":"...", "body":"..."}'  # answers 202 + approval id
curl http://127.0.0.1:8000/__approvals  # lists pending

# 4. Carve a smaller app of your own.
corvid new my_app --template backend
cd my_app
# edit src/main.cor, add tables in migrations/, add evals/, ...

# 5. Build, package, inspect.
corvid build --target=cdylib src/main.cor --sign --key dev.key
corvid claim --explain target/cdylib/main.so --key dev.pub --source src/main.cor
corvid deploy package . --out deploy/ --cdylib target/cdylib/main.so
ls deploy/
# Dockerfile  oci-labels.json  env.schema.json  health.json
# migrate.sh  startup-checks.md  build-attestation.dsse.json
# sbom.spdx.json  VERIFY.md

# 6. Stress-test.
corvid jobs run --kill-after 2s some_job  # crash-recovery proof
corvid audit my_app  # operator-summary report
corvid claim audit --explain-failures  # nothing aspirational?
```

## Report template

Open a GitHub issue at
<https://github.com/Micrurus-Ai/Corvid-lang/issues/new> with the
label `friends-and-family-trial` and the title `[Trial]
<your-handle> — <one-line summary>`. The body should follow
the
[`phase-42-feedback-triage.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/external-trials/phase-42-feedback-triage.md)
intake shape:

```markdown
## Identity
- Handle / name (whatever you want public; "anonymous-2026-06-04" is fine):
- Commit SHA tested:
- OS + shell:
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
- This is a CODE issue: [ ] specific to my use case / [ ]
  general
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

The disposition rate is published in the v1.0 launch
materials, per the closing criterion at
[`ROADMAP.md L51`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md#L51):
"their feedback closes as code / docs / tests / explicit
non-scope before the public cut."

You'll see your handle in the launch announcement's
contributor list unless you ask to be anonymized.

## Time commitment

We're asking for **roughly half a day to a day** of your time,
broken into two parts:

1. **Build** (2-6 hours, depending on app complexity and
   your familiarity with Rust-shaped syntax).
2. **Write the report** (30-90 minutes — please don't polish
   it, raw thoughts are more valuable than polished prose).

If you find yourself spending more than that and still want
to ship the app, please file the partial report at the
"Build" stage with the partial-coverage caveat — that's
useful signal.

## Questions before you start?

Reply to the original outreach (DM / email / Signal / however
we reached you) and we'll answer before you commit. If
something blocks you mid-build, file a partial-report issue
with `[Blocked]` in the title and we'll prioritize unblocking
you over the rest of the round.

Thank you for doing this. This is the round that catches the
things nobody on the maintainer side can see anymore.
